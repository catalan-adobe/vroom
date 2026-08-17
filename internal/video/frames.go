package video

import (
	"bytes"
	"fmt"
	"os/exec"
)

// ExtractFrame extracts one frame at time t (seconds) from path,
// scaled to targetH pixels tall with aspect ratio preserved (-2 rounds
// the auto-computed width to an even number as required by most codecs).
// Uses ffmpeg via shell-out.
func ExtractFrame(path string, t float64, targetH int) ([]byte, error) {
	scale := fmt.Sprintf("scale=-2:%d", targetH)
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
