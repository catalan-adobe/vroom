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

// SpeedColor returns a terminal colour for the given speed multiplier.
//
//	1.0       → neutral gray  (#AAAAAA)
//	< 1.0     → orange, more saturated the slower  (0.25× = #FF6600)
//	> 1.0     → green, more saturated the faster   (4×   = #00EE55)
//
// A slight non-linear ramp (t^0.7) is applied so even small speed
// changes produce a visible colour shift.
// SpeedColorHex returns a hex colour string for the given speed multiplier.
// Call lipgloss.Color(th.SpeedColorHex(s)) to use as a lipgloss foreground.
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
	switch {
	case speed < 1.0:
		t := math.Min((1.0-speed)/0.75, 1.0)
		t = math.Pow(t, 0.7)
			return fmt.Sprintf("#%02X%02X%02X",
			clampLerp(nR, sR, t),
			clampLerp(nG, sG, t),
			clampLerp(nB, sB, t),
		)
	case speed > 1.0:
		t := math.Min((speed-1.0)/3.0, 1.0)
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
