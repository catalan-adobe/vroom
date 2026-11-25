use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::{
    model::Project,
    paths::{export_path, save_path},
};

const STATUS_TIME: Duration = Duration::from_secs(3);
const MAX_PLAYBACK_ELAPSED: Duration = Duration::from_secs(2);

#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Preview,
    Export,
    Quit,
}

pub struct App {
    pub project: Project,
    pub cursor: f64,
    pub active_segment: usize,
    pub frame_mode: bool,
    pub steps: Vec<f64>,
    pub step_index: usize,
    last_frame_at: Option<Instant>,
    status: Option<Status>,
    exporting: bool,
}

struct Status {
    text: String,
    expires: Option<Instant>,
}

impl App {
    pub fn new(project: Project) -> Self {
        let steps = seek_steps(project.info.duration, project.info.fps);
        Self {
            project,
            cursor: 0.0,
            active_segment: 0,
            frame_mode: false,
            steps,
            step_index: 1,
            last_frame_at: None,
            status: None,
            exporting: false,
        }
    }

    pub fn key(&mut self, key: KeyEvent, now: Instant) -> Effect {
        use Effect::{Export, None as NoEffect, Preview, Quit};

        if key.kind == KeyEventKind::Release {
            return NoEffect;
        }
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            return Quit;
        }
        if !(key.modifiers - KeyModifiers::SHIFT).is_empty() {
            return NoEffect;
        }
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char('q') if !shift => Quit,
            KeyCode::Left | KeyCode::Right if !shift => {
                let delta = self.steps[self.step_index];
                self.cursor = if key.code == KeyCode::Left {
                    self.cursor - delta
                } else {
                    self.cursor + delta
                }
                .clamp(0.0, self.project.info.duration);
                self.select_at_cursor();
                Preview
            }
            KeyCode::Up if !shift => {
                self.step_index = (self.step_index + 1).min(self.steps.len() - 1);
                NoEffect
            }
            KeyCode::Down if !shift => {
                self.step_index = self.step_index.saturating_sub(1);
                NoEffect
            }
            KeyCode::Char('m' | 'M') => {
                if key.code == KeyCode::Char('M') || shift {
                    self.project.remove_mark(self.cursor);
                } else {
                    self.project.add_mark(self.cursor);
                }
                self.select_at_cursor();
                Preview
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let previous = shift || key.code == KeyCode::BackTab;
                let next = if previous {
                    self.active_segment.checked_sub(1)
                } else {
                    Some(self.active_segment + 1)
                };
                if let Some((index, segment)) =
                    next.and_then(|index| self.project.segment(index).map(|s| (index, s)))
                {
                    self.active_segment = index;
                    self.cursor = segment.start;
                    Preview
                } else {
                    NoEffect
                }
            }
            KeyCode::Char('c') if !shift => {
                self.project.toggle_cut(self.active_segment);
                NoEffect
            }
            KeyCode::Char('+' | '=') => {
                self.project.step_speed(self.active_segment, 2.0);
                NoEffect
            }
            KeyCode::Char('-') if !shift => {
                self.project.step_speed(self.active_segment, 0.5);
                NoEffect
            }
            KeyCode::Char('f') if !shift => {
                self.frame_mode = !self.frame_mode;
                NoEffect
            }
            KeyCode::Char(' ') if !shift => {
                if self.last_frame_at.take().is_none() && self.cursor < self.project.info.duration {
                    self.last_frame_at = Some(now);
                    Preview
                } else {
                    NoEffect
                }
            }
            KeyCode::Char('s') if !shift => {
                let path = save_path(&self.project.source);
                let text = match self.project.save(&path) {
                    Ok(()) => format!(
                        "Saved → {}",
                        path.file_name()
                            .unwrap_or(path.as_os_str())
                            .to_string_lossy()
                    ),
                    Err(error) => format!("Save failed: {error:#}"),
                };
                self.set_status(text, Some(now + STATUS_TIME));
                NoEffect
            }
            KeyCode::Char('e') if !shift && !self.exporting => {
                self.exporting = true;
                self.set_status("Exporting…".into(), None);
                Export
            }
            _ => NoEffect,
        }
    }

    pub fn frame_ready(&mut self, now: Instant) -> bool {
        if let Some(last) = self.last_frame_at {
            self.last_frame_at = Some(now);
            self.advance_cursor(
                now.saturating_duration_since(last)
                    .min(MAX_PLAYBACK_ELAPSED),
            );
        }
        self.last_frame_at.is_some()
    }

    pub fn frame_failed(&mut self) {
        // An empty PNG or a failed seek (e.g. at EOF) must not leave playback
        // active without a frame in flight.
        self.last_frame_at = None;
    }

    fn advance_cursor(&mut self, elapsed: Duration) {
        // Use wall time rather than polling at the source FPS: FFmpeg seeks
        // can take longer than a frame. Use the speed at the start of the tick.
        if let Some((_, segment)) = self.project.segment_at(self.cursor) {
            if segment.cut {
                self.cursor = segment.end;
            } else {
                self.cursor += elapsed.as_secs_f64() * segment.speed;
            }
            while let Some((_, segment)) =
                self.project.segment_at(self.cursor).filter(|(_, s)| s.cut)
            {
                self.cursor = segment.end;
            }
        } else {
            self.cursor = self.project.info.duration;
        }

        if self.cursor >= self.project.info.duration {
            self.cursor = self.project.info.duration;
            self.last_frame_at = None;
        }
        self.select_at_cursor();
    }

    fn select_at_cursor(&mut self) {
        self.active_segment = self
            .project
            .segment_at(self.cursor)
            .map_or_else(|| self.project.segment_count() - 1, |(index, _)| index);
    }

    pub fn export_finished(&mut self, result: anyhow::Result<()>, now: Instant) {
        self.exporting = false;
        let text = match result {
            Ok(()) => format!("Exported → {}", export_path(&self.project.source).display()),
            Err(error) => format!("Export failed: {error:#}"),
        };
        self.set_status(text, Some(now + STATUS_TIME));
    }

    fn set_status(&mut self, text: String, expires: Option<Instant>) {
        self.status = Some(Status { text, expires });
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_ref().map(|status| status.text.as_str())
    }

    pub fn status_deadline(&self) -> Option<Instant> {
        self.status.as_ref().and_then(|status| status.expires)
    }

    pub fn clear_status(&mut self) {
        self.status = None;
    }
}

fn seek_steps(duration: f64, fps: f64) -> Vec<f64> {
    let mut steps = vec![1.0 / fps, 1.0];
    steps.extend_from_slice(if duration < 600.0 {
        &[5.0, 30.0]
    } else if duration < 3600.0 {
        &[10.0, 60.0, 300.0]
    } else {
        &[30.0, 300.0, 1200.0]
    });
    steps
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::video::Info;

    pub(in crate::tui) fn app() -> App {
        App::new(Project::new(
            "demo.mp4".into(),
            Info {
                duration: 12.0,
                fps: 25.0,
                width: 1920,
                height: 1080,
            },
        ))
    }

    fn key(app: &mut App, code: KeyCode) -> Effect {
        app.key(KeyEvent::new(code, KeyModifiers::NONE), Instant::now())
    }

    #[test]
    fn step_ladders_have_the_same_anchors_and_thresholds() {
        assert_eq!(seek_steps(599.0, 25.0), [0.04, 1.0, 5.0, 30.0]);
        assert_eq!(seek_steps(600.0, 25.0), [0.04, 1.0, 10.0, 60.0, 300.0]);
        assert_eq!(seek_steps(3600.0, 25.0), [0.04, 1.0, 30.0, 300.0, 1200.0]);
        let mut app = app();
        key(&mut app, KeyCode::Right);
        assert_eq!(app.cursor, 1.0);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Right);
        assert_eq!(app.cursor, 1.04);
        for _ in 0..8 {
            key(&mut app, KeyCode::Up);
        }
        key(&mut app, KeyCode::Right);
        assert_eq!(app.cursor, 12.0);
        key(&mut app, KeyCode::Left);
        assert_eq!(app.cursor, 0.0);
    }

    #[test]
    fn editing_and_segment_navigation_stay_on_source_time() {
        let mut app = app();
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Char('m'));
        assert_eq!(app.active_segment, 1);
        key(&mut app, KeyCode::Char('c'));
        key(&mut app, KeyCode::Char('='));
        assert!(app.project.segment(1).unwrap().cut);
        assert_eq!(app.project.segment(1).unwrap().speed, 2.0);
        assert_eq!(key(&mut app, KeyCode::Tab), Effect::None);
        assert_eq!(key(&mut app, KeyCode::BackTab), Effect::Preview);
        assert_eq!(app.cursor, 0.0);
        assert_eq!(key(&mut app, KeyCode::BackTab), Effect::None);
        assert_eq!(key(&mut app, KeyCode::Tab), Effect::Preview);
        assert_eq!(app.cursor, 1.0);
        key(&mut app, KeyCode::Char('M'));
        assert!(app.project.marks().is_empty());
        key(&mut app, KeyCode::Char('f'));
        assert!(app.frame_mode);
    }

    #[test]
    fn playback_uses_elapsed_wall_time_and_skips_consecutive_cuts() {
        let mut app = app();
        for mark in [2.0, 4.0, 6.0, 10.0] {
            app.project.add_mark(mark);
        }
        app.project.toggle_cut(1);
        app.project.toggle_cut(2);
        app.project.step_speed(3, 2.0);
        let now = Instant::now();
        app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), now);
        assert!(app.frame_ready(now + Duration::from_secs(5)));
        assert_eq!(app.cursor, 6.0); // two-second cap, then consecutive cuts
        assert!(app.frame_ready(now + Duration::from_millis(5750)));
        assert_eq!(app.cursor, 7.5);
        assert!(app.frame_ready(now + Duration::from_secs(8)));
        assert_eq!(app.cursor, 11.5); // same speed through this tick
        assert!(!app.frame_ready(now + Duration::from_secs(9)));
        assert_eq!(app.cursor, 12.0);
        assert_eq!(app.active_segment, 4);
    }

    #[test]
    fn a_failed_frame_stops_playback_and_export_is_single_flight() {
        let mut app = app();
        assert_eq!(key(&mut app, KeyCode::Char(' ')), Effect::Preview);
        app.frame_failed();
        assert_eq!(key(&mut app, KeyCode::Char(' ')), Effect::Preview);
        assert_eq!(key(&mut app, KeyCode::Char('e')), Effect::Export);
        assert_eq!(key(&mut app, KeyCode::Char('e')), Effect::None);
        app.export_finished(Ok(()), Instant::now());
        assert_eq!(app.status(), Some("Exported → demo_vroom.mp4"));
        assert!(app.status_deadline().is_some());
        assert_eq!(key(&mut app, KeyCode::Char('e')), Effect::Export);
    }
}
