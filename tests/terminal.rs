#![cfg(unix)]

use anyhow::{Context, Result, ensure};
use nix::sys::signal::Signal;
use serde_json::{Value, json};
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};

mod support;
use support::{
    media::{
        PreviewFixture, assert_success, dominant, make_video, read_preview, seek_preview, shim,
    },
    pty::{BIN, KITTY_UPLOAD, Session, TIMEOUT, editor, kitty_delete},
};

#[test]
fn edits_resizes_previews_exports_and_resumes_real_media() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("clip café.v1.mp4");
    let sidecar = dir.path().join("clip café.v1.vroom");
    let output = dir.path().join("clip café.v1_vroom.mp4");
    make_video(&path)?;

    let mut session = Session::start(editor(&path, true))?;
    session.wait_for_screen("[0:00.0 / 0:08.0]")?;
    let (end, preview) = read_preview(&mut session, 0, (8, 4))?;
    preview.assert_size((74, 21), 336);
    preview.assert_color(0);

    session.resize(110, 25)?;
    let (end, preview) = read_preview(&mut session, end, (3, 28))?;
    preview.assert_size((56, 16), 256);
    preview.assert_color(0);

    // Save marks at 2s and 4s, a 2x speed on the last segment, and no cut.
    // Exercise both navigation directions, delete/re-add, and frame mode.
    let end = seek_preview(&mut session, end, "\x1b[C", (3, 28), 0)?;
    let end = seek_preview(&mut session, end, "\x1b[C", (3, 28), 1)?;
    let end = seek_preview(&mut session, end, "m", (3, 28), 1)?;
    let end = seek_preview(&mut session, end, "\x1b[C", (3, 28), 1)?;
    let end = seek_preview(&mut session, end, "\x1b[C", (3, 28), 2)?;
    let end = seek_preview(&mut session, end, "m", (3, 28), 2)?;
    session.send("=")?;
    let end = seek_preview(&mut session, end, "\x1b[Z", (3, 28), 1)?;
    session.send("-")?;
    let end = seek_preview(&mut session, end, "\t", (3, 28), 2)?;
    let end = seek_preview(&mut session, end, "M", (3, 28), 2)?;
    let end = seek_preview(&mut session, end, "m", (3, 28), 2)?;
    session.send("f")?;
    session.wait_for_screen("[60 / 80]")?;
    session.wait_for_screen("Seg 3/3  40–80  KEEP  speed: ×2.00")?;

    session.send("s")?;
    session.wait_for_screen("Saved → clip café.v1.vroom")?;
    let bytes = fs::read(&sidecar)?;
    let edit: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        edit,
        json!({ "version": 1, "video": path, "segments": [
        {"start": 0.0, "speed": 1.0}, {"start": 2.0, "speed": 0.5}, {"start": 4.0, "speed": 2.0}
    ] })
    );

    // Cut [2,4) after saving; the export should omit the green frames. Seek
    // back to 1s before exercising playback.
    let end = seek_preview(&mut session, end, "\x1b[Z", (3, 28), 1)?;
    session.send("c")?;
    seek_preview(&mut session, end, "\x1b[D", (3, 28), 0)?;
    session.wait_for_screen("[10 / 40]")?;
    session.wait_for_screen("Seg 1/3  0–20  KEEP")?;
    session.drain_output()?;
    let end = session.output.len();
    session.send(" ")?;
    let (first, preview) = read_preview(&mut session, end, (3, 28))?;
    preview.assert_color(0);
    let (second, next) = read_preview(&mut session, first, (3, 28))?;
    let (third, last) = read_preview(&mut session, second, (3, 28))?;
    assert_frame_swap(&session, first, second, preview.id, next.id)?;
    assert_frame_swap(&session, second, third, next.id, last.id)?;
    session.send(" e")?; // pause and export the unsaved cut
    session.wait_for_screen("Exported →")?;
    session.send("\x03")?;
    session.finish()?;
    assert_eq!(fs::read(&sidecar)?, bytes, "export must not save edits");

    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type",
            "-of",
            "json",
        ])
        .arg(&output)
        .output()?;
    assert_success(probe.status, &probe.stderr);
    let metadata: Value = serde_json::from_slice(&probe.stdout)?;
    assert_eq!(metadata["streams"], json!([{ "codec_type": "video" }]));
    let duration = metadata["format"]["duration"]
        .as_str()
        .unwrap()
        .parse::<f64>()?;
    // FFmpeg may round output to the frame cadence or duplicate the final frame.
    assert!(
        (3.8..=4.4).contains(&duration),
        "export duration: {duration}"
    );
    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&output)
        .args([
            "-vf",
            "scale=2:2",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .output()?;
    assert_success(decoded.status, &decoded.stderr);
    let frames: Vec<_> = decoded.stdout.chunks_exact(12).collect();
    let red = |frame: &&[u8]| dominant(frame, 0);
    let blue = |frame: &&[u8]| dominant(frame, 2);
    let first_blue = frames.iter().position(blue).unwrap();
    assert!(
        (17..=23).contains(&first_blue),
        "unexpected red-to-blue transition at frame {first_blue}"
    );
    assert!(frames[..first_blue].iter().all(red));
    assert!(frames[first_blue..].iter().all(blue));
    let log = fs::read_to_string(output.with_extension("log"))?;
    assert!(log.contains("[v0][v1]concat=n=2:v=1:a=0[vout]"));
    assert!(log.contains("trim=start=4.000000:end=8.000000,setpts=(PTS-STARTPTS)*0.500000"));

    let mut command = editor(&path, false);
    command.arg("extra args are ignored");
    let mut session = Session::start(command)?;
    session.wait_for_wire(0, "Resumed from")?;
    session.wait_for_screen("[0:00.0 / 0:08.0]")?;
    session.wait_for_screen("[ Kitty / WezTerm / Ghostty required for video preview ]")?;
    let before_paste = session.output.len();
    session.send("\x1b[200~qec\x1b[201~")?;
    session.send("f")?;
    session.wait_for_screen("[0 / 80]")?; // processed a real key after paste without quitting or cutting
    session.send("cq")?; // an ordinary edit followed by quit must not save
    session.finish()?;
    let after_paste = String::from_utf8_lossy(&session.output[before_paste..]);
    assert!(
        !after_paste.contains("Exporting"),
        "bracketed paste triggered an export"
    );
    assert_eq!(fs::read(&sidecar)?, bytes);
    Ok(())
}

#[test]
fn rapid_seek_and_resize_cancel_pending_frames_and_show_the_latest_preview() -> Result<()> {
    let fixture = PreviewFixture::new()?;
    let (initial, old_seek, resized) = ("0.000-336", "1.000-336", "2.000-256");
    fixture.frame(initial, (598, 336), [255, 0, 0])?;
    fixture.frame(old_seek, (598, 336), [0, 255, 0])?;
    fixture.frame(resized, (456, 256), [0, 0, 255])?;

    let mut session = fixture.start()?;
    fixture.wait_for_requests(&mut session, &[initial])?;
    session.send("\x1b[C")?;
    fixture.wait_for_requests(&mut session, &[initial, old_seek])?;
    session.resize(110, 25)?;
    // The resize and seek can be delivered in either order. Gate the final
    // request until both the requested position and height have landed.
    session.send("\x1b[C")?;
    session.wait_for_file(&fixture.path(&format!("pid-{resized}")))?;
    fixture.wait_for_cancel(&mut session, initial)?;
    fixture.wait_for_cancel(&mut session, old_seek)?;
    fixture.allow(resized)?;

    let (_, preview) = read_preview(&mut session, 0, (3, 28))?;
    preview.assert_size((56, 16), 256);
    preview.assert_color(2);
    session.wait_for_screen("[0:02.0 / 0:08.0]")?;
    session.send("q")?;
    session.finish()?;
    let wire = String::from_utf8_lossy(&session.output);
    assert_eq!(
        wire.matches(KITTY_UPLOAD).count(),
        1,
        "a stale frame was sent to the terminal"
    );
    Ok(())
}

#[test]
fn playback_recovers_from_failed_preview_and_skips_consecutive_cuts() -> Result<()> {
    let fixture = PreviewFixture::new()?;
    let (initial, start, after_cut) = ("0.000-336", "1.000-336", "4.000-336");
    fs::write(
        fixture.path("demo.vroom"),
        r#"{"version":1,"segments":[{"start":0},{"start":1.1,"cut":true},{"start":2,"cut":true},{"start":4}]}"#,
    )?;
    fixture.frame(initial, (598, 336), [255, 0, 0])?;
    fixture.frame(start, (598, 336), [255, 0, 0])?;
    fixture.frame(after_cut, (598, 336), [0, 0, 255])?;
    fixture.allow(initial)?;

    let mut session = fixture.start()?;
    let (end, _) = read_preview(&mut session, 0, (8, 4))?;
    fixture.allow(start)?;
    session.send("\x1b[C")?;
    let (end, preview) = read_preview(&mut session, end, (8, 4))?;
    preview.assert_color(0);

    // A failed FFmpeg process can have written bytes to stdout. Ignore its
    // PNG, clear the previous image, and stop playback so Space can restart it.
    fs::remove_file(fixture.path(&format!("allow-{start}")))?;
    fs::write(fixture.path(&format!("fail-{start}")), [])?;
    session.send(" ")?;
    fixture.wait_for_requests(&mut session, &[initial, start, start])?;
    fixture.allow(start)?;
    let cleared = session.wait_for_wire(end, &kitty_delete(preview.id))?;

    fs::remove_file(fixture.path(&format!("allow-{start}")))?;
    fs::remove_file(fixture.path(&format!("fail-{start}")))?;
    session.send(" ")?;
    fixture.wait_for_requests(&mut session, &[initial, start, start, start])?;
    // Give the wall-clock tick enough time to cross 1.1s. Its two-second
    // cap means it must land in one of the cuts before jumping to 4.000.
    std::thread::sleep(Duration::from_millis(150));
    fixture.allow(start)?;
    let (red, preview) = read_preview(&mut session, cleared, (8, 4))?;
    preview.assert_color(0);
    fixture.wait_for_requests(&mut session, &[initial, start, start, start, after_cut])?;
    fixture.allow(after_cut)?;
    let (end, preview) = read_preview(&mut session, red, (8, 4))?;
    preview.assert_color(2);
    session.send("q")?;
    session.finish()?;
    let wire = String::from_utf8_lossy(&session.output[..end]);
    assert_eq!(
        wire.matches(KITTY_UPLOAD).count(),
        4,
        "failed extraction must clear, not paint a frame"
    );
    Ok(())
}

#[test]
fn replacing_a_displayed_frame_never_deletes_it_before_its_replacement() -> Result<()> {
    let fixture = PreviewFixture::new()?;
    let frames = ["0.000-336", "1.000-336", "2.000-336"];
    for (request, color) in frames
        .into_iter()
        .zip([[255, 0, 0], [0, 255, 0], [0, 0, 255]])
    {
        fixture.frame(request, (598, 336), color)?;
        fixture.allow(request)?;
    }
    let mut session = fixture.start()?;
    let (mut end, initial) = read_preview(&mut session, 0, (8, 4))?;
    initial.assert_color(0);
    assert!(!String::from_utf8_lossy(&session.output[..end]).contains("a=d"));
    let mut previous = initial.id;
    for channel in [1, 2] {
        session.send("\x1b[C")?;
        let (next, preview) = read_preview(&mut session, end, (8, 4))?;
        preview.assert_color(channel);
        assert_frame_swap(&session, end, next, previous, preview.id)?;
        previous = preview.id;
        end = next;
    }
    assert_eq!(previous, initial.id, "use only two image IDs");
    session.send("q")?;
    session.finish()?;
    let cleanup = std::str::from_utf8(&session.output[end..])?;
    let (cleanup, _) = cleanup
        .split_once("\x1b[?1049l")
        .context("expected to leave alternate screen")?;
    for id in [42, 43] {
        assert!(
            cleanup.contains(&kitty_delete(id)),
            "image ID {id} not cleaned up"
        );
    }
    Ok(())
}

fn assert_frame_swap(
    session: &Session,
    from: usize,
    to: usize,
    previous: u32,
    next: u32,
) -> Result<()> {
    assert_ne!(next, previous, "do not reuse the visible image's ID");
    let delete = kitty_delete(previous);
    let wire = std::str::from_utf8(&session.output[from..to])?;
    // The reader decoded the complete incoming PNG. It must precede the
    // delete, and there must be no screen clear during the swap.
    let (upload, tail) = wire
        .split_once(&delete)
        .context("previous image was not retired")?;
    assert!(upload.contains(KITTY_UPLOAD));
    assert!(upload.ends_with("\x1b\\"));
    assert_eq!(tail, "\x1b8");
    assert_eq!(wire.matches("a=d").count(), 1);
    assert!(!wire.contains("\x1b[2J"));
    Ok(())
}

#[test]
fn export_errors_are_reported_and_ffmpeg_failures_still_log() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("demo.mp4");
    fs::write(&path, [])?;
    fs::create_dir(path.with_extension("vroom"))?; // unreadable as a file and cannot overwrite
    shim(
        dir.path(),
        "ffprobe",
        r#"printf '%s' '{"format":{"duration":"8.0"}}'"#,
    )?;
    shim(
        dir.path(),
        "ffmpeg",
        r#"
        [ "$2" = quiet ] && exit 0
        for arg; do output="$arg"; done
        case "$VROOM_TEST_EXPORT" in
            fail) echo ffmpeg-stdout; echo ffmpeg-stderr >&2; exit 2;;
            zero) : > "$output";;
        esac
    "#,
    )?;
    for (mode, keys, message) in [
        ("all-cut", "ce", "nothing to export: all segments are cut"),
        ("fail", "e", "ffmpeg failed (see "),
        ("missing", "e", "export produced an empty file (see "),
        ("zero", "e", "export produced an empty file (see "),
    ] {
        let log = dir.path().join("demo_vroom.log");
        let mut command = editor(&path, false);
        command
            .env("PATH", dir.path())
            .env("VROOM_TEST_EXPORT", mode);
        let mut session = Session::start(command)?;
        session.wait_for_wire(0, "warning: could not load")?;
        session.wait_for_screen("[ Kitty / WezTerm / Ghostty required for video preview ]")?;
        session.send("s")?;
        session.wait_for_screen("Save failed:")?;
        session.send(keys)?;
        session.wait_for_screen(message)?;
        session.signal(Signal::SIGTERM)?;
        session.finish()?;
        if mode == "all-cut" {
            assert!(!log.exists());
        } else {
            let text = fs::read_to_string(&log)?;
            assert!(text.contains("command:\nffmpeg -v verbose"));
            if mode == "fail" {
                assert!(text.contains("ffmpeg-stdout") && text.contains("ffmpeg-stderr"));
            }
            fs::remove_file(log)?;
        }
    }
    Ok(())
}

#[test]
fn export_finishes_after_quit_and_split_input_cannot_block_cleanup() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let video = dir.path().join("demo.mp4");
    fs::write(&video, [])?;
    shim(
        dir.path(),
        "ffprobe",
        r#"printf '%s' '{"format":{"duration":"8.0"}}'"#,
    )?;
    shim(
        dir.path(),
        "ffmpeg",
        r#"
        [ "$2" = quiet ] && exit 0
        for arg; do output="$arg"; done
        attempts=0
        : > export-started
        while [ ! -f finish ]; do
            attempts=$((attempts + 1))
            [ "$attempts" -lt 200 ] || exit 1
            /bin/sleep 0.05
        done
        printf complete > "$output"
        echo 'export finished' >&2
    "#,
    )?;
    let log_path = dir.path().join("demo_vroom.log");
    let output_path = dir.path().join("demo_vroom.mp4");
    let finish_path = dir.path().join("finish");
    fs::write(&output_path, "previous export")?;
    let mut command = editor(&video, false);
    command.env("PATH", dir.path()).current_dir(dir.path());
    let mut session = Session::spawn(command, true)?;
    session.wait_for_screen("[0:00.0 / 0:08.0]")?;
    session.send("e")?;
    session.wait_for_screen("Exporting…")?;
    session.wait_for_file(&dir.path().join("export-started"))?;

    session.send("\x1b[")?; // deliberately incomplete terminal input
    session.read_output(30)?;
    session.signal(Signal::SIGTERM)?;
    // Release the child even if the assertion below fails.
    let stopped = session.finish();
    fs::write(&finish_path, [])?;
    stopped?;
    let deadline = Instant::now() + TIMEOUT;
    while !fs::read_to_string(&log_path)?.contains("export finished") {
        ensure!(
            Instant::now() < deadline,
            "export did not finish after editor exit"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fs::read(&output_path)?, b"complete");
    Ok(())
}

#[test]
fn cli_keeps_original_usage_and_probe_errors() -> Result<()> {
    let usage = Command::new(BIN).output()?;
    assert_eq!(usage.status.code(), Some(1));
    assert_eq!(usage.stderr, b"vedit: usage: vedit <video-file>\n");

    let dir = tempfile::tempdir()?;
    let missing = Command::new(BIN)
        .current_dir(dir.path())
        .arg("--help")
        .output()?;
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8(missing.stderr)?.starts_with("vedit: open \"--help\":"));

    let path = dir.path().join("demo.mp4");
    fs::write(&path, [])?;
    let mut command = Command::new(BIN);
    command.arg(&path).env("PATH", dir.path());
    for (stub, message) in [
        (None, "probe: ffprobe:"),
        (Some("exit 17"), "ffprobe: exit status 17"),
        (Some("echo not-json"), "parse ffprobe output:"),
    ] {
        if let Some(stub) = stub {
            shim(dir.path(), "ffprobe", stub)?;
        }
        let output = command.output()?;
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr)?;
        assert!(stderr.starts_with("Probing "));
        assert!(stderr.contains(message), "{stderr}");
    }
    Ok(())
}
