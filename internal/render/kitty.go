// Package render handles Kitty graphics protocol encoding for video frames.
// Protocol spec: https://sw.kovidgoyal.net/kitty/graphics-protocol/
package render

import (
	"encoding/base64"
	"fmt"
	"os"
	"strings"
)

// imgID is the stable Kitty image ID used for the video preview.
// Sending a new frame with the same ID replaces the previous one.
const imgID = 42

// Supported reports whether the current terminal supports the
// Kitty graphics protocol.
func Supported() bool {
	return os.Getenv("KITTY_WINDOW_ID") != "" ||
		os.Getenv("TERM") == "xterm-kitty" ||
		os.Getenv("TERM_PROGRAM") == "ghostty" ||
		os.Getenv("TERM_PROGRAM") == "WezTerm"
}

// DeleteAll returns the escape sequence that deletes all Kitty images.
func DeleteAll() string {
	return "\x1b_Ga=d\x1b\\"
}

// Frame encodes PNG bytes as a Kitty protocol transmission
// that displays the image in cols×rows terminal cells at
// the given 1-based terminal row and col.
//
// The image is placed at z=-1 (above the cell background, below text glyphs).
// Terminal cells with no explicit background colour are transparent to the
// Kitty layer, so the view's empty preview area shows the frame through.
func Frame(png []byte, cols, rows, atRow, atCol int) string {
	if len(png) == 0 || cols < 1 || rows < 1 {
		return ""
	}
	b64 := base64.StdEncoding.EncodeToString(png)
	cursor := fmt.Sprintf("\x1b[%d;%dH", atRow, atCol)
	return cursor + chunked(b64, cols, rows)
}

// chunked splits b64 into ≤4096-byte APC chunks per the Kitty
// graphics protocol spec. The first chunk carries display parameters;
// subsequent chunks carry only the continuation flag.
func chunked(b64 string, cols, rows int) string {
	const chunkSize = 4096
	var sb strings.Builder
	for i := 0; i < len(b64); i += chunkSize {
		end := i + chunkSize
		if end > len(b64) {
			end = len(b64)
		}
		chunk := b64[i:end]
		more := 1
		if end >= len(b64) {
			more = 0
		}
		if i == 0 {
			// f=100 PNG, a=T transmit+display, c/r = cell dimensions, z=-1
			fmt.Fprintf(&sb,
				"\x1b_Gf=100,a=T,i=%d,c=%d,r=%d,z=-1,m=%d;%s\x1b\\",
				imgID, cols, rows, more, chunk,
			)
		} else {
			fmt.Fprintf(&sb, "\x1b_Gm=%d;%s\x1b\\", more, chunk)
		}
	}
	return sb.String()
}
