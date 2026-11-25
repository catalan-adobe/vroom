use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use super::{Project, start_key};

#[derive(Serialize)]
struct SaveFile<'a> {
    version: u8,
    video: &'a str,
    segments: Vec<SavedSegment>,
}

#[derive(Serialize)]
struct SavedSegment {
    start: f64,
    #[serde(skip_serializing_if = "is_false")]
    cut: bool,
    speed: f64,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Deserialize)]
struct LoadFile {
    // Read but do not use the stored path/version: metadata comes from the
    // source selected at launch, just as it did in the Go editor.
    #[serde(rename = "version")]
    _version: Option<i64>,
    #[serde(rename = "video")]
    _video: Option<String>,
    segments: Option<Vec<Option<LoadedSegment>>>,
}

#[derive(Deserialize)]
struct LoadedSegment {
    start: Option<f64>,
    cut: Option<bool>,
    speed: Option<f64>,
}

impl Project {
    pub fn save(&self, path: &Path) -> Result<()> {
        let segments: Vec<_> = self
            .segments()
            .map(|s| SavedSegment {
                start: s.start,
                cut: s.cut,
                speed: s.speed,
            })
            .collect();
        ensure!(
            segments.iter().all(|s| s.speed.is_finite()),
            "segment speed is not finite"
        );
        let video = self.source.to_string_lossy();
        let file = SaveFile {
            version: 1,
            video: &video,
            segments,
        };
        let bytes = serde_json::to_vec_pretty(&file)?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o644);
        }
        options.open(path)?.write_all(&bytes)?;
        Ok(())
    }

    pub fn load(&mut self, path: &Path) -> Result<()> {
        let file: Option<LoadFile> = serde_json::from_slice(&fs::read(path)?)?;
        // Only reset the project after decoding succeeds.
        self.marks.clear();
        self.edits.clear();
        if let Some(segments) = file.and_then(|file| file.segments) {
            for segment in segments.into_iter().flatten() {
                let start = segment.start.unwrap_or_default();
                if start > 0.0 {
                    self.marks.push(start);
                }
                let key = start_key(start);
                if segment.cut.unwrap_or_default() {
                    self.edits.entry(key).or_default().cut = true;
                }
                if let Some(speed) = segment.speed.filter(|&speed| speed != 0.0 && speed != 1.0) {
                    self.edits.entry(key).or_default().speed = speed;
                }
            }
            self.marks.sort_by(f64::total_cmp);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::project;

    #[test]
    fn legacy_json_restores_edits_not_source_metadata_and_round_trips_times() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("a.vroom");
        fs::write(
            &path,
            r#"{
            "version": 41, "video": "/old/path.mp4", "unknown": true,
            "segments": [
                {"start": 5.2, "cut": true, "speed": 0.5},
                {"start": 0}, {"start": 3.0000000000000004, "speed": 2},
                {"start": 8.0, "speed": 0}
            ]
        }"#,
        )?;
        let mut project = project();
        project.load(&path)?;
        let segments: Vec<_> = project.segments().collect();
        assert_eq!(project.marks(), &[3.0000000000000004, 5.2, 8.0]);
        assert_eq!(project.info.duration, 12.0);
        assert!(segments[2].cut);
        assert_eq!(segments[2].speed, 0.5);
        project.save(&path)?;
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(saved["version"], 1);
        assert_eq!(saved["video"], "demo.mp4");
        assert_eq!(
            saved["segments"][0],
            serde_json::json!({ "start": 0.0, "speed": 1.0 })
        );
        project.load(&path)?;
        assert_eq!(segments, project.segments().collect::<Vec<_>>());
        Ok(())
    }

    #[test]
    fn loading_is_lenient_but_invalid_json_leaves_existing_state_untouched() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("a.vroom");
        fs::write(
            &path,
            r#"{"segments":[null,{"start":-0.0,"cut":true},
            {"start":4,"cut":true,"speed":0.01}, {"start":4,"cut":false,"speed":1},
            {"start":15,"speed":null}]}"#,
        )?;
        let mut project = project();
        project.load(&path)?;
        assert_eq!(project.marks(), &[4.0, 4.0, 15.0]);
        assert!(project.segment(0).unwrap().cut);
        assert!(project.segment(1).unwrap().cut);
        assert_eq!(project.segment(2).unwrap().speed, 0.01);

        fs::write(&path, "not JSON")?;
        assert!(project.load(&path).is_err());
        assert_eq!(project.marks(), &[4.0, 4.0, 15.0]);
        Ok(())
    }
}
