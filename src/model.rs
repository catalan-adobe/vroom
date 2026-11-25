use std::{collections::HashMap, path::PathBuf};

use crate::video::Info;

mod save;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub cut: bool,
    pub speed: f64,
}

impl Segment {
    pub fn output_duration(self) -> f64 {
        if self.cut {
            0.0
        } else {
            (self.end - self.start) / self.speed
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Edit {
    cut: bool,
    speed: f64,
}

impl Default for Edit {
    fn default() -> Self {
        Self {
            cut: false,
            speed: 1.0,
        }
    }
}

#[derive(Debug)]
pub struct Project {
    pub source: PathBuf,
    pub info: Info,
    marks: Vec<f64>,
    // Key by the exact start time, not by index. Removed marks leave their
    // edits here until reload, so reinserting the same start restores them.
    edits: HashMap<u64, Edit>,
}

impl Project {
    pub fn new(source: PathBuf, info: Info) -> Self {
        Self {
            source,
            info,
            marks: Vec::new(),
            edits: HashMap::new(),
        }
    }

    pub fn marks(&self) -> &[f64] {
        &self.marks
    }

    pub fn add_mark(&mut self, time: f64) {
        const SNAP: f64 = 0.1;
        if !time.is_finite() || time <= SNAP || time >= self.info.duration - SNAP {
            return;
        }
        let index = self.marks.partition_point(|&mark| mark < time);
        if index
            .checked_sub(1)
            .is_some_and(|i| time - self.marks[i] < SNAP)
            || self
                .marks
                .get(index)
                .is_some_and(|&next| next - time < SNAP)
        {
            return;
        }
        self.marks.insert(index, time);
    }

    pub fn remove_mark(&mut self, time: f64) {
        let mut nearest = None;
        let mut min_distance = 1.0;
        for (i, &mark) in self.marks.iter().enumerate() {
            let distance = (mark - time).abs();
            if distance < min_distance {
                nearest = Some(i);
                min_distance = distance;
            }
        }
        if let Some(i) = nearest {
            self.marks.remove(i);
        }
    }

    pub fn segment_count(&self) -> usize {
        self.marks.len() + 1
    }

    pub fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        std::iter::once(0.0)
            .chain(self.marks.iter().copied())
            .zip(
                self.marks
                    .iter()
                    .copied()
                    .chain(std::iter::once(self.info.duration)),
            )
            .map(|(start, end)| self.segment_between(start, end))
    }

    pub fn segment_at(&self, time: f64) -> Option<(usize, Segment)> {
        self.segments()
            .enumerate()
            .find(|(_, segment)| time >= segment.start && time < segment.end)
    }

    pub fn segment(&self, index: usize) -> Option<Segment> {
        let start = self.segment_start(index)?;
        let end = self.marks.get(index).copied().unwrap_or(self.info.duration);
        Some(self.segment_between(start, end))
    }

    fn segment_start(&self, index: usize) -> Option<f64> {
        match index {
            0 => Some(0.0),
            _ => self.marks.get(index - 1).copied(),
        }
    }

    fn segment_between(&self, start: f64, end: f64) -> Segment {
        let edit = self
            .edits
            .get(&start_key(start))
            .copied()
            .unwrap_or_default();
        Segment {
            start,
            end,
            cut: edit.cut,
            speed: edit.speed,
        }
    }

    pub fn toggle_cut(&mut self, index: usize) {
        if let Some(start) = self.segment_start(index) {
            let edit = self.edits.entry(start_key(start)).or_default();
            edit.cut = !edit.cut;
        }
    }

    pub fn step_speed(&mut self, index: usize, factor: f64) {
        if let Some(start) = self.segment_start(index) {
            let edit = self.edits.entry(start_key(start)).or_default();
            edit.speed = (edit.speed * factor).max(1.0 / 64.0);
        }
    }

    pub fn output_duration(&self) -> f64 {
        self.segments().map(Segment::output_duration).sum()
    }

    pub fn output_position(&self, time: f64) -> f64 {
        let mut position = 0.0;
        for segment in self.segments() {
            if time <= segment.start {
                break;
            }
            if time < segment.end {
                if !segment.cut {
                    position += (time - segment.start) / segment.speed;
                }
                break;
            }
            position += segment.output_duration();
        }
        position
    }
}

fn start_key(start: f64) -> u64 {
    // Float equality in the Go format treats both signed zeros as time zero.
    if start == 0.0 { 0 } else { start.to_bits() }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn project() -> Project {
        Project::new(
            "demo.mp4".into(),
            Info {
                duration: 12.0,
                fps: 25.0,
                width: 1920,
                height: 1080,
            },
        )
    }

    #[test]
    fn marks_are_sorted_and_obey_snap_distances() {
        let mut project = project();
        for mark in [4.0, 2.0, 0.1, 11.9, 4.05, 2.0, 3.0, f64::NAN] {
            project.add_mark(mark);
        }
        assert_eq!(project.marks(), &[2.0, 3.0, 4.0]);
        project.remove_mark(1.0); // exactly 1 s away
        project.remove_mark(2.5); // ties choose the earlier mark
        assert_eq!(project.marks(), &[3.0, 4.0]);
        assert_eq!(project.segment_at(4.0).unwrap().0, 2);
        assert!(project.segment_at(12.0).is_none());
    }

    #[test]
    fn edits_survive_index_changes_and_reinserting_the_same_mark() {
        let mut project = project();
        project.add_mark(4.0);
        project.toggle_cut(1);
        project.step_speed(1, 2.0);
        project.add_mark(2.0);
        assert!(!project.segment(1).unwrap().cut);
        assert_eq!(project.segment(2).unwrap().speed, 2.0);
        project.remove_mark(4.0);
        project.add_mark(4.0);
        assert!(project.segment(2).unwrap().cut);
        assert_eq!(project.segment(2).unwrap().speed, 2.0);
    }

    #[test]
    fn edited_time_excludes_cuts_and_scales_partial_segments() {
        let mut project = project();
        for mark in [2.0, 4.0, 6.0, 8.0, 10.0] {
            project.add_mark(mark);
        }
        for index in [1, 2, 5] {
            project.toggle_cut(index);
        }
        project.step_speed(3, 0.5);
        project.step_speed(4, 2.0);
        assert_eq!(project.output_duration(), 7.0);
        for (time, edited) in [
            (1.0, 1.0),
            (2.0, 2.0),
            (5.0, 2.0),
            (6.0, 2.0),
            (7.0, 4.0),
            (9.0, 6.5),
            (11.0, 7.0),
        ] {
            assert_eq!(project.output_position(time), edited);
        }
    }

    #[test]
    fn speed_uses_powers_of_two_with_only_a_lower_bound() {
        let mut project = project();
        for _ in 0..12 {
            project.step_speed(0, 0.5);
        }
        assert_eq!(project.segment(0).unwrap().speed, 1.0 / 64.0);
        for _ in 0..12 {
            project.step_speed(0, 2.0);
        }
        assert_eq!(project.segment(0).unwrap().speed, 64.0);
    }
}
