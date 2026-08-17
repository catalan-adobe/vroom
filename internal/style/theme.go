// Package style provides shared colours and lipgloss styles.
package style

import (
	"fmt"

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
