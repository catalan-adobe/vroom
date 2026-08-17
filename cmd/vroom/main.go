// Command vedit is a TUI video editor for simple cut/trim/speed edits.
//
// Usage: vedit <video-file>
package main

import (
	"fmt"
	"os"

	tea "charm.land/bubbletea/v2"

	"github.com/catalan-adobe/vroom/internal/model"
	"github.com/catalan-adobe/vroom/internal/tui"
	"github.com/catalan-adobe/vroom/internal/video"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintf(os.Stderr, "vedit: %v\n", err)
		os.Exit(1)
	}
}

func run() error {
	if len(os.Args) < 2 {
		return fmt.Errorf("usage: vedit <video-file>")
	}
	path := os.Args[1]

	if _, err := os.Stat(path); err != nil {
		return fmt.Errorf("open %q: %w", path, err)
	}

	fmt.Fprintf(os.Stderr, "Probing %s…\n", path)
	info, err := video.Probe(path)
	if err != nil {
		return fmt.Errorf("probe: %w", err)
	}

	proj := model.NewProject(path, info.Duration, info.FPS, info.Width, info.Height)
	app := tui.New(proj)

	p := tea.NewProgram(app)
	_, err = p.Run()
	return err
}
