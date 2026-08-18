package model

import (
	"encoding/json"
	"os"
	"sort"
)

// saveFile is the on-disk JSON structure for a vroom project.
type saveFile struct {
	Version  int            `json:"version"`
	Video    string         `json:"video"`
	Segments []savedSegment `json:"segments"`
}

// savedSegment encodes one segment's state.
// Marks are implicitly defined by all Start values > 0.
type savedSegment struct {
	Start float64 `json:"start"`
	Cut   bool    `json:"cut,omitempty"`
	Speed float64 `json:"speed,omitempty"`
}

// SavePath returns the .vroom file path that corresponds to videoPath.
// e.g. /videos/demo.mp4 → /videos/demo.mp4.vroom
func SavePath(videoPath string) string {
	return videoPath + ".vroom"
}

// Save writes the project's editing state to path as JSON.
func (p *Project) Save(path string) error {
	segs := p.Segments()
	saved := make([]savedSegment, len(segs))
	for i, s := range segs {
		saved[i] = savedSegment{
			Start: s.Start,
			Cut:   s.Op == Cut,
			Speed: s.Speed,
		}
	}
	f := saveFile{Version: 1, Video: p.VideoPath, Segments: saved}
	data, err := json.MarshalIndent(f, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, data, 0o644)
}

// Load restores editing state from a .vroom file into p.
// It does not change VideoPath, Duration, FPS, or pixel dimensions.
func (p *Project) Load(path string) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	var f saveFile
	if err := json.Unmarshal(data, &f); err != nil {
		return err
	}

	// Reset, then restore from the saved segment array.
	p.marks = nil
	p.ops = make(map[float64]Op)
	p.speeds = make(map[float64]float64)

	for _, s := range f.Segments {
		if s.Start > 0 {
			p.marks = append(p.marks, s.Start)
		}
		if s.Cut {
			p.ops[s.Start] = Cut
		}
		if s.Speed != 0 && s.Speed != 1.0 {
			p.speeds[s.Start] = s.Speed
		}
	}
	sort.Float64s(p.marks)
	return nil
}
