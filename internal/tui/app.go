// Package tui is the Bubble Tea root model for vedit.
package tui

import (
	"fmt"
	"path/filepath"
	"strconv"
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
//   topPad     (variable — centers UI vertically)
//   header     (1 row)
//   separator  (1 row)
//   preview    (previewHeight rows) ← Kitty frame at row topPad+3
//   separator  (1 row)
//   timeline   (3 rows: ruler / marks / segment strip)
//   separator  (1 row)
//   segInfo    (1 row)
//   hintBar    (1 row)
//   bottomPad  (variable)
//              ─────────────────
//   total:     height (exact)

const (
	fixedRows      = 9  // rows excluding preview and padding
	maxPreviewRows = 20 // hard cap: preview never taller than this
)

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
		// Force a full repaint so stale cells from the old layout are
		// completely overwritten before the new frame arrives.
		return a, tea.Batch(
			tea.ClearScreen,
			extractFrameCmd(a.project.VideoPath, a.cursor,
				a.framePixelW(), a.framePixelH()),
		)

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
		// ClearScreen wipes stale cells from any previous layout so the
		// timeline always redraws cleanly after a segment count change.
		return a, tea.Batch(
			tea.ClearScreen,
			extractFrameCmd(a.project.VideoPath, a.cursor,
				a.framePixelW(), a.framePixelH()),
		)

	case key.Matches(msg, a.keys.DelMark):
		a.project.RemoveMark(a.cursor)
		a.activeSeg = a.segAtCursor()
		return a, tea.Batch(
			tea.ClearScreen,
			extractFrameCmd(a.project.VideoPath, a.cursor,
				a.framePixelW(), a.framePixelH()),
		)

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

// idealPreviewRows returns the number of rows required to display the video
// at its native aspect ratio given the current terminal width.
// Terminal cells are assumed ~0.5 wide:tall (8×16 px typical), so a column
// maps to half a pixel-row in aspect-ratio terms.
func (a App) idealPreviewRows() int {
	if a.project.PixelW == 0 || a.project.PixelH == 0 || a.width == 0 {
		return maxPreviewRows
	}
	videoAR := float64(a.project.PixelW) / float64(a.project.PixelH)
	const cellAR = 0.5 // cell width / cell height ≈ 8px/16px
	rows := int(float64(a.width) * cellAR / videoAR)
	if rows < 3 {
		return 3
	}
	if rows > maxPreviewRows {
		return maxPreviewRows
	}
	return rows
}

// previewHeight returns the clamped preview row count:
// ideal aspect-ratio rows, bounded by maxPreviewRows and available space.
func (a App) previewHeight() int {
	ideal := a.idealPreviewRows()
	avail := a.height - fixedRows
	if avail < 3 {
		avail = 3
	}
	if ideal > avail {
		return avail
	}
	return ideal
}

// topPad returns the number of blank rows to add above the UI so that
// the content is vertically centred in the terminal.
func (a App) topPad() int {
	uiH := a.previewHeight() + fixedRows
	if uiH >= a.height {
		return 0
	}
	return (a.height - uiH) / 2
}

// framePixelW/H: pixel dimensions for the extracted frame.
// Sized to the preview area at ~8×16px per cell, capped to keep PNG small.
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
	// Preview area starts at row topPad+3 (padding + header + sep), col 1.
	frame := render.Frame(png, a.width, pH, a.topPad()+3, 1)
	del := render.DeleteAll()
	return tea.Sequence(tea.Raw(del), tea.Raw(frame))
}

// ─── Render ───────────────────────────────────────────────────────────────────

func (a App) render() string {
	if a.width == 0 || a.height == 0 {
		return "loading…"
	}

	w := a.width
	segs := a.project.Segments()
	pH := a.previewHeight()
	tPad := a.topPad()
	bPad := a.height - pH - fixedRows - tPad
	if bPad < 0 {
		bPad = 0
	}

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

	// Pad short lines to the full terminal width so stale characters from a
	// previous render (different layout or different content length) are
	// completely overwritten and never bleed through.
	segInfo = padToWidth(segInfo, w)
	hintBar = padToWidth(hintBar, w)

	// Build padding rows (full-width spaces so they overwrite stale cells).
	padLine := strings.Repeat(" ", w)
	parts := make([]string, 0, a.height)
	for range tPad {
		parts = append(parts, padLine)
	}
	parts = append(parts, header, sep, preview, sep, timeline, sep, segInfo, hintBar)
	for range bPad {
		parts = append(parts, padLine)
	}
	return strings.Join(parts, "\n")
}

// padToWidth pads a (possibly ANSI-styled) string to exactly w visible
// columns by appending plain spaces. If the string is already wider, it
// is returned unchanged.
func padToWidth(s string, w int) string {
	vw := lipgloss.Width(s)
	if vw < w {
		return s + strings.Repeat(" ", w-vw)
	}
	return s
}

// segBlock renders a w-column background-coloured terminal cell block with
// label text left-aligned inside it. Uses raw ANSI escape codes so the
// output width is always exactly w columns regardless of w's size.
func segBlock(bgHex, fgHex string, w int, label string, bold bool) string {
	if w <= 0 {
		return ""
	}
	// Truncate or pad the label to exactly w rune columns.
	lr := []rune(label)
	var content string
	if len(lr) >= w {
		content = string(lr[:w])
	} else {
		content = string(lr) + strings.Repeat(" ", w-len(lr))
	}
	bgR, bgG, bgB := hexRGB(bgHex)
	fgR, fgG, fgB := hexRGB(fgHex)
	var sb strings.Builder
	fmt.Fprintf(&sb, "\x1b[48;2;%d;%d;%dm\x1b[38;2;%d;%d;%dm",
		bgR, bgG, bgB, fgR, fgG, fgB)
	if bold {
		sb.WriteString("\x1b[1m")
	}
	sb.WriteString(content)
	sb.WriteString("\x1b[0m")
	return sb.String()
}

// hexRGB parses a "#RRGGBB" hex colour string into its R, G, B components.
func hexRGB(hex string) (int, int, int) {
	hex = strings.TrimPrefix(hex, "#")
	if len(hex) != 6 {
		return 170, 170, 170
	}
	r, _ := strconv.ParseInt(hex[0:2], 16, 32)
	g, _ := strconv.ParseInt(hex[2:4], 16, 32)
	b, _ := strconv.ParseInt(hex[4:6], 16, 32)
	return int(r), int(g), int(b)
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

	// ── Line 3: segment strip with embedded labels ──────────────────────────
	// Each segment block is coloured by speed/cut and its label
	// (KEEP / CUT / ×N.NN) is written into the leading characters of the
	// block itself, avoiding a separate row that collides with the ruler.
	var strip strings.Builder
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

		// Solid background bar: spaces fill the width, label sits at the left.
		// Using Background() instead of Foreground()+block-chars means the
		// entire cell is painted, preventing old content from bleeding through.
		var bgHex, fgHex string
		var label string
		if s.Op == model.Cut {
			bgHex = "#882233" // dark crimson
			fgHex = "#FFFFFF" // white text — enough contrast on dark red
			label = " CUT "
		} else {
			bgHex = th.SpeedColorHex(s.Speed)
			fgHex = "#111111" // dark text on any speed colour
			if s.Speed != 1.0 {
				label = fmt.Sprintf(" ×%.2f ", s.Speed)
			} else {
				label = " KEEP "
			}
		}

		// Use raw ANSI codes instead of lipgloss for each block.
		// lipgloss Width(w) misbehaves at small values (1-3 cols), producing
		// lines that wrap or leave transparent cells. Direct escape sequences
		// are fully predictable at every width.
		strip.WriteString(segBlock(bgHex, fgHex, w, label, i == activeSeg))
	}

	// The segment widths sum to exactly `width` (segments partition [0,width]
	// without gaps), so no padding is needed — and using lipgloss.Width on an
	// ANSI-background string returns the wrong value, which caused the strip
	// to double-wrap and appear in the wrong terminal row.
	stripStr := strip.String()

	return strings.Join([]string{
		string(ruler),
		string(markLine),
		stripStr,
	}, "\n")
}

// ─── Path helpers ─────────────────────────────────────────────────────────────

func exportOutputPath(videoPath string) string {
	ext := filepath.Ext(videoPath)
	base := strings.TrimSuffix(videoPath, ext)
	return base + "_vroom" + ext
}
