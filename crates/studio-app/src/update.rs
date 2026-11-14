//! The app side of `studio-updater`: background checks, the "ready" toast, applying on
//! restart, rollback.
//!
//! Builds are only updatable when compiled with the release signing key:
//! `STUDIO_UPDATE_PUBKEY=<minisign public key> cargo build --release`. Without it the
//! updater stays off and Settings says so.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{AsyncApp, Context, WeakEntity};

use crate::app::Workspace;

/// Where releases publish `manifest-<channel>.json` and its `.minisig`.
pub const BASE_URL: &str = "https://github.com/ItsTatsuya/manim-studio/releases/download/updates";
pub const PUBLIC_KEY: Option<&str> = option_env!("STUDIO_UPDATE_PUBKEY");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `STUDIO_UPDATE_URL` points the updater at a test server (and checks at startup).
/// Manifests are still verified against the built-in key.
fn base_url() -> String {
    std::env::var("STUDIO_UPDATE_URL").unwrap_or_else(|_| BASE_URL.to_string())
}
const CHANNELS: [&str; 2] = ["stable", "beta"];

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    /// This build has no signing key.
    Disabled,
    Idle,
    Checking,
    UpToDate,
    Downloading(String),
    /// Downloaded, verified and staged; applies on restart.
    Ready { version: String, notes: String },
    Error(String),
}

fn staging() -> Option<PathBuf> {
    crate::store::data_dir().map(|d| d.join("updates"))
}

fn install_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

/// Before the window opens: tidy earlier swaps and apply a staged update by swapping
/// the files and starting the new version. Returns true when this process should exit.
pub fn apply_pending_at_start() -> bool {
    let (Some(stage), Some(dir)) = (staging(), install_dir()) else { return false };
    studio_updater::clean(&dir);
    let Some((_, staged)) = studio_updater::pending(&stage, VERSION) else { return false };
    if studio_updater::apply(&staged, &dir).is_err() {
        return false;
    }
    relaunch()
}

fn relaunch() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    std::process::Command::new(exe).spawn().is_ok()
}

impl Workspace {
    pub fn update_channel(&self) -> &'static str {
        CHANNELS[self.prefs.update_channel.min(1)]
    }

    /// Check for an update (and download it when there is one), off the UI thread.
    pub fn check_for_update(&mut self, cx: &mut Context<Self>) {
        let Some(key) = PUBLIC_KEY else {
            self.update = UpdateState::Disabled;
            cx.notify();
            return;
        };
        if matches!(self.update, UpdateState::Checking | UpdateState::Downloading(_) | UpdateState::Ready { .. }) {
            return;
        }
        let Some(stage) = staging() else { return };
        self.update = UpdateState::Checking;
        self.prefs.last_update_check = Some(crate::store::now());
        self.save_prefs();
        let channel = self.update_channel();
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let base = base_url();
            let checked = cx.background_executor().spawn(async move { studio_updater::check(&base, channel, key, VERSION) }).await;
            let manifest = match checked {
                Ok(Some(m)) => m,
                Ok(None) => {
                    this.update(cx, |this, cx| {
                        this.update = UpdateState::UpToDate;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                Err(e) => {
                    this.update(cx, |this, cx| {
                        this.update = UpdateState::Error(format!("{e:#}"));
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let version = manifest.version.clone();
            let notes = manifest.notes.clone();
            this.update(cx, |this, cx| {
                this.update = UpdateState::Downloading(version.clone());
                cx.notify();
            })
            .ok();
            let staged = cx.background_executor().spawn(async move { studio_updater::download(&manifest, &stage) }).await;
            this.update(cx, |this, cx| {
                match staged {
                    Ok(_) => {
                        this.update = UpdateState::Ready { version, notes };
                        this.toast = true;
                    }
                    Err(e) => this.update = UpdateState::Error(format!("{e:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Daily background check after startup (quietly; errors only show in Settings).
    pub fn schedule_update_check(&mut self, cx: &mut Context<Self>) {
        if PUBLIC_KEY.is_none() {
            self.update = UpdateState::Disabled;
            return;
        }
        if let (Some(stage), true) = (staging(), true)
            && let Some((version, _)) = studio_updater::pending(&stage, VERSION) {
                self.update = UpdateState::Ready { version, notes: String::new() };
                return;
            }
        let testing = std::env::var_os("STUDIO_UPDATE_URL").is_some();
        let due = testing || self.prefs.last_update_check.is_none_or(|t| crate::store::now().saturating_sub(t) > 86_400);
        if !due {
            return;
        }
        let delay = if testing { 2 } else { 20 };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_secs(delay)).await;
            this.update(cx, |this, cx| this.check_for_update(cx)).ok();
        })
        .detach();
    }

    /// Install the staged update now: swap files, start the new version, quit.
    pub fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        let (Some(stage), Some(dir)) = (staging(), install_dir()) else { return };
        let Some((_, staged)) = studio_updater::pending(&stage, VERSION) else { return };
        match studio_updater::apply(&staged, &dir) {
            Ok(()) => {
                if relaunch() {
                    cx.quit();
                }
            }
            Err(e) => {
                self.update = UpdateState::Error(format!("{e:#}"));
                cx.notify();
            }
        }
    }

    pub fn can_roll_back(&self) -> bool {
        install_dir().is_some_and(|d| studio_updater::can_roll_back(&d))
    }

    /// Go back to the previous version and restart.
    pub fn roll_back(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = install_dir() else { return };
        match studio_updater::roll_back(&dir) {
            Ok(()) => {
                if relaunch() {
                    cx.quit();
                }
            }
            Err(e) => {
                self.notice = Some(format!("Couldn't roll back: {e:#}"));
                cx.notify();
            }
        }
    }
}
