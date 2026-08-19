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
	maxPreviewRows = 40 // hard cap: preview never taller than this
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
	playing     bool      // true while playback is active
	lastFrameAt time.Time // wall-clock time the last frame was displayed

	steps   []float64 // seek step ladder (seconds), computed from duration
	stepIdx int       // index into steps; ↑ increases, ↓ decreases

	frameMode bool // true → display frame numbers instead of timestamps

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

// seekSteps computes a duration-appropriate step ladder for timeline
// navigation. The finest step is always 1 frame; coarser steps scale with
// the video length so every tier is useful regardless of duration.
func seekSteps(duration, fps float64) []float64 {
	oneFrame := 1.0 / fps
	switch {
	case duration < 10*60: // < 10 min — 4 steps
		return []float64{oneFrame, 1, 5, 30}
	case duration < 60*60: // 10 min – 1 hr — 5 steps
		return []float64{oneFrame, 1, 10, 60, 5 * 60}
	default: // > 1 hr — 5 steps
		return []float64{oneFrame, 1, 30, 5 * 60, 20 * 60}
	}
}

// New creates the App from a probed project.
func New(proj *model.Project) App {
	steps := seekSteps(proj.Duration, proj.FPS)
	return App{
		project: proj,
		keys:    defaultKeyMap(),
		kitty:   render.Supported(),
		steps:   steps,
		stepIdx: 1, // start at second tier — usable out of the box
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
				a.framePixelH()),
		)

	case tea.KeyPressMsg:
		return a.handleKey(msg)

	case frameMsg:
		if msg.err != nil || len(msg.png) == 0 {
			return a, nil
		}
		// During seeking, discard frames that no longer match the current
		// cursor. Rapid arrow-key navigation launches many concurrent
		// goroutines; without this guard every completion sends tea.Raw
		// cursor movements that accumulate and corrupt bubbletea's layout.
		if !a.playing && msg.t != a.cursor {
			return a, nil
		}
		rawCmd := a.sendFrameCmd(msg.png)
		if a.playing {
			// Wall-clock advance: move by actual elapsed time so playback
			// stays at 1× regardless of how long ffmpeg took to extract
			// the frame (critical for large files with slow keyframe seeks).
			now := time.Now()
			elapsed := now.Sub(a.lastFrameAt)
			if elapsed > 2*time.Second {
				elapsed = 2 * time.Second // cap: don't jump more than 2s
			}
			a.lastFrameAt = now
			a.advanceCursor(elapsed)
			a.activeSeg = a.segAtCursor()
			if !a.playing {
				return a, rawCmd
			}
			return a, tea.Batch(rawCmd,
				extractFrameCmd(a.project.VideoPath, a.cursor,
					a.framePixelH()))
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
		return a.seek(-a.steps[a.stepIdx])
	case key.Matches(msg, a.keys.SeekRight):
		return a.seek(a.steps[a.stepIdx])
	case key.Matches(msg, a.keys.StepUp):
		if a.stepIdx < len(a.steps)-1 {
			a.stepIdx++
		}
		return a, nil
	case key.Matches(msg, a.keys.StepDown):
		if a.stepIdx > 0 {
			a.stepIdx--
		}
		return a, nil

	case key.Matches(msg, a.keys.AddMark):
		a.project.AddMark(a.cursor)
		a.activeSeg = a.segAtCursor()
		// ClearScreen wipes stale cells from any previous layout so the
		// timeline always redraws cleanly after a segment count change.
		return a, tea.Batch(
			tea.ClearScreen,
			extractFrameCmd(a.project.VideoPath, a.cursor,
				a.framePixelH()),
		)

	case key.Matches(msg, a.keys.DelMark):
		a.project.RemoveMark(a.cursor)
		a.activeSeg = a.segAtCursor()
		return a, tea.Batch(
			tea.ClearScreen,
			extractFrameCmd(a.project.VideoPath, a.cursor,
				a.framePixelH()),
		)

	case key.Matches(msg, a.keys.NextSeg):
		segs := a.project.Segments()
		if a.activeSeg < len(segs)-1 {
			a.activeSeg++
			a.cursor = segs[a.activeSeg].Start
			return a, extractFrameCmd(a.project.VideoPath, a.cursor,
				a.framePixelH())
		}
		return a, nil

	case key.Matches(msg, a.keys.PrevSeg):
		if a.activeSeg > 0 {
			a.activeSeg--
			segs := a.project.Segments()
			a.cursor = segs[a.activeSeg].Start
			return a, extractFrameCmd(a.project.VideoPath, a.cursor,
				a.framePixelH())
		}
		return a, nil

	case key.Matches(msg, a.keys.ToggleCut):
		a.project.ToggleCut(a.activeSeg)
		return a, nil

	case key.Matches(msg, a.keys.SpeedUp):
		a.project.SetSpeedStep(a.activeSeg, 2.0)
		return a, nil

	case key.Matches(msg, a.keys.SpeedDown):
		a.project.SetSpeedStep(a.activeSeg, 0.5)
		return a, nil

	case key.Matches(msg, a.keys.ToggleFrameMode):
		a.frameMode = !a.frameMode
		return a, nil

	case key.Matches(msg, a.keys.PlayPause):
		a.playing = !a.playing
		if a.playing {
			a.lastFrameAt = time.Now() // anchor wall-clock for first advance
			return a, extractFrameCmd(a.project.VideoPath, a.cursor,
				a.framePixelH())
		}
		return a, nil

	case key.Matches(msg, a.keys.Save):
		sp := model.SavePath(a.project.VideoPath)
		if err := a.project.Save(sp); err != nil {
			a.statusMsg = "Save failed: " + err.Error()
		} else {
			a.statusMsg = "Saved → " + filepath.Base(sp)
		}
		return a, tea.Tick(3*time.Second, func(_ time.Time) tea.Msg {
			return clearStatusMsg{}
		})

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

// fmtFrameCompact converts a time to a compact frame string using K-notation
// for numbers ≥1000 (e.g. 36000 → "36K", 1800 → "1.8K").
// Used for ruler tick labels where horizontal space is tight.
func fmtFrameCompact(t, fps float64) string {
	f := int(t * fps)
	if f < 1000 {
		return fmt.Sprintf("%d", f)
	}
	k := float64(f) / 1000.0
	if k == float64(int(k)) {
		return fmt.Sprintf("%dK", int(k))
	}
	return fmt.Sprintf("%.1fK", k)
}

// fmtFrameFull converts a time to a plain frame number string with no
// abbreviation — used where space is ample (header, segInfo).
func fmtFrameFull(t, fps float64) string {
	return fmt.Sprintf("%d", int(t*fps))
}

// fmtPos formats a position (seconds) as either a timestamp or a full frame
// number depending on the app's display mode.
func (a App) fmtPos(t float64) string {
	if a.frameMode {
		return fmtFrameFull(t, a.project.FPS)
	}
	return th.FormatTime(t)
}

// fmtStep formats a step duration as a compact human-readable string.
func fmtStep(s float64) string {
	switch {
	case s < 1:
		return "1f"
	case s < 60:
		if s == float64(int(s)) {
			return fmt.Sprintf("%ds", int(s))
		}
		return fmt.Sprintf("%.1fs", s)
	default:
		m := int(s) / 60
		if int(s)%60 == 0 {
			return fmt.Sprintf("%dm", m)
		}
		return fmt.Sprintf("%dm%ds", m, int(s)%60)
	}
}

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
		a.framePixelH())
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

// advanceCursor moves the playback cursor by the given wall-clock duration,
// honouring the edited timeline:
//   - KEEP ×1.0 → advance by elapsed seconds
//   - Speed ×N  → advance by elapsed × N
//   - CUT       → jump to the segment's end (skip it entirely)
//
// Using elapsed wall-clock time (not a fixed 1/FPS step) keeps playback
// at 1× regardless of ffmpeg extraction latency. Critical for large files
// where keyframe seeks take 200-500 ms.
func (a *App) advanceCursor(elapsed time.Duration) {
	segs := a.project.Segments()

	segAt := func(t float64) *model.Segment {
		for i := range segs {
			if t >= segs[i].Start && t < segs[i].End {
				return &segs[i]
			}
		}
		return nil
	}

	skipCuts := func() {
		for {
			s := segAt(a.cursor)
			if s == nil || s.Op != model.Cut {
				break
			}
			a.cursor = s.End
		}
	}

	cur := segAt(a.cursor)
	if cur == nil {
		a.cursor = a.project.Duration
		a.playing = false
		return
	}

	if cur.Op == model.Cut {
		a.cursor = cur.End
		skipCuts()
	} else {
		a.cursor += elapsed.Seconds() * cur.Speed
		skipCuts()
	}

	if a.cursor >= a.project.Duration {
		a.cursor = a.project.Duration
		a.playing = false
	}
}

// previewDims returns (rows, cols) for the inner preview area.
//
// rows: ideal for the video AR capped at maxPreviewRows and available space.
// cols: computed from rows to match the video AR, capped at terminal width-2.
//
// Terminal cells are assumed ~0.5 wide:tall (8×16 px), so:
//
//	rows = termCols × cellAR / videoAR
//	cols = rows    × videoAR / cellAR
func (a App) previewDims() (rows, cols int) {
	const cellAR = 0.5
	maxCols := a.width - 2
	if maxCols < 4 {
		maxCols = 4
	}

	videoAR := 16.0 / 9.0 // sensible default
	if a.project.PixelW > 0 && a.project.PixelH > 0 {
		videoAR = float64(a.project.PixelW) / float64(a.project.PixelH)
	}

	// Rows: how many rows does the video need at full terminal width?
	idealRows := int(float64(maxCols) * cellAR / videoAR)
	if idealRows < 3 {
		idealRows = 3
	}
	if idealRows > maxPreviewRows {
		idealRows = maxPreviewRows
	}
	avail := a.height - fixedRows
	if avail < 3 {
		avail = 3
	}
	if idealRows > avail {
		idealRows = avail
	}
	rows = idealRows

	// Cols: given the row count, how wide must the box be for the video AR?
	idealCols := int(float64(rows) * videoAR / cellAR)
	if idealCols > maxCols {
		idealCols = maxCols
	}
	if idealCols < 4 {
		idealCols = 4
	}
	cols = idealCols
	return
}

// previewHeight is the row count of the preview (for layout accounting).
func (a App) previewHeight() int {
	rows, _ := a.previewDims()
	return rows
}

// topPad returns blank rows above the UI for vertical centering.
func (a App) topPad() int {
	uiH := a.previewHeight() + fixedRows
	if uiH >= a.height {
		return 0
	}
	return (a.height - uiH) / 2
}

// framePixelH is the pixel height for ffmpeg frame extraction.
// We only constrain height; ffmpeg preserves the AR automatically.
func (a App) framePixelH() int {
	rows, _ := a.previewDims()
	h := rows * 16
	if h > 720 {
		h = 720
	}
	return h
}

// ─── Commands ─────────────────────────────────────────────────────────────────

func extractFrameCmd(path string, t float64, ph int) tea.Cmd {
	return func() tea.Msg {
		png, err := video.ExtractFrame(path, t, ph)
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
	_, pW := a.previewDims()
	boxLeft := (a.width - pW - 2) / 2 // centre the box horizontally
	// Frame inside the border: col = boxLeft+2 (1-indexed), row = topPad+3.
	frame := render.Frame(png, pW, pH, a.topPad()+3, boxLeft+2)
	// Frame() wraps the Kitty data in DECSC/DECRC (\x1b7...\x1b8):
	// cursor is saved before the positioning escape and restored after,
	// so bubbletea's diff renderer always finds the cursor where it left
	// it — no ghost cells, no ClearScreen, no flash.
	return tea.Raw(render.DeleteAll() + frame)
}

// ─── Render ───────────────────────────────────────────────────────────────────

func (a App) render() string {
	if a.width == 0 || a.height == 0 {
		return "loading…"
	}

	w := a.width
	segs := a.project.Segments()
	pH, pW := a.previewDims()
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
		a.fmtPos(a.project.OutputPosition(a.cursor)),
		a.fmtPos(a.project.OutputDuration()),
	))
	headerGap := w - lipgloss.Width(title) - lipgloss.Width(pos)
	if headerGap < 1 {
		headerGap = 1
	}
	header := title + strings.Repeat(" ", headerGap) + pos

	sep := th.DimS.Render(strings.Repeat("─", w))

	// ── Preview area: centred box sized to the video aspect ratio ───────────
	// The box is pW cols wide (inner), centred in the terminal.
	// Border characters are text (z=0), so they render in front of the
	// Kitty frame (z=-1), giving a clean visible container.
	bStyle := th.MutedS
	boxW := pW + 2
	boxLeft := (w - boxW) / 2
	if boxLeft < 0 {
		boxLeft = 0
	}
	boxRight := boxLeft + boxW
	rightPadW := w - boxRight
	if rightPadW < 0 {
		rightPadW = 0
	}
	lPad := strings.Repeat(" ", boxLeft)
	rPad := strings.Repeat(" ", rightPadW)
	sideL := bStyle.Render("│")
	sideR := bStyle.Render("│")
	topBorder := lPad + bStyle.Render("╭"+strings.Repeat("─", pW)+"╮") + rPad
	botBorder := lPad + bStyle.Render("╰"+strings.Repeat("─", pW)+"╯") + rPad
	innerRow := lPad + sideL + strings.Repeat(" ", pW) + sideR + rPad
	previewLines := make([]string, pH)
	for i := range previewLines {
		previewLines[i] = innerRow
	}
	if !a.kitty {
		mid := pH / 2
		label := "[ Kitty / WezTerm / Ghostty required for video preview ]"
		if len(label) > pW {
			label = label[:pW]
		}
		pad := (pW - len(label)) / 2
		previewLines[mid] = lPad + sideL +
			strings.Repeat(" ", pad) + th.MutedS.Render(label) +
			strings.Repeat(" ", pW-pad-len(label)) + sideR + rPad
	}
	preview := strings.Join(previewLines, "\n")

	// ── Timeline ────────────────────────────────────────────────────────────
	timeline := renderTimeline(a.project, a.cursor, a.activeSeg, w, a.frameMode)

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
			a.fmtPos(s.Start), a.fmtPos(s.End),
			opLabel, s.Speed,
		)
	}

	// ── Hint / status bar ───────────────────────────────────────────────────
	// Step ladder: show all tiers, highlight the active one.
	stepParts := make([]string, len(a.steps))
	for i, s := range a.steps {
		if i == a.stepIdx {
			stepParts[i] = th.AccentS.Render("[" + fmtStep(s) + "]")
		} else {
			stepParts[i] = th.DimS.Render(fmtStep(s))
		}
	}
	stepLadder := strings.Join(stepParts, th.DimS.Render("·"))

	// Frame-mode indicator shown next to the step ladder.
	var modeTag string
	if a.frameMode {
		modeTag = "  " + th.AccentS.Render("[f]") + th.DimS.Render(" frames")
	} else {
		modeTag = "  " + th.DimS.Render("[f] time")
	}

	var hintBar string
	if a.statusMsg != "" {
		hintBar = "  " + th.AccentS.Render(a.statusMsg)
	} else {
		hintBar = "  " + stepLadder + modeTag +
			th.DimS.Render("  ↑↓:step  ←→:seek  m:mark  M:del  tab:seg  c:cut  +−:speed  space:play  e:export  q:quit")
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
	parts = append(parts, header, topBorder, preview, botBorder, timeline, sep, segInfo, hintBar)
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

func renderTimeline(proj *model.Project, cursor float64, activeSeg, width int, frameMode bool) string {
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

	// rulerLabel formats a tick value as either a timestamp or a compact
	// frame number depending on the display mode.
	rulerLabel := func(t float64) string {
		if frameMode {
			return fmtFrameCompact(t, proj.FPS)
		}
		return th.FormatTime(t)
	}

	// ── Line 1: ruler ─────────────────────────────────────────────────────────
	ruler := []rune(strings.Repeat("─", width))
	for i := 0; i <= 5; i++ {
		t := dur * float64(i) / 5.0
		label := []rune(rulerLabel(t))
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
