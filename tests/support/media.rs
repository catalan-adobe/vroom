use super::pty::{KITTY_UPLOAD, Session, TIMEOUT, editor};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use std::{
    fs,
    io::Cursor,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::Instant,
};

pub fn assert_success(status: ExitStatus, stderr: &[u8]) {
    assert!(
        status.success(),
        "{status}: {}",
        String::from_utf8_lossy(stderr)
    );
}

pub fn make_video(path: &Path) -> Result<()> {
    let video = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=160x90:r=10:d=2",
            "-f",
            "lavfi",
            "-i",
            "color=c=lime:s=160x90:r=10:d=2",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=160x90:r=10:d=4",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=8:sample_rate=44100",
            "-filter_complex",
            "[0:v][1:v][2:v]concat=n=3:v=1:a=0[v]",
            "-map",
            "[v]",
            "-map",
            "3:a:0",
            "-c:v",
            "mpeg4",
            "-q:v",
            "3",
            "-c:a",
            "aac",
            "-y",
        ])
        .arg(path)
        .output()
        .context("install ffmpeg/ffprobe to run the media integration test")?;
    assert_success(video.status, &video.stderr);
    Ok(())
}

#[derive(Debug)]
pub struct Preview {
    pub id: u32,
    cells: (u16, u16),
    pixel_width: u32,
    pixel_height: u32,
    center: [u8; 3],
}

impl Preview {
    pub fn assert_size(&self, cells: (u16, u16), height: u32) {
        assert_eq!(self.cells, cells, "{self:?}");
        assert_eq!(self.pixel_height, height, "{self:?}");
        assert!(self.pixel_width.is_multiple_of(2), "{self:?}");
        assert!(
            (f64::from(self.pixel_width) / f64::from(height) - 16.0 / 9.0).abs() < 0.01,
            "{self:?}"
        );
    }

    pub fn assert_color(&self, channel: usize) {
        assert!(
            dominant(&self.center, channel),
            "expected RGB channel {channel} to dominate; got {self:?}"
        );
    }
}

pub fn dominant(pixel: &[u8], channel: usize) -> bool {
    pixel[channel] > 200 && (0..3).all(|i| i == channel || pixel[i] < 40)
}

pub fn read_preview(
    session: &mut Session,
    from: usize,
    placement: (u16, u16),
) -> Result<(usize, Preview)> {
    // Consume the next transfer in order; don't search for the expected
    // placement, which could skip an earlier frame sent to an old viewport.
    let cursor = session.wait_for_wire(from, "\x1b7\x1b[")?;
    let end = session.wait_for_wire(cursor, "\x1b8")?;
    let wire = std::str::from_utf8(&session.output[cursor..end])?;
    let (coords, wire) = wire
        .split_once('H')
        .context("missing Kitty frame placement")?;
    assert_eq!(coords, format!("{};{}", placement.0, placement.1));
    ensure!(
        wire.starts_with(KITTY_UPLOAD),
        "expected Kitty PNG upload after cursor placement"
    );
    let header = wire
        .split_once(';')
        .context("missing Kitty upload parameters")?
        .0;
    let param = |name| {
        header
            .split(',')
            .find_map(|field| field.strip_prefix(name))
            .with_context(|| format!("missing Kitty upload parameter {name}"))
    };
    let id = param("i=")?.parse()?;
    let cells = (param("c=")?.parse()?, param("r=")?.parse()?);
    let mut payload = String::new();
    let mut chunks = wire.split("\x1b\\");
    loop {
        let chunk = chunks.next().context("missing final Kitty chunk")?;
        let (header, base64) = chunk
            .strip_prefix("\x1b_G")
            .and_then(|chunk| chunk.split_once(';'))
            .context("missing Kitty chunk header")?;
        let complete = match header.split(',').find_map(|field| field.strip_prefix("m=")) {
            Some("0") => true,
            Some("1") => false,
            _ => anyhow::bail!("missing Kitty continuation flag in {header}"),
        };
        assert!(base64.len() <= 4096);
        payload.push_str(base64);
        if complete {
            break;
        }
    }
    // After the final upload chunk, vroom can retire the previous image
    // before DECRC. Keep those delete commands out of the PNG payload.
    for trailing in chunks.take_while(|chunk| *chunk != "\x1b8") {
        ensure!(
            trailing.starts_with("\x1b_Gq=2,a=d,d=I,i="),
            "unexpected escape after Kitty upload: {trailing:?}"
        );
    }
    let bytes = STANDARD.decode(payload)?;
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut pixels = vec![
        0;
        reader
            .output_buffer_size()
            .context("preview PNG is too large")?
    ];
    let frame = reader.next_frame(&mut pixels)?;
    ensure!(
        matches!(frame.color_type, png::ColorType::Rgb | png::ColorType::Rgba),
        "expected an RGB preview"
    );
    let center = (frame.height as usize / 2) * frame.line_size
        + (frame.width as usize / 2) * frame.color_type.samples();
    let preview = Preview {
        id,
        cells,
        pixel_width: frame.width,
        pixel_height: frame.height,
        center: pixels[center..center + 3].try_into()?,
    };
    Ok((end, preview))
}

pub fn seek_preview(
    session: &mut Session,
    from: usize,
    keys: &str,
    placement: (u16, u16),
    channel: usize,
) -> Result<usize> {
    session.send(keys)?;
    let (end, preview) = read_preview(session, from, placement)?;
    preview.assert_color(channel);
    Ok(end)
}

pub fn shim(dir: &Path, name: &str, script: &str) -> Result<()> {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{script}\n"))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

// Each request records its seek and requested height; a per-request gate
// lets the test decide exactly when that FFmpeg process can complete.
pub struct PreviewFixture(tempfile::TempDir);

impl PreviewFixture {
    pub fn new() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("demo.mp4"), [])?;
        shim(
            dir.path(),
            "ffprobe",
            r#"printf '%s' '{"format":{"duration":"8.0"},"streams":[{"codec_type":"video","width":160,"height":90,"r_frame_rate":"10/1"}]}'"#,
        )?;
        shim(
            dir.path(),
            "ffmpeg",
            r#"
            while [ "$#" -gt 0 ]; do
                case "$1" in
                    -ss) seek="$2"; shift;;
                    -vf) height="${2#scale=-2:}"; shift;;
                esac
                shift
            done
            request="$seek-$height"
            printf '%s\n' "$$" > "pid-$request"
            printf '%s\n' "$request" >> requests
            attempts=0
            while [ ! -f "allow-$request" ]; do
                attempts=$((attempts + 1))
                [ "$attempts" -lt 400 ] || exit 1
                /bin/sleep 0.05
            done
            if [ -f "frame-$request.png" ]; then
                /bin/cat "frame-$request.png"
            fi
            [ ! -f "fail-$request" ]
        "#,
        )?;
        Ok(Self(dir))
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }

    pub fn start(&self) -> Result<Session> {
        let mut command = editor(&self.path("demo.mp4"), true);
        command
            .env("PATH", self.0.path())
            .current_dir(self.0.path());
        Session::start(command)
    }

    pub fn frame(&self, request: &str, (width, height): (u32, u32), color: [u8; 3]) -> Result<()> {
        let mut bytes = vec![];
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()?
            .write_image_data(&color.repeat((width * height) as usize))?;
        fs::write(self.path(&format!("frame-{request}.png")), bytes)?;
        Ok(())
    }

    pub fn allow(&self, request: &str) -> Result<()> {
        fs::write(self.path(&format!("allow-{request}")), [])?;
        Ok(())
    }

    pub fn wait_for_requests(&self, session: &mut Session, expected: &[&str]) -> Result<()> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let requests = fs::read_to_string(self.path("requests")).unwrap_or_default();
            let complete_lines: Vec<_> = requests.split_terminator('\n').collect();
            if requests.ends_with('\n') && complete_lines.len() >= expected.len() {
                assert_eq!(
                    &complete_lines[..expected.len()],
                    expected,
                    "unexpected preview requests"
                );
                return Ok(());
            }
            session.tick(
                deadline,
                &format!("FFmpeg requests {expected:?}; received {requests:?}"),
            )?;
        }
    }

    pub fn wait_for_cancel(&self, session: &mut Session, request: &str) -> Result<()> {
        let pid = Pid::from_raw(
            fs::read_to_string(self.path(&format!("pid-{request}")))?
                .trim()
                .parse()?,
        );
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match kill(pid, None) {
                Err(Errno::ESRCH) => return Ok(()),
                Err(error) => return Err(error.into()),
                Ok(()) => session.tick(
                    deadline,
                    &format!("superseded preview {request} process {pid} to exit"),
                )?,
            }
        }
    }
}
