package tui

import "charm.land/bubbles/v2/key"

type keyMap struct {
	SeekLeft     key.Binding
	SeekRight    key.Binding
	SeekBigLeft  key.Binding
	SeekBigRight key.Binding
	AddMark      key.Binding
	DelMark      key.Binding
	NextSeg      key.Binding
	PrevSeg      key.Binding
	ToggleCut    key.Binding
	SpeedUp      key.Binding
	SpeedDown    key.Binding
	PlayPause    key.Binding
	Save         key.Binding
	Export       key.Binding
	Quit         key.Binding
}

func defaultKeyMap() keyMap {
	return keyMap{
		SeekLeft:     key.NewBinding(key.WithKeys("left"),    key.WithHelp("←", "seek -0.5s")),
		SeekRight:    key.NewBinding(key.WithKeys("right"),   key.WithHelp("→", "seek +0.5s")),
		SeekBigLeft:  key.NewBinding(key.WithKeys("h"),       key.WithHelp("h", "seek -5s")),
		SeekBigRight: key.NewBinding(key.WithKeys("l"),       key.WithHelp("l", "seek +5s")),
		AddMark:      key.NewBinding(key.WithKeys("m"),       key.WithHelp("m", "add mark")),
		DelMark:      key.NewBinding(key.WithKeys("M"),       key.WithHelp("M", "del mark")),
		NextSeg:      key.NewBinding(key.WithKeys("tab"),     key.WithHelp("tab", "next seg")),
		PrevSeg:      key.NewBinding(key.WithKeys("backtab"), key.WithHelp("⇧tab", "prev seg")),
		ToggleCut:    key.NewBinding(key.WithKeys("c"),       key.WithHelp("c", "cut/keep")),
		SpeedUp:      key.NewBinding(key.WithKeys("+", "="),  key.WithHelp("+", "faster")),
		SpeedDown:    key.NewBinding(key.WithKeys("-"),        key.WithHelp("-", "slower")),
		PlayPause:    key.NewBinding(key.WithKeys("space"),    key.WithHelp("space", "play/pause")),
		Save:         key.NewBinding(key.WithKeys("s"),         key.WithHelp("s", "save")),
		Export:       key.NewBinding(key.WithKeys("e"),         key.WithHelp("e", "export")),
		Quit:         key.NewBinding(key.WithKeys("q", "ctrl+c"), key.WithHelp("q", "quit")),
	}
}
