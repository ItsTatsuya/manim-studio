//! Disk probes the views read while rendering, cached so a frame never touches the disk.
//!
//! Each value is re-probed when it is older than `TTL` (a low-rate refresh, so changes made
//! outside the app still show up) or when `Probes::clear` is called after something the app
//! itself did: an asset added, a download finished, a screen or tab opened.

use std::rc::Rc;
use std::time::{Duration, Instant};

/// How long a probe is trusted without an explicit invalidation.
pub const TTL: Duration = Duration::from_secs(2);

/// One cached probe, keyed by the inputs it was computed from.
pub struct Cached<K, T> {
    v: Option<(Instant, K, T)>,
}

impl<K, T> Default for Cached<K, T> {
    fn default() -> Self {
        Self { v: None }
    }
}

impl<K: PartialEq, T: Clone> Cached<K, T> {
    pub fn get(&mut self, key: K, f: impl FnOnce(&K) -> T) -> T {
        if let Some((at, k, v)) = &self.v
            && *k == key
            && at.elapsed() < TTL
        {
            return v.clone();
        }
        let v = f(&key);
        self.v = Some((Instant::now(), key, v.clone()));
        v
    }

    pub fn clear(&mut self) {
        self.v = None;
    }
}

pub type AssetList = Rc<Vec<(String, String)>>;

#[derive(Default)]
pub struct Probes {
    /// The open video's assets folder, by video folder.
    pub assets: Cached<std::path::PathBuf, AssetList>,
    /// A natural voice is installed and loadable.
    pub kokoro_usable: Cached<(), bool>,
    /// The picked model is downloaded (key: model index).
    pub kokoro_installed: Cached<usize, bool>,
    /// Bytes still to download for the picked model (key: model index).
    pub kokoro_size: Cached<usize, u64>,
    pub espeak: Cached<(), Option<studio_render::kokoro::espeak::Espeak>>,
    pub crash_report: Cached<(), bool>,
    pub can_roll_back: Cached<(), bool>,
    /// A script's modification time (key: path).
    pub code_mtime: Cached<std::path::PathBuf, Option<std::time::SystemTime>>,
}

impl Probes {
    /// Forget everything: the next read probes the disk again.
    pub fn clear(&mut self) {
        self.assets.clear();
        self.kokoro_usable.clear();
        self.kokoro_installed.clear();
        self.kokoro_size.clear();
        self.espeak.clear();
        self.crash_report.clear();
        self.can_roll_back.clear();
        self.code_mtime.clear();
    }
}
