// Package model holds the editing data model: marks and segments.
package model

import "sort"

// Op is the operation applied to a segment.
type Op int

const (
	Keep Op = iota // keep this segment in the output
	Cut            // remove this segment from the output
)

// Segment is a slice of video between two consecutive marks
// (or between the start/end of the file and a mark).
type Segment struct {
	Start float64 // seconds
	End   float64 // seconds
	Op    Op
	Speed float64 // 1.0 = normal, 0.5 = half speed, 2.0 = double
}

// Project holds the full editing state for a single video.
type Project struct {
	VideoPath string
	Duration  float64 // seconds
	FPS       float64
	PixelW    int
	PixelH    int

	marks  []float64           // sorted timestamps
	ops    map[float64]Op      // segment start time → op
	speeds map[float64]float64 // segment start time → speed multiplier
}

// NewProject creates an empty project for the given video.
func NewProject(path string, dur, fps float64, w, h int) *Project {
	return &Project{
		VideoPath: path,
		Duration:  dur,
		FPS:       fps,
		PixelW:    w,
		PixelH:    h,
		ops:       make(map[float64]Op),
		speeds:    make(map[float64]float64),
	}
}

// AddMark inserts a mark at time t.
// Marks within 0.1s of an existing mark or the file boundaries are ignored.
func (p *Project) AddMark(t float64) {
	const snap = 0.1
	if t <= snap || t >= p.Duration-snap {
		return
	}
	for _, m := range p.marks {
		if abs64(m-t) < snap {
			return
		}
	}
	p.marks = append(p.marks, t)
	sort.Float64s(p.marks)
}

// RemoveMark removes the mark nearest to t (within 1s).
func (p *Project) RemoveMark(t float64) {
	nearest, minDist := -1, 1.0
	for i, m := range p.marks {
		if d := abs64(m - t); d < minDist {
			minDist, nearest = d, i
		}
	}
	if nearest >= 0 {
		p.marks = append(p.marks[:nearest], p.marks[nearest+1:]...)
	}
}

// Marks returns the sorted list of mark timestamps.
func (p *Project) Marks() []float64 {
	return p.marks
}

// Segments derives the ordered segment list from the current marks.
// Each segment's op and speed are keyed by the segment's start time.
func (p *Project) Segments() []Segment {
	bounds := make([]float64, 0, len(p.marks)+2)
	bounds = append(bounds, 0)
	bounds = append(bounds, p.marks...)
	bounds = append(bounds, p.Duration)

	segs := make([]Segment, len(bounds)-1)
	for i := range segs {
		start := bounds[i]
		speed := p.speeds[start]
		if speed == 0 {
			speed = 1.0
		}
		segs[i] = Segment{
			Start: start,
			End:   bounds[i+1],
			Op:    p.ops[start],
			Speed: speed,
		}
	}
	return segs
}

// ToggleCut toggles the Cut operation on segment at index segIdx.
func (p *Project) ToggleCut(segIdx int) {
	segs := p.Segments()
	if segIdx < 0 || segIdx >= len(segs) {
		return
	}
	start := segs[segIdx].Start
	if p.ops[start] == Cut {
		delete(p.ops, start)
	} else {
		p.ops[start] = Cut
	}
}

// AdjustSpeed changes the speed multiplier for segment at index segIdx
// by delta. Clamped to [0.25, 4.0].
func (p *Project) AdjustSpeed(segIdx int, delta float64) {
	segs := p.Segments()
	if segIdx < 0 || segIdx >= len(segs) {
		return
	}
	start := segs[segIdx].Start
	speed := p.speeds[start]
	if speed == 0 {
		speed = 1.0
	}
	speed += delta
	if speed < 0.25 {
		speed = 0.25
	}
	if speed > 4.0 {
		speed = 4.0
	}
	p.speeds[start] = speed
}

// OutputDuration returns the total duration of the edited output:
// CUT segments are excluded; speed multipliers compress or stretch time.
func (p *Project) OutputDuration() float64 {
	var total float64
	for _, s := range p.Segments() {
		if s.Op != Cut {
			total += (s.End - s.Start) / s.Speed
		}
	}
	return total
}

// OutputPosition returns the position in the edited output that
// corresponds to cursor t in the original video.
//
// Rule: iterate segments in order, accumulating output time.
//   - CUT past the cursor  → skip (contributes 0 to output)
//   - CUT containing cursor → stop; cursor is inside excised material
//   - KEEP/speed fully before cursor → add full output duration
//   - KEEP/speed containing cursor → add partial output duration; stop
func (p *Project) OutputPosition(t float64) float64 {
	var pos float64
	for _, s := range p.Segments() {
		if t <= s.Start {
			break // cursor hasn’t reached this segment yet
		}
		if s.Op == Cut {
			if t < s.End {
				break // cursor is inside a CUT; stop accumulating
			}
			continue // cursor is past this CUT; 0 output contribution
		}
		// KEEP or speed segment.
		if t < s.End {
			// Cursor is inside this segment — add partial then stop.
			pos += (t - s.Start) / s.Speed
			break
		}
		// Cursor is past this segment — add full output duration.
		pos += (s.End - s.Start) / s.Speed
	}
	return pos
}

func abs64(x float64) float64 {
	if x < 0 {
		return -x
	}
	return x
}
