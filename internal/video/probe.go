// Package video wraps ffmpeg/ffprobe for frame extraction and export.
package video

import (
	"encoding/json"
	"fmt"
	"os/exec"
	"strconv"
	"strings"
)

// Info holds metadata about a video file.
type Info struct {
	Duration float64
	FPS      float64
	Width    int
	Height   int
}

type ffprobeOut struct {
	Streams []ffprobeStream `json:"streams"`
	Format  ffprobeFormat   `json:"format"`
}

type ffprobeStream struct {
	CodecType  string `json:"codec_type"`
	Width      int    `json:"width"`
	Height     int    `json:"height"`
	RFrameRate string `json:"r_frame_rate"`
	Duration   string `json:"duration"`
}

type ffprobeFormat struct {
	Duration string `json:"duration"`
}

// Probe uses ffprobe to read video metadata from path.
func Probe(path string) (Info, error) {
	out, err := exec.Command(
		"ffprobe", "-v", "quiet",
		"-print_format", "json",
		"-show_streams", "-show_format",
		path,
	).Output()
	if err != nil {
		return Info{}, fmt.Errorf("ffprobe: %w", err)
	}

	var probe ffprobeOut
	if err := json.Unmarshal(out, &probe); err != nil {
		return Info{}, fmt.Errorf("parse ffprobe output: %w", err)
	}

	var info Info
	if probe.Format.Duration != "" {
		info.Duration, _ = strconv.ParseFloat(probe.Format.Duration, 64)
	}

	for _, s := range probe.Streams {
		if s.CodecType != "video" {
			continue
		}
		info.Width = s.Width
		info.Height = s.Height
		if info.Duration == 0 && s.Duration != "" {
			info.Duration, _ = strconv.ParseFloat(s.Duration, 64)
		}
		info.FPS = parseRatio(s.RFrameRate)
		break
	}

	if info.FPS == 0 {
		info.FPS = 30
	}
	return info, nil
}

// parseRatio parses "num/den" fraction strings (e.g. "24000/1001").
func parseRatio(s string) float64 {
	i := strings.IndexByte(s, '/')
	if i < 0 {
		v, _ := strconv.ParseFloat(s, 64)
		return v
	}
	num, _ := strconv.ParseFloat(s[:i], 64)
	den, _ := strconv.ParseFloat(s[i+1:], 64)
	if den == 0 {
		return 0
	}
	return num / den
}
