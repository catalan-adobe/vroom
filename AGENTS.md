# vroom — Agent Guide

Terminal video editor built with Go + Bubble Tea v2. Cut, trim, and retime video without leaving your shell.

## Architecture

```
cmd/vroom/main.go           entry point: ffprobe → load .vroom → launch TUI
internal/
  model/
    project.go              editing state: marks, ops (cut/keep), speeds
    save.go                 save/load to .vroom JSON file
  video/
    probe.go                ffprobe wrapper → Info{Duration, FPS, Width, Height}
    frames.go               ffmpeg frame extraction (scale=-2:H, PNG pipe)
    export.go               ffmpeg filter graph: trim+setpts+concat
  render/
    kitty.go                Kitty graphics protocol encoder
  tui/
    app.go                  root Bubble Tea model + all rendering
    keys.go                 key bindings
  style/
    theme.go                colours, SpeedColorHex gradient, FormatTime
```

## Key Design Decisions

### Data model
- Marks are `[]float64` (sorted timestamps). Segments are derived from marks on each call to `Segments()`.
- Segment ops and speeds are keyed by **segment start time** (not index). This survives mark add/remove without losing settings.
- `OutputPosition(t)` and `OutputDuration()` convert original-video time to edited-output time: CUT segments contribute 0, speed segments divide time by `speed`.

### Video preview (Kitty graphics protocol)
- Preview uses the [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) via `tea.Raw`.
- Frame bytes are extracted with `ffmpeg scale=-2:H` (preserves aspect ratio; only height is specified).
- Preview box is **centred horizontally**, sized to match the video aspect ratio: `cols = rows × videoAR / cellAR` (cellAR ≈ 0.5, i.e. cells are ~2× taller than wide).
- **DECSC/DECRC cursor save-restore** (`\x1b7` / `\x1b8`) wrap every Kitty positioning escape. This is critical — see Pitfalls below.

### Step-based timeline navigation

`←/→` seek by the currently selected step. `↑/↓` cycle the step up or down through a duration-appropriate ladder — floor is always 1 frame, ceiling scales with video length:

| Duration | Steps |
|---|---|
| < 10 min | 1f · 1s · 5s · 30s |
| 10 min – 1 hr | 1f · 1s · 10s · 1min · 5min |
| > 1 hr | 1f · 1s · 30s · 5min · 20min |

`1f` and `1s` are always the two finest steps regardless of duration.

The hint bar renders the full ladder with the active step highlighted: `1f · [1s] · 10s · 1min`. `stepIdx` starts at 1 (coarse-but-not-glacial default).

### Timeline-aware playback
- `advanceCursor()` advances in original-video time respecting the edited timeline: KEEP segments advance by `speed/FPS`, CUT segments jump to `segment.End`.
- Stale frame filter: `if !a.playing && msg.t != a.cursor { return a, nil }` — discards frames from goroutines launched by earlier seek positions.

### Segment strip rendering
- Uses **raw ANSI escape codes** (`\x1b[48;2;R;G;Bm`) for background-coloured bars, NOT lipgloss per-segment styles. Lipgloss `Width(w)` misbehaves at small values (1–3 cols), causing lines to overflow and wrap to wrong rows.
- `segBlock()` function + `hexRGB()` helper produce predictable output at every width.
- Speed colour gradient: neutral gray at ×1.0 → orange at ×0.25, green at ×4.0, with `t^0.7` non-linear ramp.

## Bubble Tea v2 Conventions

- Import path: `charm.land/bubbletea/v2` (NOT `github.com/charmbracelet/bubbletea`). Same for `bubbles` and `lipgloss`.
- `View()` returns `tea.View` (not `string`). Use `tea.NewView(content)` with `v.AltScreen = true`.
- Space key binding: `key.WithKeys("space")` — NOT `" "` (literal space). This differs from v1.
- Key press type: `tea.KeyPressMsg` (not `tea.KeyMsg`).

## Pitfalls

### 1. DECSC/DECRC is mandatory around Kitty frames
`tea.Raw` moves the terminal cursor invisibly from bubbletea's perspective. The diff renderer then writes subsequent view updates at wrong row offsets, leaving ghost timeline strips, duplicate segInfo rows, or `▶` cursor markers scattered across the marks line.

**Fix:** `\x1b7` (save cursor) before the Kitty positioning escape, `\x1b8` (restore cursor) after the Kitty data — implemented in `render.Frame()`.

### 2. `\x1b[2J` (ClearScreen) deletes Kitty images
The Kitty protocol spec says placed images survive screen clears, but WezTerm and Ghostty both delete them on `\x1b[2J`. `tea.ClearScreen` causes a visible flash. Use DECSC/DECRC (pitfall 1) instead of ClearScreen to avoid ghost content.

### 3. lipgloss v2 colour type gotcha
`lipgloss.Color` is `func(s string) color.Color` — a function, not a type. Returning `color.Color` from a helper and passing it to `Foreground()` **silently produces no colour**. Always call `lipgloss.Color()` at the callsite:
```go
// WRONG
func speedColor() color.Color { return lipgloss.Color("#FF6600") }
st.Foreground(speedColor())          // silently ignored

// RIGHT
func speedColorHex() string { return "#FF6600" }
st.Foreground(lipgloss.Color(speedColorHex()))  // works
```

### 4. lipgloss `Width(w)` breaks at small values
`lipgloss.NewStyle().Width(w)` with `w = 1–3` produces output wider than the terminal, causing the strip line to wrap onto the next terminal row. This makes timeline content appear at wrong rows and corrupts the diff renderer's model. Use raw ANSI codes for segment blocks (see `segBlock()` in `app.go`).

### 5. ffmpeg frame scaling — always use `scale=-2:H`
`scale=W:H` forces exact dimensions and **distorts** the video. Use `scale=-2:H` (height only; width auto-computed, rounded to even). Kitty scales the received image to fill the specified `c×r` cell area.

### 6. Stale frames corrupt the diff renderer
Rapid seeking launches many concurrent `extractFrameCmd` goroutines. Each completion sends `tea.Raw` with cursor movement. Without filtering, these accumulate and corrupt bubbletea's layout. Always discard frames that no longer match the current cursor:
```go
if !a.playing && msg.t != a.cursor {
    return a, nil
}
```

### 7. Segment ops/speeds are keyed by start time, not index
`p.ops[segStart]` and `p.speeds[segStart]` survive mark add/remove. Index-based keys don't — they shift when a mark is inserted before an existing one.

### 8. `OutputPosition` must break after partial segment
The loop accumulating output position must `break` immediately after adding a partial contribution (cursor inside a segment). Continuing to the next segment and relying on `t <= nextSeg.Start` to break works for non-CUT segments but fails when a CUT appears between two KEEP segments.

## Running / Building

```bash
go build -o vroom ./cmd/vroom
./vroom video.mp4          # auto-loads video.vroom if it exists
```

**Dependencies:** Go 1.24+, `ffmpeg` + `ffprobe` on `$PATH`. Kitty-compatible terminal (Kitty, WezTerm, Ghostty) for video preview; character-mode fallback otherwise.

## Save Format (.vroom)

```json
{
  "version": 1,
  "video": "/path/to/video.mp4",
  "segments": [
    {"start": 0.0},
    {"start": 5.2, "cut": true},
    {"start": 10.5, "speed": 2.0}
  ]
}
```

Marks = all `start > 0`. Missing `cut`/`speed` fields default to `false`/`1.0`.
