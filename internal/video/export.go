package video

import (
	"fmt"
	"os/exec"
	"strings"

	"github.com/catalan-adobe/vroom/internal/model"
)

// Export builds an ffmpeg filter graph from segments and writes
// the output to outputPath. Cut segments are omitted; kept segments
// have their speed multiplier applied via setpts.
func Export(inputPath string, segments []model.Segment, outputPath string) error {
	var kept []model.Segment
	for _, s := range segments {
		if s.Op != model.Cut {
			kept = append(kept, s)
		}
	}
	if len(kept) == 0 {
		return fmt.Errorf("nothing to export: all segments are cut")
	}

	// Build one trim+setpts filter per kept segment.
	var filters []string
	var vLabels []string
	for i, s := range kept {
		// setpts multiplier is 1/speed: slower video = larger PTS values.
		pts := fmt.Sprintf("%.6f", 1.0/s.Speed)
		filters = append(filters, fmt.Sprintf(
			"[0:v]trim=start=%.6f:end=%.6f,setpts=(PTS-STARTPTS)*%s[v%d]",
			s.Start, s.End, pts, i,
		))
		vLabels = append(vLabels, fmt.Sprintf("[v%d]", i))
	}

	// Concatenate all kept segments.
	filters = append(filters, fmt.Sprintf(
		"%sconcat=n=%d:v=1:a=0[vout]",
		strings.Join(vLabels, ""), len(kept),
	))

	args := []string{
		"-v", "warning",
		"-i", inputPath,
		"-filter_complex", strings.Join(filters, ";"),
		"-map", "[vout]",
		"-y", outputPath,
	}

	out, err := exec.Command("ffmpeg", args...).CombinedOutput()
	if err != nil {
		return fmt.Errorf("ffmpeg: %w\n%s", err, out)
	}
	return nil
}
