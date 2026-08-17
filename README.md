# vroom 🎬

Terminal video editor. Cut, trim, and retime video without leaving your shell.

Built with [Go](https://go.dev) + [Bubble Tea](https://github.com/charmbracelet/bubbletea). Video preview via the [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/).

## Demo

```
vedit  demo.mp4                                        [0:05.2 / 0:26.3]
────────────────────────────────────────────────────────────────────────
│                                                                       │
│                     (video frame preview)                             │
│                                                                       │
────────────────────────────────────────────────────────────────────────
0:00.0        0:05.2       0:10.5       0:15.7       0:21.0      0:26.3
─────────────────│──────────────────────────────────────────────────────
████████████████▒▒▒▒▒▒▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓████████████████████████████████
KEEP             CUT   ×2.00            KEEP
────────────────────────────────────────────────────────────────────────
  Seg 1/4  0:00.0–0:05.2  KEEP  speed: ×1.00
  ← →: seek  h l: jump 5s  m: mark  M: del  tab: seg  c: cut  +/−: speed  space: play  e: export  q: quit
```

## Requirements

- [Go 1.24+](https://go.dev/dl/)
- [ffmpeg](https://ffmpeg.org) + ffprobe on `$PATH`
- A [Kitty](https://sw.kovidgoyal.net/kitty/), [WezTerm](https://wezfurlong.org/wezterm/), or [Ghostty](https://ghostty.org/) terminal for video preview

## Install

```sh
go install github.com/catalan-adobe/vroom/cmd/vroom@latest
```

Or build from source:

```sh
git clone https://github.com/catalan-adobe/vroom
cd vroom
go build -o vroom ./cmd/vroom
```

## Usage

```sh
vroom video.mp4
```

Output is written to `video_vroom.mp4` alongside the original when you press `e`.

## Keybindings

| Key | Action |
|-----|--------|
| `←` / `→` | Seek ±0.5s |
| `h` / `l` | Seek ±5s |
| `m` | Add mark at cursor |
| `M` | Remove nearest mark |
| `tab` / `⇧tab` | Select next / previous segment |
| `c` | Toggle cut on selected segment |
| `+` / `-` | Speed ×0.25 up / down |
| `space` | Play / pause preview |
| `e` | Export edited video |
| `q` | Quit |

## How it works

1. **Add marks** at timestamps to slice the video into segments
2. **Per segment**: mark it as `CUT` (excluded from output) or adjust its **speed** (`×0.5` slow motion → `×4.0` fast forward)
3. **Export** — vroom builds an ffmpeg filter graph (`trim` + `setpts` + `concat`) and renders the result

## License

MIT
