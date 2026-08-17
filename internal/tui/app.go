// Package tui is the Bubble Tea root model for vedit.
package tui

import (
	"fmt"
	"path/filepath"
	"strings"
	"time"

	"charm.land/bubbles/v2/key"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"

	"github.com/catalan-adobe/vroom/internal/model"
	"github.com/catalan-adobe/vroom/internal/render"
	th "github.com/catalan-adobe/vroom/internal/style"
	"github.com/catalan-adobe/vroom/internal/video"
)

// ─── Layout constants ──────────────────────────────────────────────────────────
//
// Screen layout (top → bottom):
//
//   header     (1 row)
//   separator  (1 row)
//   preview    (previewHeight rows) ← Kitty frame rendered via tea.Raw
//   separator  (1 row)
//   timeline   (4 rows: ruler / marks / segment strip / segment labels)
//   separator  (1 row)
//   segInfo    (1 row)
//   hintBar    (1 row)
//              ─────────────────
//   total:     previewHeight + 10

const fixedRows = 10 // all rows except the preview area

// App is the root Bubble Tea model.
type App struct {
	project *model.Project
	keys    keyMap
	kitty   bool // Kitty graphics protocol supported

	width  int
	height int

	cursor    float64 // current playhead position in seconds
	activeSeg int     // index of the currently focused segment
	playing   bool    // true while playback is active

	statusMsg string
	exporting bool
}

// ─── Messages ─────────────────────────────────────────────────────────────────

type frameMsg struct {
	png []byte
	t   float64
	err error
}

type exportDoneMsg struct{ err error }
type clearStatusMsg struct{}

// ─── Constructor ──────────────────────────────────────────────────────────────

// New creates the App from a probed project.
func New(proj *model.Project) App {
	return App{
		project: proj,
		keys:    defaultKeyMap(),
		kitty:   render.Supported(),
	}
}

// ─── Bubble Tea interface ──────────────────────────────────────────────────────

func (a App) Init() tea.Cmd {
	return nil // first frame arrives after WindowSizeMsg
}

func (a App) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {

	case tea.WindowSizeMsg:
		a.width = msg.Width
		a.height = msg.Height
		return a, extractFrameCmd(a.project.VideoPath, a.cursor,
			a.framePixelW(), a.framePixelH())

	case tea.KeyPressMsg:
		return a.handleKey(msg)

	case frameMsg:
		if msg.err != nil || len(msg.png) == 0 {
			return a, nil
		}
		rawCmd := a.sendFrameCmd(msg.png)
		if a.playing {
			// Advance cursor and request the next frame.
			a.cursor += 1.0 / a.project.FPS
			if a.cursor >= a.project.Duration {
				a.cursor = a.project.Duration
				a.playing = false
				return a, rawCmd
			}
			return a, tea.Batch(rawCmd,
				extractFrameCmd(a.project.VideoPath, a.cursor,
					a.framePixelW(), a.framePixelH()))
		}
		return a, rawCmd

	case exportDoneMsg:
		a.exporting = false
		if msg.err != nil {
			a.statusMsg = "Export failed: " + msg.err.Error()
		} else {
			a.statusMsg = "Exported → " + exportOutputPath(a.project.VideoPath)
		}
		return a, tea.Tick(3*time.Second, func(_ time.Time) tea.Msg {
			return clearStatusMsg{}
		})

	case clearStatusMsg:
		a.statusMsg = ""
	}

	return a, nil
}

func (a App) View() tea.View {
	v := tea.NewView(a.render())
	v.AltScreen = true
	v.WindowTitle = "vroom"
	return v
}

// ─── Key handler ──────────────────────────────────────────────────────────────

func (a App) handleKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if msg.String() == "ctrl+c" {
		return a, tea.Quit
	}
	switch {
	case key.Matches(msg, a.keys.Quit):
		return a, tea.Quit

	case key.Matches(msg, a.keys.SeekLeft):
		return a.seek(-0.5)
	case key.Matches(msg, a.keys.SeekRight):
		return a.seek(0.5)
	case key.Matches(msg, a.keys.SeekBigLeft):
		return a.seek(-5)
	case key.Matches(msg, a.keys.SeekBigRight):
		return a.seek(5)

	case key.Matches(msg, a.keys.AddMark):
		a.project.AddMark(a.cursor)
		a.activeSeg = a.segAtCursor()
		return a, nil

	case key.Matches(msg, a.keys.DelMark):
		a.project.RemoveMark(a.cursor)
		a.activeSeg = a.segAtCursor()
		return a, nil

	case key.Matches(msg, a.keys.NextSeg):
		segs := a.project.Segments()
		if a.activeSeg < len(segs)-1 {
			a.activeSeg++
			a.cursor = segs[a.activeSeg].Start
			return a, extractFrameCmd(a.project.VideoPath, a.cursor,
				a.framePixelW(), a.framePixelH())
		}
		return a, nil

	case key.Matches(msg, a.keys.PrevSeg):
		if a.activeSeg > 0 {
			a.activeSeg--
			segs := a.project.Segments()
			a.cursor = segs[a.activeSeg].Start
			return a, extractFrameCmd(a.project.VideoPath, a.cursor,
				a.framePixelW(), a.framePixelH())
		}
		return a, nil

	case key.Matches(msg, a.keys.ToggleCut):
		a.project.ToggleCut(a.activeSeg)
		return a, nil

	case key.Matches(msg, a.keys.SpeedUp):
		a.project.AdjustSpeed(a.activeSeg, 0.25)
		return a, nil

	case key.Matches(msg, a.keys.SpeedDown):
		a.project.AdjustSpeed(a.activeSeg, -0.25)
		return a, nil

	case key.Matches(msg, a.keys.PlayPause):
		a.playing = !a.playing
		if a.playing {
			return a, extractFrameCmd(a.project.VideoPath, a.cursor,
				a.framePixelW(), a.framePixelH())
		}
		return a, nil

	case key.Matches(msg, a.keys.Export):
		if !a.exporting {
			a.exporting = true
			a.statusMsg = "Exporting…"
			return a, exportCmd(
				a.project.VideoPath,
				a.project.Segments(),
				exportOutputPath(a.project.VideoPath),
			)
		}
		return a, nil
	}
	return a, nil
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

func (a App) seek(delta float64) (App, tea.Cmd) {
	a.cursor += delta
	if a.cursor < 0 {
		a.cursor = 0
	}
	if a.cursor > a.project.Duration {
		a.cursor = a.project.Duration
	}
	a.activeSeg = a.segAtCursor()
	return a, extractFrameCmd(a.project.VideoPath, a.cursor,
		a.framePixelW(), a.framePixelH())
}

func (a App) segAtCursor() int {
	segs := a.project.Segments()
	for i, s := range segs {
		if a.cursor >= s.Start && a.cursor < s.End {
			return i
		}
	}
	if len(segs) > 0 {
		return len(segs) - 1
	}
	return 0
}

// previewHeight is the number of rows the video preview occupies.
func (a App) previewHeight() int {
	h := a.height - fixedRows
	if h < 3 {
		h = 3
	}
	return h
}

// framePixelW/H: pixel dimensions for the extracted frame.
// Assuming ~8px wide × ~16px tall cells; keeps PNG small for fast PTY transfer.
func (a App) framePixelW() int {
	w := a.width * 8
	if w > 1280 {
		w = 1280
	}
	return w
}

func (a App) framePixelH() int {
	h := a.previewHeight() * 16
	if h > 720 {
		h = 720
	}
	return h
}

// ─── Commands ─────────────────────────────────────────────────────────────────

func extractFrameCmd(path string, t float64, pw, ph int) tea.Cmd {
	return func() tea.Msg {
		png, err := video.ExtractFrame(path, t, pw, ph)
		return frameMsg{png: png, t: t, err: err}
	}
}

func exportCmd(inputPath string, segs []model.Segment, outputPath string) tea.Cmd {
	return func() tea.Msg {
		err := video.Export(inputPath, segs, outputPath)
		return exportDoneMsg{err: err}
	}
}

// sendFrameCmd returns a tea.Cmd that transmits the PNG frame via
// the Kitty graphics protocol (tea.Raw). No-op on non-Kitty terminals.
func (a App) sendFrameCmd(png []byte) tea.Cmd {
	if !a.kitty || a.width == 0 || a.height == 0 {
		return nil
	}
	pH := a.previewHeight()
	if pH < 2 || a.width < 4 {
		return nil
	}
	// Preview area starts at terminal row 3 (1 header + 1 sep), col 1.
	frame := render.Frame(png, a.width, pH, 3, 1)
	del := render.DeleteAll()
	return tea.Sequence(tea.Raw(del), tea.Raw(frame))
}

// ─── Render ───────────────────────────────────────────────────────────────────

func (a App) render() string {
	if a.width == 0 {
		return "loading…"
	}

	w := a.width
	segs := a.project.Segments()

	// ── Header ──────────────────────────────────────────────────────────────
	title := th.AccentS.Render("vroom") + "  " +
		th.BoldS.Render(filepath.Base(a.project.VideoPath))
	pos := th.MutedS.Render(fmt.Sprintf(
		"[%s / %s]",
		th.FormatTime(a.cursor),
		th.FormatTime(a.project.Duration),
	))
	headerGap := w - lipgloss.Width(title) - lipgloss.Width(pos)
	if headerGap < 1 {
		headerGap = 1
	}
	header := title + strings.Repeat(" ", headerGap) + pos

	sep := th.DimS.Render(strings.Repeat("─", w))

	// ── Preview area (blank — Kitty frame is sent via tea.Raw) ──────────────
	pH := a.previewHeight()
	emptyRow := strings.Repeat(" ", w)
	previewLines := make([]string, pH)
	for i := range previewLines {
		previewLines[i] = emptyRow
	}
	// On non-Kitty terminals, display a placeholder in the middle row.
	if !a.kitty {
		mid := pH / 2
		label := th.MutedS.Render("[ video preview requires a Kitty-compatible terminal ]")
		pad := (w - lipgloss.Width(label)) / 2
		if pad < 0 {
			pad = 0
		}
		previewLines[mid] = strings.Repeat(" ", pad) + label +
			strings.Repeat(" ", w-pad-lipgloss.Width(label))
	}
	preview := strings.Join(previewLines, "\n")

	// ── Timeline ────────────────────────────────────────────────────────────
	timeline := renderTimeline(a.project, a.cursor, a.activeSeg, w)

	// ── Segment info ────────────────────────────────────────────────────────
	var segInfo string
	if len(segs) > 0 && a.activeSeg < len(segs) {
		s := segs[a.activeSeg]
		var opLabel string
		if s.Op == model.Cut {
			opLabel = th.DangerS.Render("CUT")
		} else {
			opLabel = lipgloss.NewStyle().Foreground(th.Success).Bold(true).Render("KEEP")
		}
		segInfo = fmt.Sprintf(
			"  Seg %d/%d  %s–%s  %s  speed: ×%.2f",
			a.activeSeg+1, len(segs),
			th.FormatTime(s.Start), th.FormatTime(s.End),
			opLabel, s.Speed,
		)
	}

	// ── Hint / status bar ───────────────────────────────────────────────────
	var hintBar string
	if a.statusMsg != "" {
		hintBar = "  " + th.AccentS.Render(a.statusMsg)
	} else {
		hintBar = th.DimS.Render(
			"  ← →: seek  h l: jump 5s  m: mark  M: del  " +
				"tab: seg  c: cut  +/−: speed  space: play  e: export  q: quit",
		)
	}

	return strings.Join([]string{
		header,
		sep,
		preview,
		sep,
		timeline,
		sep,
		segInfo,
		hintBar,
	}, "\n")
}

// ─── Timeline renderer ────────────────────────────────────────────────────────

func renderTimeline(proj *model.Project, cursor float64, activeSeg, width int) string {
	dur := proj.Duration
	segs := proj.Segments()
	marks := proj.Marks()

	if dur == 0 || width < 10 {
		return strings.Repeat("\n", 3)
	}

	// Map a time value to a character column [0, width-1].
	toX := func(t float64) int {
		x := int(t / dur * float64(width-1))
		if x < 0 {
			x = 0
		}
		if x >= width {
			x = width - 1
		}
		return x
	}

	// ── Line 1: time ruler ──────────────────────────────────────────────────
	ruler := []rune(strings.Repeat("─", width))
	for i := 0; i <= 5; i++ {
		t := dur * float64(i) / 5.0
		label := []rune(th.FormatTime(t))
		x := toX(t)
		for j, ch := range label {
			if x+j < width {
				ruler[x+j] = ch
			}
		}
	}

	// ── Line 2: mark positions and cursor ───────────────────────────────────
	markLine := []rune(strings.Repeat("─", width))
	for _, m := range marks {
		markLine[toX(m)] = '│'
	}
	markLine[toX(cursor)] = '▶'

	// ── Line 3: segment strip (coloured blocks) ─────────────────────────────
	var strip strings.Builder
	for i, s := range segs {
		x0 := toX(s.Start)
		x1 := toX(s.End)
		if i == len(segs)-1 {
			x1 = width // extend last segment to edge
		}
		w := x1 - x0
		if w < 1 {
			w = 1
		}

		var st lipgloss.Style
		var ch string
		if s.Op == model.Cut {
			st = lipgloss.NewStyle().Foreground(th.Danger)
			ch = "▒"
		} else {
			st = lipgloss.NewStyle().Foreground(th.SpeedColor(s.Speed))
			ch = "█"
		}
		if i == activeSeg {
			st = st.Reverse(true)
		}
		strip.WriteString(st.Render(strings.Repeat(ch, w)))
	}

	// ── Line 4: segment labels ───────────────────────────────────────────────
	var labels strings.Builder
	for i, s := range segs {
		x0 := toX(s.Start)
		x1 := toX(s.End)
		if i == len(segs)-1 {
			x1 = width
		}
		w := x1 - x0
		if w < 1 {
			w = 1
		}

		var label string
		switch {
		case s.Op == model.Cut:
			label = "CUT"
		case s.Speed != 1.0:
			label = fmt.Sprintf("×%.2f", s.Speed)
		default:
			label = "KEEP"
		}
		if len(label) < w {
			label += strings.Repeat(" ", w-len(label))
		} else if len(label) > w {
			label = label[:w]
		}

		var st lipgloss.Style
		if i == activeSeg {
			st = lipgloss.NewStyle().Foreground(th.Accent).Bold(true)
		} else {
			st = th.MutedS
		}
		labels.WriteString(st.Render(label))
	}

	return strings.Join([]string{
		string(ruler),
		string(markLine),
		strip.String(),
		labels.String(),
	}, "\n")
}

// ─── Path helpers ─────────────────────────────────────────────────────────────

func exportOutputPath(videoPath string) string {
	ext := filepath.Ext(videoPath)
	base := strings.TrimSuffix(videoPath, ext)
	return base + "_vroom" + ext
}
