package video

import (
	"bytes"
	"fmt"
	"os/exec"
)

// ExtractFrame extracts one frame at time t (seconds) from path,
// scaled to targetW×targetH pixels, and returns it as PNG bytes.
// Uses ffmpeg via shell-out.
func ExtractFrame(path string, t float64, targetW, targetH int) ([]byte, error) {
	scale := fmt.Sprintf("scale=%d:%d", targetW, targetH)
	var buf bytes.Buffer
	cmd := exec.Command(
		"ffmpeg", "-v", "quiet",
		"-ss", fmt.Sprintf("%.3f", t),
		"-i", path,
		"-vframes", "1",
		"-vf", scale,
		"-f", "image2",
		"-vcodec", "png",
		"pipe:1",
	)
	cmd.Stdout = &buf
	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("extract frame at %.3fs: %w", t, err)
	}
	return buf.Bytes(), nil
}
