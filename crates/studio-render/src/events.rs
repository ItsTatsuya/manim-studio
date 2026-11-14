//! The engine's `--progress json` protocol: one JSON object per stderr line, tagged by
//! `"event"` (see manim-rust's docs/SCRIPTING.md, "Progress events").

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EngineEvent {
    Start {
        scene: String,
        width: u32,
        height: u32,
        fps: f64,
        renderer: String,
        output: String,
    },
    Play {
        index: usize,
        name: String,
        start: f64,
        run_time: f64,
        frames: usize,
    },
    Wait {
        #[serde(default)]
        index: usize,
        start: f64,
        run_time: f64,
        frames: usize,
    },
    /// Inside a play: `percent` of that play, `time` is scene time.
    Progress {
        index: usize,
        percent: u32,
        time: f64,
    },
    Marker {
        name: String,
        time: f64,
        kind: String,
    },
    Sound {
        source: String,
        start: f64,
        #[serde(default)]
        gain: f64,
        #[serde(default)]
        duration: Option<f64>,
    },
    Problem {
        kind: String,
        time: f64,
        #[serde(default)]
        play: Option<usize>,
        #[serde(default)]
        message: String,
    },
    Snapshot {
        path: String,
        time: f64,
        label: String,
    },
    Done {
        scene: String,
        duration: f64,
        frames: u64,
        rasterized: u64,
        elapsed: f64,
        #[serde(default)]
        outputs: Vec<String>,
        #[serde(default)]
        problems: usize,
    },
    Error {
        #[serde(default)]
        scene: String,
        message: String,
    },
    Warning {
        message: String,
    },
}

impl EngineEvent {
    /// Parse one stderr line. Non-JSON lines (script `log()` output, ffmpeg noise) are `None`.
    pub fn parse(line: &str) -> Option<EngineEvent> {
        let line = line.trim();
        if !line.starts_with('{') {
            return None;
        }
        serde_json::from_str(line).ok()
    }

    /// Scene time this event reached, for overall progress.
    pub fn time(&self) -> Option<f64> {
        match self {
            EngineEvent::Play { start, .. } | EngineEvent::Wait { start, .. } => Some(*start),
            EngineEvent::Progress { time, .. } => Some(*time),
            EngineEvent::Done { duration, .. } => Some(*duration),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documented_events() {
        let e = EngineEvent::parse(r#"{"event":"progress","index":2,"percent":40,"time":3.5}"#).unwrap();
        assert_eq!(e, EngineEvent::Progress { index: 2, percent: 40, time: 3.5 });
        let e = EngineEvent::parse(
            r#"{"event":"done","scene":"A","duration":4.0,"frames":120,"rasterized":40,"elapsed":0.8,"outputs":["a.mp4"],"problems":0}"#,
        )
        .unwrap();
        assert!(matches!(e, EngineEvent::Done { frames: 120, .. }));
        assert!(EngineEvent::parse("hello from log()").is_none());
        assert!(EngineEvent::parse(r#"{"event":"something_new"}"#).is_none());
    }
}
