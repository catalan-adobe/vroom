# vroom 🎬

Cut, trim, and retime video without leaving your terminal. vroom is built with Rust,
[Ratatui](https://ratatui.rs/), and [Crossterm](https://github.com/crossterm-rs/crossterm).
It uses FFmpeg for preview and export and the
[Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) to show video frames.

## Requirements

- Rust 1.88 or newer and Cargo to build from source
- `ffmpeg` and `ffprobe` on your `PATH`
- An interactive terminal. Kitty, WezTerm, and Ghostty display the video preview.
  On other terminals the preview is a text placeholder; editing and export still work.

## Install

```sh
git clone https://github.com/catalan-adobe/vroom
cd vroom
cargo install --locked --path .
```

To build without installing, run `cargo build --release --locked` and use
`./target/release/vroom`.

## Usage

```sh
vroom "my video.mp4"
```

1. Seek with `←` / `→` and press `m` to split the source into segments.
2. Select a segment, then press `c` to cut it or `+` / `-` to change its speed.
   The timeline shows source time, while the header shows the edited output position and duration.
3. Press `s` to save your edits to `my video.vroom`. vroom automatically loads that file
   the next time you open the same video, including project files saved by the Go version.
   Quitting does **not** save automatically.
4. Press `e` to render the current edits into `my video_vroom.mp4`. Export keeps the original
   container extension, overwrites an existing export, and leaves the source untouched.
   As in the Go version, exports are **video-only** (no audio). FFmpeg diagnostics go into
   `my video_vroom.log`, including when FFmpeg fails. An export already in progress can
   finish after you quit the editor.

## Keybindings

| Key | Action |
|---|---|
| `←` / `→` | Seek backward / forward by the selected step (default: 1 second) |
| `↑` / `↓` | Increase / decrease the seek step, from one frame to larger steps that depend on video length |
| `m` / `M` | Add a mark at the cursor / remove the nearest mark within 1 second |
| `Tab` / `Shift+Tab` | Go to the start of the next / previous segment |
| `c` | Toggle cut / keep on the selected segment |
| `+` or `=` / `-` | Double / halve the selected segment's speed (minimum 1/64×) |
| `Space` | Play / pause the edited preview |
| `f` | Toggle time / frame numbers (using the source frame rate) |
| `s` | Save the `.vroom` project beside the source video |
| `e` | Export the edited video |
| `q` / `Ctrl+C` | Quit |

The segment strip is gray at normal speed, orange for slowed segments, green for sped-up segments,
and dark red for cuts. The selected segment's label is bold.

## Development

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

On Unix, `cargo test --locked --test terminal` runs the executable in a pseudoterminal. The
real-media test decodes preview PNGs to verify seeking and checks that exported frames reflect
unsaved edits while reopening restores the last save. Controlled FFmpeg shims test pending-frame
cancellation on seek and resize, playback recovery and cut skipping, and failure paths. The harness
checks the reconstructed terminal screen and terminal restoration without requiring a desktop
terminal emulator.

## License

MIT
