package video

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/catalan-adobe/vroom/internal/model"
)

// logPath returns the path of the export log file next to the output video.
func logPath(outputPath string) string {
	ext := filepath.Ext(outputPath)
	return strings.TrimSuffix(outputPath, ext) + ".log"
}

// Export builds an ffmpeg filter graph from segments and writes
// the output to outputPath. Cut segments are omitted; kept segments
// have their speed multiplier applied via setpts.
//
// A log file is always written alongside the output (same name, .log
// extension) containing the full ffmpeg command and its output.
// Inspect it when an export produces an empty or corrupt file.
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
		"-v", "verbose", // full output so the log is useful
		"-i", inputPath,
		"-filter_complex", strings.Join(filters, ";"),
		"-map", "[vout]",
		"-y", outputPath,
	}

	cmd := exec.Command("ffmpeg", args...)
	out, err := cmd.CombinedOutput()

	// Always write the log — useful even on success to diagnose empty outputs.
	log := fmt.Sprintf("vroom export — %s\n\ncommand:\nffmpeg %s\n\noutput:\n%s\n",
		time.Now().Format(time.RFC3339),
		strings.Join(args, " "),
		string(out),
	)
	_ = os.WriteFile(logPath(outputPath), []byte(log), 0o644)

	if err != nil {
		return fmt.Errorf("ffmpeg failed (see %s): %w", logPath(outputPath), err)
	}

	// ffmpeg can exit 0 but produce an empty file (e.g. all segments had
	// near-zero duration). Catch that explicitly.
	info, statErr := os.Stat(outputPath)
	if statErr != nil || info.Size() == 0 {
		return fmt.Errorf("export produced an empty file (see %s)", logPath(outputPath))
	}

	return nil
}
