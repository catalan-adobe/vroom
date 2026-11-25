# vroom — Agent Guide

Terminal video editor in Rust using Ratatui, Crossterm and FFmpeg. Cut, trim,
and retime video without leaving your shell.

## Architecture

```text
src/main.rs            probe source, restore adjacent .vroom sidecar, launch TUI
src/model.rs           sorted marks, segment edits keyed by source start, output time
src/model/save.rs      backwards-compatible .vroom JSON persistence
src/paths.rs           sidecar, export and log filenames
src/video.rs           ffprobe metadata, asynchronous FFmpeg extraction and export
src/tui.rs             terminal lifecycle, input and asynchronous media jobs
src/tui/app.rs         editor actions, wall-clock playback, transient status
src/tui/view.rs        layout, timeline, segment colors and time/frame formatting
src/tui/kitty.rs       Kitty PNG placement, chunking and cleanup
tests/terminal.rs      pseudoterminal and real-media integration tests
```

## Editing and persistence

- Marks are sorted source timestamps in seconds (`f64`). Derive segments from
  the marks; keep cut/speed settings keyed by the exact segment start, not the
  index. Removing a mark leaves its settings available if the same timestamp is
  reinserted in the session. Only current segments are saved.
- The cursor and segment boundaries are in **source** time. The header displays
  **edited output** time: cuts contribute zero, and kept segments divide their
  duration by speed. Stop accumulating at the segment containing the cursor.
- The seek ladder starts at `1f · [1s]`. Its coarser tiers are `5s · 30s` for
  videos under 10 minutes, `10s · 1m · 5m` below one hour, and
  `30s · 5m · 20m` otherwise. Frame numbers are derived from the source FPS and
  are never persisted. Speed controls multiply by 2 or ½, with a floor of 1/64×.
- The `.vroom` format stays at version 1. Positive segment starts restore marks;
  absent `cut` and missing or zero `speed` mean KEEP and 1×. Use full round-trip
  float precision. The `video` field is informational: the argument supplied at
  launch and probed metadata remain authoritative. Save is explicit (`s`).

```json
{
  "version": 1,
  "video": "/path/to/video.mp4",
  "segments": [
    {"start": 0, "speed": 1},
    {"start": 5.2, "cut": true, "speed": 1},
    {"start": 10.5, "speed": 2}
  ]
}
```

## Preview and export

- Resize and seeking supersede an outstanding preview, so stale results cannot
  repaint a previous timestamp or use old dimensions. Playback advances by
  elapsed wall-clock time (capped at two seconds per frame) and segment speed,
  skips cuts, and stops at EOF. Do not advance by a fixed `1/FPS` per extraction.
- FFmpeg previews are PNGs scaled with `scale=-2:H` (height only, even width,
  original aspect ratio). The centered preview uses an estimated cell aspect
  ratio of 0.5 and a maximum of 40 rows.
- Kitty writes happen **after** the Ratatui text render. Keep the preview's
  background at the default, place the image at `z=-1`, and wrap the placement
  in **DECSC/DECRC (`\x1b7` / `\x1b8`)**. Alternate two image IDs so the old
  frame remains visible while the next image's chunks arrive. Delete the old
  ID only after the new placement; reusing the active ID or clearing the frame
  first can flash. Ratatui handles text diffs and viewport resize. The text
  fallback is used on other terminals.
- Export snapshots the current edits, not the save file. Use FFmpeg
  `trim` + `setpts` + `concat`, map only the video stream, and keep the original
  output extension. Send FFmpeg diagnostics directly to the sibling log so an
  in-flight export can finish after the editor closes.
  If all segments are cut, do not invoke FFmpeg or write a log.

## Build and checks

```sh
cargo build --locked
cargo run --locked -- video.mp4
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Dependencies: Rust 1.88+, `ffmpeg` and `ffprobe` on `PATH`. The Unix integration
tests create their own media and PTYs; no checked-in binaries or media fixtures.
