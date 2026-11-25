use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Size,
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::app::App;
use crate::{model::Segment, video::Info};

const FIXED_ROWS: usize = 9;
const MAX_PREVIEW_ROWS: usize = 40;
const ACCENT: Style = Style::new()
    .fg(Color::Rgb(0, 229, 160))
    .add_modifier(Modifier::BOLD);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
const DIM: Style = Style::new().fg(Color::Rgb(68, 68, 68));
const MUTED: Style = Style::new().fg(Color::Rgb(119, 119, 119));
const KEEP: Style = Style::new()
    .fg(Color::Rgb(0, 204, 119))
    .add_modifier(Modifier::BOLD);
const CUT: Style = Style::new()
    .fg(Color::Rgb(255, 68, 85))
    .add_modifier(Modifier::BOLD);
const PREVIEW_HINT: &str = "[ Kitty / WezTerm / Ghostty required for video preview ]";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    width: usize,
    height: usize,
    top: usize,
    left: usize,
    pub rows: usize,
    pub cols: usize,
}

impl Layout {
    pub fn new(size: Size, video: Info) -> Self {
        let width = usize::from(size.width);
        let height = usize::from(size.height);
        // A cell is approximately twice as tall as it is wide. Predict the
        // rectangle in cells, but only tell FFmpeg the height; it keeps the AR.
        let aspect = if video.width > 0 && video.height > 0 {
            f64::from(video.width) / f64::from(video.height)
        } else {
            16.0 / 9.0
        };
        let max_cols = width.saturating_sub(2).max(4);
        let available_rows = height.saturating_sub(FIXED_ROWS).max(3);
        let rows = ((max_cols as f64 * 0.5 / aspect) as usize)
            .clamp(3, MAX_PREVIEW_ROWS)
            .min(available_rows);
        let cols = ((rows as f64 * aspect / 0.5) as usize).clamp(4, max_cols);
        let top = height.saturating_sub(rows + FIXED_ROWS) / 2;
        let left = width.saturating_sub(cols + 2) / 2;
        Self {
            width,
            height,
            top,
            left,
            rows,
            cols,
        }
    }

    pub fn frame_pixel_height(self) -> usize {
        (self.rows * 16).min(720)
    }

    // Kitty expects 1-based coordinates, and the preview must fit entirely
    // within the viewport so a tiny resize cannot scroll the alternate screen.
    pub fn placement(self) -> Option<(usize, usize)> {
        (self.cols + 2 <= self.width && self.top + self.rows + 2 < self.height)
            .then_some((self.top + 3, self.left + 2))
    }
}

pub fn draw(frame: &mut Frame<'_>, app: &App, kitty: bool) -> Layout {
    let layout = Layout::new(frame.area().as_size(), app.project.info);
    render(frame.buffer_mut(), layout, app, kitty);
    layout
}

fn render(buffer: &mut Buffer, layout: Layout, app: &App, kitty: bool) {
    let Layout {
        width,
        height,
        top,
        left,
        rows,
        cols,
    } = layout;
    let mut canvas = Canvas {
        buffer,
        width,
        height,
    };
    if width == 0 || height == 0 {
        return;
    }

    let project = &app.project;
    let pos = |time| format_position(time, project.info.fps, app.frame_mode);
    let filename = project
        .source
        .file_name()
        .unwrap_or(project.source.as_os_str())
        .to_string_lossy();
    let title = [
        Span::styled("vroom", ACCENT),
        Span::raw("  "),
        Span::styled(filename, BOLD),
    ];
    let position = Span::styled(
        format!(
            "[{} / {}]",
            pos(project.output_position(app.cursor)),
            pos(project.output_duration())
        ),
        MUTED,
    );
    let title_width = title.iter().map(Span::width).sum::<usize>();
    let x = (title_width + 1).max(width.saturating_sub(position.width()));
    canvas.line(top, Line::from(title.to_vec()));
    canvas.span(x, top, position);

    let top_border = top + 1;
    canvas.span(
        left,
        top_border,
        Span::styled(format!("╭{}╮", "─".repeat(cols)), MUTED),
    );
    for row in top_border + 1..=top_border + rows {
        canvas.span(left, row, Span::styled("│", MUTED));
        canvas.span(left + cols + 1, row, Span::styled("│", MUTED));
    }
    if !kitty {
        let hint = &PREVIEW_HINT[..cols.min(PREVIEW_HINT.len())];
        canvas.span(
            left + 1 + (cols - hint.len()) / 2,
            top + 2 + rows / 2,
            Span::styled(hint, MUTED),
        );
    }
    let bottom_border = top + 2 + rows;
    canvas.span(
        left,
        bottom_border,
        Span::styled(format!("╰{}╯", "─".repeat(cols)), MUTED),
    );

    let timeline = bottom_border + 1;
    draw_timeline(&mut canvas, timeline, app);
    // The original blank timeline occupies one extra line at tiny widths or
    // when the input has no reported duration.
    let separator = timeline
        + if width < 10 || project.info.duration == 0.0 {
            4
        } else {
            3
        };
    canvas.span(0, separator, Span::styled("─".repeat(width), DIM));

    if let Some(segment) = project.segment(app.active_segment) {
        canvas.line(
            separator + 1,
            Line::from(vec![
                Span::raw(format!(
                    "  Seg {}/{}  {}–{}  ",
                    app.active_segment + 1,
                    project.segment_count(),
                    pos(segment.start),
                    pos(segment.end)
                )),
                if segment.cut {
                    Span::styled("CUT", CUT)
                } else {
                    Span::styled("KEEP", KEEP)
                },
                Span::raw(format!(
                    "  speed: ×{:.2}  dur: {}",
                    segment.speed,
                    pos(segment.output_duration())
                )),
            ]),
        );
    }

    let mut hints = vec![Span::raw("  ")];
    if let Some(status) = app.status() {
        hints.push(Span::styled(status, ACCENT));
    } else {
        for (i, step) in app.steps.iter().enumerate() {
            if i > 0 {
                hints.push(Span::styled("·", DIM));
            }
            let step = format_step(*step);
            hints.push(if i == app.step_index {
                Span::styled(format!("[{step}]"), ACCENT)
            } else {
                Span::styled(step, DIM)
            });
        }
        if app.frame_mode {
            hints.extend([
                Span::raw("  "),
                Span::styled("[f]", ACCENT),
                Span::styled(" frames", DIM),
            ]);
        } else {
            hints.push(Span::styled("  [f] time", DIM));
        }
        hints.push(Span::styled("  ↑↓:step  ←→:seek  m:mark  M:del  tab:seg  c:cut  +−:speed  space:play  e:export  q:quit", DIM));
    }
    canvas.line(separator + 2, Line::from(hints));
}

struct Canvas<'a> {
    buffer: &'a mut Buffer,
    width: usize,
    height: usize,
}

impl Canvas<'_> {
    fn line(&mut self, row: usize, line: Line<'_>) {
        if row < self.height {
            self.buffer
                .set_line(0, row as u16, &line, self.width as u16);
        }
    }

    fn span(&mut self, col: usize, row: usize, span: Span<'_>) {
        if col < self.width && row < self.height {
            self.buffer
                .set_span(col as u16, row as u16, &span, (self.width - col) as u16);
        }
    }
}

fn draw_timeline(canvas: &mut Canvas<'_>, row: usize, app: &App) {
    let width = canvas.width;
    let info = app.project.info;
    if info.duration == 0.0 || width < 10 {
        return;
    }
    let to_col = |time: f64| ((time / info.duration * (width - 1) as f64) as usize).min(width - 1);

    let mut ruler = vec!['─'; width];
    for i in 0..=5 {
        let time = info.duration * f64::from(i) / 5.0;
        let label = if app.frame_mode {
            format_frame_compact(time, info.fps)
        } else {
            format_time(time)
        };
        for (cell, ch) in ruler[to_col(time)..].iter_mut().zip(label.chars()) {
            *cell = ch;
        }
    }
    canvas.span(0, row, Span::raw(ruler.into_iter().collect::<String>()));

    let mut marks = vec!['─'; width];
    for &mark in app.project.marks() {
        marks[to_col(mark)] = '│';
    }
    marks[to_col(app.cursor)] = '▶';
    canvas.span(0, row + 1, Span::raw(marks.into_iter().collect::<String>()));

    for (i, segment) in app.project.segments().enumerate() {
        let start = to_col(segment.start);
        let end = if i + 1 == app.project.segment_count() {
            width
        } else {
            to_col(segment.end)
        };
        let count = end.saturating_sub(start);
        if count == 0 {
            continue;
        }
        let label = if segment.cut {
            " CUT ".into()
        } else if segment.speed == 1.0 {
            " KEEP ".into()
        } else {
            format!(" ×{:.2} ", segment.speed)
        };
        let mut style = segment_style(segment);
        if i == app.active_segment {
            style = style.add_modifier(Modifier::BOLD);
        }
        let block: String = label
            .chars()
            .chain(std::iter::repeat(' '))
            .take(count)
            .collect();
        canvas.span(start, row + 2, Span::styled(block, style));
    }

    // Segments shorter than a cell share the following segment's column.
    // Show the selected one there without shifting every subsequent block.
    if let Some(segment) = app.project.segment(app.active_segment) {
        let x = to_col(segment.start);
        if app.active_segment + 1 < app.project.segment_count() && x == to_col(segment.end) {
            let style = segment_style(segment).add_modifier(Modifier::BOLD);
            canvas.span(x, row + 2, Span::styled(" ", style));
        }
    }
}

fn segment_style(segment: Segment) -> Style {
    if segment.cut {
        Style::new()
            .fg(Color::Rgb(255, 255, 255))
            .bg(Color::Rgb(136, 34, 51))
    } else {
        Style::new()
            .fg(Color::Rgb(17, 17, 17))
            .bg(speed_color(segment.speed))
    }
}

fn format_position(time: f64, fps: f64, frames: bool) -> String {
    if frames {
        ((time * fps) as i64).to_string()
    } else {
        format_time(time)
    }
}

fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    let whole = seconds as i64;
    let decisecond = ((seconds - whole as f64) * 10.0) as i64;
    format!("{}:{:02}.{decisecond}", whole / 60, whole % 60)
}

fn format_frame_compact(time: f64, fps: f64) -> String {
    let frames = (time * fps) as i64;
    match frames {
        ..1000 => frames.to_string(),
        n if n % 1000 == 0 => format!("{}K", n / 1000),
        n => format!("{:.1}K", n as f64 / 1000.0),
    }
}

fn format_step(time: f64) -> String {
    let seconds = time as u64;
    if time < 1.0 {
        "1f".into()
    } else if time < 60.0 {
        if time == seconds as f64 {
            format!("{seconds}s")
        } else {
            format!("{time:.1}s")
        }
    } else if seconds.is_multiple_of(60) {
        format!("{}m", seconds / 60)
    } else {
        format!("{}m{}s", seconds / 60, seconds % 60)
    }
}

fn speed_color(speed: f64) -> Color {
    if speed <= 0.0 {
        return Color::Rgb(255, 102, 0);
    }
    let log = speed.log2();
    let target = if log < 0.0 {
        (255.0, 102.0, 0.0)
    } else {
        (0.0, 238.0, 85.0)
    };
    let t = (log.abs() / 3.0).min(1.0).powf(0.7);
    let mix = |end: f64| (170.0 + t * (end - 170.0)).round() as u8;
    Color::Rgb(mix(target.0), mix(target.1), mix(target.2))
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;
    use crate::tui::app::tests::app;

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn preview_dimensions_match_video_and_terminal_aspect() {
        let info = app().project.info;
        let layout = Layout::new(Size::new(80, 41), info);
        assert_eq!(
            (layout.rows, layout.cols, layout.top, layout.left),
            (21, 74, 5, 2)
        );
        assert_eq!(layout.placement(), Some((8, 4)));
        assert_eq!(layout.frame_pixel_height(), 336);
        let portrait = Layout::new(
            Size::new(200, 100),
            Info {
                width: 500,
                height: 1000,
                ..info
            },
        );
        assert_eq!((portrait.rows, portrait.cols), (40, 40));
        assert_eq!(Layout::new(Size::new(4, 4), info).placement(), None);
    }

    #[test]
    fn header_uses_output_time_but_timeline_and_segment_bounds_use_source() {
        let mut app = app();
        for mark in [2.0, 4.0, 6.0, 8.0, 10.0] {
            app.project.add_mark(mark);
        }
        for index in [1, 2, 5] {
            app.project.toggle_cut(index);
        }
        app.project.step_speed(3, 0.5);
        app.project.step_speed(4, 2.0);
        app.cursor = 7.0;
        app.active_segment = 3;
        app.frame_mode = true;
        let size = Rect::new(0, 0, 80, 41);
        let layout = Layout::new(size.as_size(), app.project.info);
        let mut buffer = Buffer::empty(size);
        render(&mut buffer, layout, &app, false);

        assert!(row(&buffer, 5).ends_with("[100 / 175]"));
        assert!(row(&buffer, 17).contains(PREVIEW_HINT));
        assert!(row(&buffer, 29).starts_with('0'));
        assert_eq!(buffer[(46, 30)].symbol(), "▶");
        assert!(row(&buffer, 31).contains(" CUT "));
        assert_eq!(buffer[(13, 31)].bg, Color::Rgb(136, 34, 51));
        assert!(buffer[(39, 31)].modifier.contains(Modifier::BOLD));
        assert!(row(&buffer, 33).starts_with("  Seg 4/6  150–200  KEEP  speed: ×0.50  dur: 100"));
        assert!(row(&buffer, 34).starts_with("  1f·[1s]·5s·30s  [f] frames"));
    }

    #[test]
    fn narrow_rows_clip_and_preview_background_is_transparent() {
        let mut app = app();
        app.project.source = "映画-demo.mp4".into();
        for i in 1..60 {
            app.project.add_mark(f64::from(i) / 5.0);
        }
        for size in [
            Rect::new(0, 0, 1, 2),
            Rect::new(0, 0, 9, 13),
            Rect::new(0, 0, 16, 14),
        ] {
            render(
                &mut Buffer::empty(size),
                Layout::new(size.as_size(), app.project.info),
                &app,
                false,
            );
        }
        let size = Rect::new(0, 0, 120, 30);
        let layout = Layout::new(size.as_size(), app.project.info);
        let mut buffer = Buffer::empty(size);
        render(&mut buffer, layout, &app, true);
        let (y, x) = layout.placement().unwrap();
        let cell = &buffer[((x - 1) as u16, (y - 1) as u16)];
        assert_eq!(cell.symbol(), " ");
        assert_eq!(cell.bg, Color::Reset);
    }

    #[test]
    fn formats_truncate_and_color_gradient_saturates() {
        assert_eq!(format_time(61.25), "1:01.2");
        assert_eq!(format_frame_compact(1999.0, 1.0), "2.0K");
        assert_eq!(format_frame_compact(2000.0, 1.0), "2K");
        assert_eq!(format_position(72.0, 25.0, true), "1800");
        assert_eq!(format_step(300.0), "5m");
        assert_eq!(speed_color(1.0), Color::Rgb(170, 170, 170));
        assert_eq!(speed_color(0.125), Color::Rgb(255, 102, 0));
        assert_eq!(speed_color(64.0), Color::Rgb(0, 238, 85));
    }
}
