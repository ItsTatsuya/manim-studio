//! Narration playback while the stage plays (preview only; renders mux audio with ffmpeg).

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

pub struct Audio {
    // Dropping the stream stops all sound, so it lives as long as the workspace.
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Option<Sink>,
    /// What is playing: (video, scene, file).
    pub current: Option<((usize, usize), usize, PathBuf)>,
    /// Playback gain for the stage, 0..=1 (0 when muted).
    volume: f32,
}

impl Audio {
    /// `None` when there is no audio device.
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(Audio { _stream: stream, handle, sink: None, current: None, volume: 1.0 })
    }

    /// Set the stage playback gain; applies to what is playing now too.
    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0., 1.);
        if let Some(s) = &self.sink {
            s.set_volume(self.volume);
        }
    }

    /// Play `wav` from `offset` seconds at the stage volume.
    pub fn play(&mut self, key: ((usize, usize), usize), wav: &Path, offset: f32) {
        self.play_at(key, wav, offset, self.volume);
    }

    /// Play at full volume regardless of the stage controls (voice previews in Settings).
    pub fn play_full(&mut self, key: ((usize, usize), usize), wav: &Path) {
        self.play_at(key, wav, 0., 1.0);
    }

    fn play_at(&mut self, key: ((usize, usize), usize), wav: &Path, offset: f32, volume: f32) {
        self.stop();
        let Ok(file) = File::open(wav) else { return };
        let Ok(src) = Decoder::new(BufReader::new(file)) else { return };
        let Ok(sink) = Sink::try_new(&self.handle) else { return };
        sink.set_volume(volume);
        sink.append(src.skip_duration(Duration::from_secs_f32(offset.max(0.))));
        self.sink = Some(sink);
        self.current = Some((key.0, key.1, wav.to_path_buf()));
    }

    pub fn stop(&mut self) {
        if let Some(s) = self.sink.take() {
            s.stop();
        }
        self.current = None;
    }
}
