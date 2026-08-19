// Package style provides shared colours and lipgloss styles.
package style

import (
	"fmt"
	"math"

	"charm.land/lipgloss/v2"
)

// Colour palette.
var (
	Accent  = lipgloss.Color("#00E5A0") // teal — UI highlights
	Muted   = lipgloss.Color("#777777") // hints, labels
	Dim     = lipgloss.Color("#444444") // very subtle text
	Danger  = lipgloss.Color("#FF4455") // cut segments
	Warning = lipgloss.Color("#FFAA00") // speed-adjusted segments
	Success = lipgloss.Color("#00CC77") // keep segments
	Border  = lipgloss.Color("#333333")
)

// Reusable styles.
var (
	AccentS = lipgloss.NewStyle().Foreground(Accent).Bold(true)
	MutedS  = lipgloss.NewStyle().Foreground(Muted)
	DimS    = lipgloss.NewStyle().Foreground(Dim)
	BoldS   = lipgloss.NewStyle().Bold(true)
	DangerS = lipgloss.NewStyle().Foreground(Danger).Bold(true)
)

// SpeedColorHex returns a hex colour string for the given speed multiplier.
// Call lipgloss.Color(th.SpeedColorHex(s)) to use as a lipgloss foreground.
//
// Colour mapping on a log2 scale so each ×2 / ÷2 step produces the same
// visual shift regardless of the current speed:
//
//	×1.0        → neutral gray   #AAAAAA
//	×0.5 / ÷2   → mild orange
//	×0.25 / ÷4  → bright orange  #FF6600  (t = 1 at 3 halvings, i.e. log2 = -3)
//	×2.0 / ×2   → mild green
//	×8.0 / ×3   → bright green   #00EE55  (t = 1 at 3 doublings, i.e. log2 = +3)
//
// Saturates beyond ±3 doublings; further steps stay at the extreme colour.
func SpeedColorHex(speed float64) string {
	const (
		nR, nG, nB = 0xAA, 0xAA, 0xAA // neutral  #AAAAAA
		sR, sG, sB = 0xFF, 0x66, 0x00  // slow max #FF6600
		fR, fG, fB = 0x00, 0xEE, 0x55  // fast max #00EE55
	)
	clampLerp := func(a, b int, t float64) int {
		v := float64(a) + t*float64(b-a)
		if v < 0 {
			return 0
		}
		if v > 255 {
			return 255
		}
		return int(v + 0.5)
	}
	if speed <= 0 {
		return fmt.Sprintf("#%02X%02X%02X", sR, sG, sB)
	}
	// log2 of speed: negative = slower, positive = faster.
	// Normalise to [-1, 1] over ±3 doublings then apply t^0.7 ramp.
	l := math.Log2(speed)
	const span = 3.0
	switch {
	case l < 0:
		t := math.Min(-l/span, 1.0)
		t = math.Pow(t, 0.7)
		return fmt.Sprintf("#%02X%02X%02X",
			clampLerp(nR, sR, t),
			clampLerp(nG, sG, t),
			clampLerp(nB, sB, t),
		)
	case l > 0:
		t := math.Min(l/span, 1.0)
		t = math.Pow(t, 0.7)
		return fmt.Sprintf("#%02X%02X%02X",
			clampLerp(nR, fR, t),
			clampLerp(nG, fG, t),
			clampLerp(nB, fB, t),
		)
	default:
		return "#AAAAAA"
	}
}

// FormatTime formats seconds as M:SS.d (e.g. "1:23.4").
func FormatTime(sec float64) string {
	if sec < 0 {
		sec = 0
	}
	total := int(sec)
	m := total / 60
	s := total % 60
	d := int((sec-float64(total))*10)
	return fmt.Sprintf("%d:%02d.%d", m, s, d)
}
