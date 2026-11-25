use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::{Command, ExitStatus, Stdio},
};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{model::Segment, paths::log_path};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub duration: f64,
    pub fps: f64,
    pub width: i32,
    pub height: i32,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ProbeOutput {
    streams: Vec<ProbeStream>,
    format: ProbeFormat,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ProbeStream {
    codec_type: String,
    width: i32,
    height: i32,
    r_frame_rate: String,
    duration: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ProbeFormat {
    duration: String,
}

pub fn probe(path: &Path) -> Result<Info> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
        ])
        .arg(path)
        .output()
        .context("ffprobe")?;
    check_status(output.status).context("ffprobe")?;
    parse_probe(&output.stdout).context("parse ffprobe output")
}

fn parse_probe(bytes: &[u8]) -> Result<Info> {
    let output: ProbeOutput = serde_json::from_slice(bytes)?;
    let duration = positive_number(&output.format.duration);
    let mut info = Info {
        duration,
        fps: 30.0,
        width: 0,
        height: 0,
    };
    if let Some(stream) = output
        .streams
        .iter()
        .find(|stream| stream.codec_type == "video")
    {
        info.width = stream.width;
        info.height = stream.height;
        if info.duration == 0.0 {
            info.duration = positive_number(&stream.duration);
        }
        if let Some((numerator, denominator)) = stream.r_frame_rate.split_once('/') {
            let fps = number(numerator) / number(denominator);
            if fps.is_finite() && fps > 0.0 {
                info.fps = fps;
            }
        } else {
            let fps = positive_number(&stream.r_frame_rate);
            if fps > 0.0 {
                info.fps = fps;
            }
        }
    }
    Ok(info)
}

fn number(value: &str) -> f64 {
    value
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .unwrap_or_default()
}

fn positive_number(value: &str) -> f64 {
    number(value).max(0.0)
}

pub async fn extract_frame(path: &Path, seconds: f64, height: usize) -> Result<Vec<u8>> {
    let output = tokio::process::Command::new("ffmpeg")
        .args(["-v", "quiet", "-ss", &format!("{seconds:.3}"), "-i"])
        .arg(path)
        .args([
            "-vframes",
            "1",
            "-vf",
            &format!("scale=-2:{height}"),
            "-f",
            "image2",
            "-vcodec",
            "png",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await?;
    check_status(output.status)?;
    Ok(output.stdout)
}

pub async fn export(source: &Path, segments: &[Segment], output: &Path) -> Result<()> {
    let filter = export_filter(segments)?;
    let args = [
        OsString::from("-v"),
        "verbose".into(),
        "-i".into(),
        source.into(),
        "-filter_complex".into(),
        filter.into(),
        "-map".into(),
        "[vout]".into(),
        "-y".into(),
        output.into(),
    ];
    // Give FFmpeg the log file itself so an in-flight export can finish (and
    // keep logging) after the editor quits, without a stdout/stderr pipe to us.
    let log = log_path(output);
    let timestamp = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default();
    let command_line = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o644);
    }
    let mut log_file = options.open(&log).ok();
    if let Some(file) = &mut log_file {
        let _ = write!(
            file,
            "vroom export — {timestamp}\n\ncommand:\nffmpeg {command_line}\n\noutput:\n"
        );
    }
    let log_stream = || {
        log_file
            .as_ref()
            .and_then(|file| file.try_clone().ok())
            .map_or_else(Stdio::null, Stdio::from)
    };
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(log_stream())
        .stderr(log_stream());
    // Remove the export from the foreground process group: the controlling
    // terminal can send that group SIGHUP when the editor/session exits.
    #[cfg(unix)]
    command.process_group(0);
    let status = command.status().await;
    if let (Err(error), Some(file)) = (&status, &mut log_file) {
        let _ = writeln!(file, "Could not run ffmpeg: {error}");
    }

    status
        .map_err(anyhow::Error::from)
        .and_then(check_status)
        .with_context(|| format!("ffmpeg failed (see {})", log.display()))?;
    ensure!(
        fs::metadata(output).is_ok_and(|file| file.len() != 0),
        "export produced an empty file (see {})",
        log.display()
    );
    Ok(())
}

fn export_filter(segments: &[Segment]) -> Result<String> {
    let mut filters = Vec::new();
    let mut labels = String::new();
    for (index, segment) in segments.iter().filter(|segment| !segment.cut).enumerate() {
        use std::fmt::Write;
        let label = format!("[v{index}]");
        let _ = write!(labels, "{label}");
        filters.push(format!(
            "[0:v]trim=start={:.6}:end={:.6},setpts=(PTS-STARTPTS)*{:.6}{label}",
            segment.start,
            segment.end,
            1.0 / segment.speed,
        ));
    }
    ensure!(
        !filters.is_empty(),
        "nothing to export: all segments are cut"
    );
    filters.push(format!("{labels}concat=n={}:v=1:a=0[vout]", filters.len()));
    Ok(filters.join(";"))
}

fn check_status(status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else if let Some(code) = status.code() {
        bail!("exit status {code}")
    } else {
        bail!("{status}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_uses_format_duration_and_first_video_stream() -> Result<()> {
        let json = br#"{
            "streams": [
                {"codec_type":"audio","duration":"16.0"},
                {"codec_type":"video","width":1920,"height":1080,"r_frame_rate":"24000/1001","duration":"12.0"},
                {"codec_type":"video","width":400,"height":600,"r_frame_rate":"25/1"}
            ],
            "format": {"duration": "18.5"}
        }"#;
        let info = parse_probe(json)?;
        assert_eq!(
            info,
            Info {
                duration: 18.5,
                fps: 24000.0 / 1001.0,
                width: 1920,
                height: 1080
            }
        );
        Ok(())
    }

    #[test]
    fn probe_falls_back_to_stream_duration_and_default_fps() -> Result<()> {
        let json = br#"{"streams":[{"codec_type":"video","duration":"2.5","r_frame_rate":"0/0"}],
            "format":{"duration":"N/A"}}"#;
        let info = parse_probe(json)?;
        assert_eq!(info.duration, 2.5);
        assert_eq!(info.fps, 30.0);
        assert!(parse_probe(b"invalid json").is_err());
        Ok(())
    }

    #[test]
    fn export_filters_trim_and_retime_only_kept_video_in_order() -> Result<()> {
        let segments = [
            Segment {
                start: 0.0,
                end: 2.0,
                cut: false,
                speed: 1.0,
            },
            Segment {
                start: 2.0,
                end: 4.0,
                cut: true,
                speed: 1.0,
            },
            Segment {
                start: 4.0,
                end: 6.125,
                cut: false,
                speed: 0.5,
            },
        ];
        assert_eq!(
            export_filter(&segments)?,
            concat!(
                "[0:v]trim=start=0.000000:end=2.000000,setpts=(PTS-STARTPTS)*1.000000[v0];",
                "[0:v]trim=start=4.000000:end=6.125000,setpts=(PTS-STARTPTS)*2.000000[v1];",
                "[v0][v1]concat=n=2:v=1:a=0[vout]",
            )
        );
        assert_eq!(
            export_filter(&segments[1..2]).unwrap_err().to_string(),
            "nothing to export: all segments are cut"
        );
        Ok(())
    }
}
