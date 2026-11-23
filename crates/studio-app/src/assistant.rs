//! The assistant in the workspace: storyboard → approve → build, chat edits scoped to
//! the selected scene, and local checks with bounded repairs (DESIGN.md §6).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use gpui::{AsyncApp, Context, WeakEntity};
use studio_agent::detect;
use studio_agent::events::{RateLimits, Tokens};
use studio_agent::prompt::{self, PlanOptions};
use studio_agent::turn::{self, EDIT_TOOLS, Turn, TurnRequest};
use studio_agent::{AgentEvent, Effort};
use studio_render::preview::OpenScene;
use studio_ui::IconName;

use crate::app::{Mode, Screen, Workspace};
use crate::model::{Busy, Issue, Msg, Provider, VideoStatus, format_tokens};
use crate::store;

/// What a turn is for; decides what happens when it ends.
#[derive(Clone, Debug, PartialEq)]
pub enum TurnKind {
    Plan,
    Revise,
    Build,
    Edit,
    /// Repair scene index `scene`, attempt `k`. `build`: the repairs follow a build (so
    /// when they finish, the studio opens and the render can start).
    Repair { scene: usize, k: u8, build: bool },
    /// The assistant looks at frames of the video and fixes what looks wrong. `build`: it
    /// follows a build (so when it ends, the studio opens and the render can start).
    Review { build: bool },
}

impl TurnKind {
    /// A build, or the repairs that follow one.
    fn is_build(&self) -> bool {
        matches!(self, TurnKind::Build | TurnKind::Repair { build: true, .. } | TurnKind::Review { build: true })
    }
}

/// Move what the engine already knows (script is the same file) from the old scenes to
/// the freshly paired ones: thumbnail, timings, check result, narration if the line is
/// unchanged. Returns the images of the old scenes that were not carried over, to free.
fn carry_engine(fresh: &mut [crate::model::Scene], old: Vec<crate::model::Scene>) -> Vec<std::sync::Arc<gpui::RenderImage>> {
    let mut leftover = Vec::new();
    let mut old = old.into_iter();
    for new in fresh.iter_mut() {
        let Some(mut o) = old.next() else { break };
        if new.engine.file.is_some() && new.engine.file == o.engine.file {
            new.engine.thumb = o.engine.thumb.take();
            new.engine.duration = o.engine.duration;
            new.engine.key_time = o.engine.key_time;
            new.engine.issue = o.engine.issue.take();
            new.engine.checked = o.engine.checked;
        }
        // The voice belongs to the line, not the script: a build pairs storyboard scenes
        // (no file yet, voice already spoken) with their new files.
        if o.say.trim() == new.say.trim() {
            new.engine.voice = o.engine.voice.take();
        }
        leftover.extend(o.engine.thumb.take().map(|t| t.0));
    }
    leftover.extend(old.filter_map(|mut o| o.engine.thumb.take().map(|t| t.0)));
    leftover
}

/// Modification time and size of the scene scripts (`scenes/` and `lib/`), keyed by path.
type FileStamps = std::collections::BTreeMap<PathBuf, (Option<std::time::SystemTime>, u64)>;

/// A cheap stat of the files a turn may edit. Codex edits through shell commands that
/// emit no file events, so what changed is found by comparing these.
fn snapshot_scripts(dir: &Path) -> FileStamps {
    let mut out = FileStamps::new();
    for sub in ["scenes", "lib"] {
        let Ok(entries) = std::fs::read_dir(dir.join(sub)) else { continue };
        for e in entries.flatten() {
            if let Ok(m) = e.metadata()
                && m.is_file()
            {
                out.insert(e.path(), (m.modified().ok(), m.len()));
            }
        }
    }
    out
}

/// Files added, removed or modified between two snapshots.
fn changed_between(before: &FileStamps, after: &FileStamps) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = after.iter().filter(|(p, st)| before.get(*p) != Some(*st)).map(|(p, _)| p.clone()).collect();
    out.extend(before.keys().filter(|p| !after.contains_key(*p)).cloned());
    out
}

/// A running assistant turn for a video.
/// A turn as it was asked for, kept when it fails so "Try again" repeats it exactly.
#[derive(Clone, Debug)]
pub struct RetrySpec {
    kind: TurnKind,
    prompt: String,
    planning: bool,
    effort: Option<Effort>,
}

pub struct Running {
    pub video: (usize, usize),
    pub kind: TurnKind,
    retry: RetrySpec,
    turn: Turn,
    started: Instant,
    text: String,
    changed: Vec<PathBuf>,
    /// The scripts as they were when the turn started.
    before: FileStamps,
    provider: Provider,
    /// Latest token report per model response (Claude), for the per-turn budget.
    usage: Vec<(String, Tokens)>,
    /// Stopped because it passed the turn budget.
    over_budget: bool,
}

fn digest() -> &'static str {
    static D: OnceLock<String> = OnceLock::new();
    D.get_or_init(|| prompt::digest_for(studio_render::TYPES, prompt::Recipe::default().digest))
}

/// Write the system prompt files (and the layout kit they rely on) into the video folder if they changed. They are the
/// same bytes every turn, so the provider's prompt cache serves them.
fn system_files(dir: &Path) -> std::io::Result<(PathBuf, PathBuf)> {
    // The prompt places content through lib/layout.ts, so every video needs the current one.
    studio_render::kit::refresh(dir)?;
    let build = dir.join("AGENTS.md");
    let text = prompt::build_system_with(digest(), prompt::Recipe::default());
    if std::fs::read_to_string(&build).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&build, &text)?;
    }
    let studio = dir.join(".studio");
    std::fs::create_dir_all(&studio)?;
    let plan = studio.join("plan.md");
    if std::fs::read_to_string(&plan).ok().as_deref() != Some(prompt::PLAN_SYSTEM) {
        std::fs::write(&plan, prompt::PLAN_SYSTEM)?;
    }
    Ok((build, plan))
}

impl Workspace {
    // ------------------------------------------------------------------ providers

    /// Look for the CLIs and their sign-in state in the background (no tokens spent).
    pub fn detect_providers(&mut self, cx: &mut Context<Self>) {
        // One detection at a time: the one in flight reports what is installed now.
        if self.detecting {
            return;
        }
        self.detecting = true;
        for p in &mut self.providers {
            p.checking = true;
        }
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let found = cx
                .background_executor()
                .spawn(async {
                    // A panic in the probe must not leave the "checking" flags set forever.
                    std::panic::catch_unwind(|| [Provider::Claude, Provider::OpenAi].map(|p| (p, detect::status(p.agent())))).ok()
                })
                .await;
            this.update(cx, |this, cx| {
                this.detecting = false;
                for s in &mut this.providers {
                    s.checking = false;
                }
                for (p, st) in found.into_iter().flatten() {
                    if let Some(s) = this.providers.iter_mut().find(|s| s.provider == p) {
                        s.checking = false;
                        s.installed = st.path.is_some();
                        s.version = st.version;
                        s.signed_in = st.signed_in;
                        s.plan = st.plan;
                    }
                }
                // Prefer a signed-in assistant as the default.
                let default_ok = this.providers.iter().any(|s| s.provider == this.default_provider && s.signed_in);
                if !default_ok
                    && let Some(s) = this.providers.iter().find(|s| s.signed_in) {
                        this.default_provider = s.provider;
                    }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn install_provider(&mut self, p: Provider, cx: &mut Context<Self>) {
        if let Err(e) = detect::install(p.agent()) {
            self.notice = Some(format!("Couldn't start the {} installer: {e}", p.cli()));
        }
        self.poll_providers(cx);
    }

    pub fn sign_in_provider(&mut self, p: Provider, cx: &mut Context<Self>) {
        if let Err(e) = detect::sign_in(p.agent()) {
            self.notice = Some(format!("Couldn't start {} sign-in: {e}", p.cli()));
        }
        self.poll_providers(cx);
    }

    /// Re-detect every few seconds for a while (after an install or sign-in window opens).
    fn poll_providers(&mut self, cx: &mut Context<Self>) {
        // One loop at a time: a new Install or Sign-in click replaces the old one.
        self.provider_poll = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            for _ in 0..40 {
                cx.background_executor().timer(std::time::Duration::from_secs(4)).await;
                let Ok(()) = this.update(cx, |this, cx| this.detect_providers(cx)) else { return };
            }
        }));
    }

    /// The assistant to use: the default if signed in, else another signed-in one when
    /// auto-switch is on.
    fn pick_provider(&self) -> Option<(Provider, PathBuf)> {
        let usable = |p: Provider| {
            self.providers.iter().find(|s| s.provider == p && s.signed_in).and_then(|_| detect::find(p.agent())).map(|path| (p, path))
        };
        usable(self.default_provider).or_else(|| {
            if self.prefs.auto_switch_provider {
                [Provider::Claude, Provider::OpenAi].into_iter().filter(|p| *p != self.default_provider).find_map(usable)
            } else {
                None
            }
        })
    }

    fn apply_rate_limits(&mut self, p: Provider, rl: RateLimits) {
        if let Some(s) = self.providers.iter_mut().find(|s| s.provider == p) {
            if rl.window.is_some() {
                s.window_used = rl.window;
            }
            if rl.week.is_some() {
                s.week_used = rl.week;
            }
            if rl.window_resets_at.is_some() {
                s.resets_at = rl.window_resets_at;
            }
            if rl.week_resets_at.is_some() {
                s.week_resets_at = rl.week_resets_at;
            }
        }
        // Rate-limit events arrive constantly: only a change is worth a write.
        if self.prefs.remember_usage(p, &rl) {
            self.save_prefs();
        }
    }

    // ------------------------------------------------------------------ turns

    /// The assistant is working on `video`: a turn is running, or one just ended and its
    /// scenes are being checked (`video.busy` stays set until the work is finished).
    pub fn busy(&self, video: (usize, usize)) -> bool {
        self.running.iter().any(|r| r.video == video)
            || self.projects.get(video.0).and_then(|p| p.videos.get(video.1)).is_some_and(|v| v.busy.is_some())
    }

    /// Detection of the assistant CLIs is still running.
    pub fn checking_providers(&self) -> bool {
        self.providers.iter().any(|p| p.checking)
    }

    /// What "Auto" reasoning means for builds and edits. Sonnet at low effort used fewer
    /// tokens and passed the engine check more often than at its default in the token
    /// benchmark (tools/tokenbench, 2026-11-13: 26k vs 34k tokens, 94% vs 82% first-try
    /// passes). Haiku needed more repairs at low effort, so it keeps the CLI's default.
    fn auto_build_effort(&self) -> Option<Effort> {
        let provider = self.pick_provider().map(|(p, _)| p)?;
        (provider == Provider::Claude && self.prefs.build_tier == crate::model::Tier::Balanced).then_some(Effort::Low)
    }

    fn video_mut(&mut self, v: (usize, usize)) -> Option<&mut crate::model::Video> {
        self.projects.get_mut(v.0).and_then(|p| p.videos.get_mut(v.1))
    }

    fn working_label(&self, v: (usize, usize)) -> Option<String> {
        self.projects.get(v.0).and_then(|p| p.videos.get(v.1)).and_then(|video| {
            video.chat.iter().find_map(|m| match m {
                Msg::Working(w) => Some(w.label.clone()),
                _ => None,
            })
        })
    }

    fn set_working(&mut self, v: (usize, usize), label: Option<String>) {
        self.set_working_progress(v, label, None);
    }

    /// Show (or clear, with `None`) the live status line. The timer keeps running
    /// across label changes until the work is done.
    fn set_working_progress(&mut self, v: (usize, usize), label: Option<String>, progress: Option<(usize, usize)>) {
        if let Some(video) = self.video_mut(v) {
            let since = video.chat.iter().find_map(|m| match m {
                Msg::Working(w) => Some(w.since),
                _ => None,
            });
            video.chat.retain(|m| !matches!(m, Msg::Working(_)));
            if let Some(label) = label {
                video.chat.push(Msg::Working(crate::model::Working { label, since: since.unwrap_or_else(Instant::now), progress }));
            }
        }
        if v == self.current {
            self.chat_scroll.scroll_to_bottom();
        }
    }

    fn push_msg(&mut self, v: (usize, usize), msg: Msg) {
        if let Some(video) = self.video_mut(v) {
            // Keep the live "working" row last.
            let working = video.chat.iter().position(|m| matches!(m, Msg::Working(_)));
            match working {
                Some(i) => video.chat.insert(i, msg),
                None => video.chat.push(msg),
            }
        }
        if v == self.current {
            self.chat_scroll.scroll_to_bottom();
        }
    }

    /// A render of `v` is queued or running (it owns the video's status meanwhile).
    fn render_active(&self, v: (usize, usize)) -> bool {
        self.jobs.iter().any(|j| j.video == v && j.status.active())
    }

    /// A build turn that didn't produce scenes: leave "Building". Only for turns that set
    /// that status, and never while a render of the video is running (it owns the status).
    fn reset_building(&mut self, v: (usize, usize), build: bool) {
        if !build || self.render_active(v) {
            return;
        }
        if let Some(video) = self.video_mut(v)
            && video.status == VideoStatus::Building
        {
            video.status = if video.scenes.iter().any(|s| s.engine.file.is_some()) { VideoStatus::Failed } else { VideoStatus::Draft };
        }
    }

    fn fail_early(&mut self, v: (usize, usize), build: bool, text: String, cx: &mut Context<Self>) {
        self.set_working(v, None);
        self.push_msg(v, Msg::Activity { icon: IconName::Info, ok: false, text, meta: String::new() });
        if let Some(video) = self.video_mut(v) {
            video.busy = None;
        }
        self.reset_building(v, build);
        cx.notify();
    }

    /// The assistant failed: explain it in the chat (and on the storyboard) and keep the
    /// turn so it can be retried.
    fn fail_problem(&mut self, v: (usize, usize), provider: Provider, raw: String, retry: Option<RetrySpec>, cx: &mut Context<Self>) {
        self.set_working(v, None);
        self.push_msg(v, Msg::Problem { provider, raw });
        let build = retry.as_ref().is_some_and(|r| r.kind.is_build());
        let dir = self.video_mut(v).and_then(|video| {
            video.busy = None;
            video.dir.clone()
        });
        self.reset_building(v, build);
        if let (Some(dir), Some(retry)) = (dir, retry) {
            self.retries.insert(dir, retry);
        }
        self.persist(v);
        cx.notify();
    }

    /// "Try again" on a failure: repeat the failed turn, or plan again after a restart.
    pub fn retry_failed(&mut self, cx: &mut Context<Self>) {
        let video = self.current;
        if self.busy(video) {
            return;
        }
        let dir = self.video().dir.clone();
        let spec = dir.and_then(|d| self.retries.remove(&d));
        let Some(spec) = spec else {
            if self.video().storyboard.is_none() {
                self.retry_plan(cx);
            }
            return;
        };
        if let Some(v) = self.video_mut(video) {
            v.busy = Some(match spec.kind {
                TurnKind::Plan => Busy::Planning,
                TurnKind::Revise => Busy::Revising,
                TurnKind::Build => Busy::Building,
                TurnKind::Edit => Busy::Editing,
                TurnKind::Repair { scene, k, .. } => Busy::Fixing { scene: scene + 1, k, of: k },
                TurnKind::Review { .. } => Busy::Reviewing,
            });
            if spec.kind == TurnKind::Build {
                v.status = VideoStatus::Building;
            }
        }
        if !spec.planning {
            self.set_working(video, Some("Trying again".into()));
        }
        self.start_turn(video, spec.kind, spec.prompt, spec.planning, spec.effort, cx);
    }

    /// Whether "Try again" has something to repeat for the open video.
    pub fn can_retry(&self) -> bool {
        self.video().dir.as_ref().is_some_and(|d| self.retries.contains_key(d)) || self.video().storyboard.is_none()
    }

    /// The other signed-in assistant, if there is one (for "Use ChatGPT instead").
    pub fn other_provider(&self, provider: Provider) -> Option<Provider> {
        self.providers.iter().find(|p| p.provider != provider && p.signed_in).map(|p| p.provider)
    }

    fn start_turn(
        &mut self,
        video: (usize, usize),
        kind: TurnKind,
        prompt_text: String,
        planning: bool,
        effort: Option<Effort>,
        cx: &mut Context<Self>,
    ) {
        self.start_turn_with(video, kind, prompt_text, planning, effort, Vec::new(), cx);
    }

    /// `images`: pictures the assistant is to look at (the visual review).
    #[allow(clippy::too_many_arguments)]
    fn start_turn_with(
        &mut self,
        video: (usize, usize),
        kind: TurnKind,
        prompt_text: String,
        planning: bool,
        effort: Option<Effort>,
        images: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let build = kind.is_build();
        let Some((provider, exe)) = self.pick_provider() else {
            if self.checking_providers() {
                // Detection is still running (just after launch): wait for it instead of
                // saying nothing is connected. The caller's label comes back afterwards.
                let previous = self.working_label(video);
                if !planning {
                    self.set_working(video, Some("Checking assistants".into()));
                }
                cx.notify();
                cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                    for _ in 0..100 {
                        cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
                        if this.update(cx, |this, _| !this.checking_providers()).unwrap_or(true) {
                            break;
                        }
                    }
                    this.update(cx, |this, cx| {
                        if !planning {
                            this.set_working(video, previous);
                        }
                        if this.checking_providers() {
                            this.fail_early(video, build, "Still checking which assistants are connected. Try again in a moment.".into(), cx);
                        } else {
                            this.start_turn_with(video, kind, prompt_text, planning, effort, images, cx);
                        }
                    })
                    .ok();
                })
                .detach();
                return;
            }
            self.fail_early(video, build, "No assistant is signed in. Open Settings → Assistants to connect Claude or ChatGPT.".into(), cx);
            self.notice = Some("Connect an assistant first: Settings → Assistants & usage.".into());
            return;
        };
        let Some(dir) = self.projects.get(video.0).and_then(|p| p.videos.get(video.1)).and_then(|v| v.dir.clone()) else {
            return self.fail_early(video, build, "This video has no folder yet.".into(), cx);
        };
        let (build_file, plan_file) = match system_files(&dir) {
            Ok(f) => f,
            Err(e) => return self.fail_early(video, build, format!("Couldn't prepare the video folder: {e}"), cx),
        };
        let tier = if planning { self.prefs.planning_tier } else { self.prefs.build_tier };
        let mut effort = effort;
        // Codex has no model choice here: the tier sets how hard it thinks.
        if provider == Provider::OpenAi && effort.is_none() {
            effort = Some(match tier {
                crate::model::Tier::Fast => Effort::Low,
                crate::model::Tier::Balanced => Effort::Medium,
                crate::model::Tier::Best => Effort::High,
            });
        }
        let v = &self.projects[video.0].videos[video.1];
        // Repairs continue the turn they fix. With lean context, each edit starts fresh and
        // sees only the scene in scope (plus the cached system prompt), instead of
        // carrying the whole build conversation along.
        let resume = match (&kind, &v.session) {
            (TurnKind::Repair { .. }, Some((p, id))) if *p == provider => Some(id.clone()),
            (TurnKind::Edit, Some((p, id))) if *p == provider && (!self.prefs.lean_context || v.resume_next) => Some(id.clone()),
            _ => None,
        };
        let retry = RetrySpec { kind: kind.clone(), prompt: prompt_text.clone(), planning, effort };
        self.retries.remove(&dir);
        let before = snapshot_scripts(&dir);
        let req = TurnRequest {
            provider: provider.agent(),
            exe,
            cwd: dir,
            system_file: if planning { plan_file } else { build_file },
            prompt: prompt_text,
            model: tier.size().model(provider.agent()),
            effort,
            resume,
            tools: if planning { vec![] } else { EDIT_TOOLS.to_vec() },
            ephemeral: planning,
            images,
        };
        if let Some(v) = self.video_mut(video) {
            v.resume_next = false;
        }
        let turn = match turn::start(req) {
            Ok(t) => t,
            Err(e) => return self.fail_problem(video, provider, format!("Couldn't start {}: {e:#}", provider.cli()), Some(retry), cx),
        };
        let events = turn.events.clone();
        // Events are routed by the turn's own id: a video can have a new turn right after
        // the last one (a repair follows its build), and the id never repeats.
        let turn_id = turn.id();
        self.running.push(Running {
            video,
            kind,
            retry,
            turn,
            started: Instant::now(),
            text: String::new(),
            changed: vec![],
            before,
            provider,
            usage: vec![],
            over_budget: false,
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut done = false;
            while let Ok(ev) = events.recv().await {
                done = matches!(ev, AgentEvent::Done { .. });
                if this.update(cx, |this, cx| this.on_agent_event(turn_id, ev, cx)).is_err() || done {
                    break;
                }
            }
            if !done {
                // The channel closed without a `Done`: never leave the video busy forever.
                let ev = AgentEvent::Done {
                    session: None,
                    tokens: Tokens::default(),
                    cost_usd: None,
                    error: Some("the assistant stopped unexpectedly".into()),
                };
                this.update(cx, |this, cx| this.on_agent_event(turn_id, ev, cx)).ok();
            }
        })
        .detach();
        cx.notify();
    }

    /// The "+" after the last scene: start a request for one more scene in the chat, for
    /// the teacher to finish and send.
    pub fn ask_for_new_scene(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.right_tab = crate::app::RightTab::Assistant;
        let n = self.video().scenes.len();
        // A new chapter is a change to the whole video, not to the selected chapter.
        self.edit_whole = true;
        // The last chapter is the ending: the new one goes before it.
        let text = if n > 1 {
            format!("Add a chapter between chapter {} and chapter {n} (the ending) that ", n - 1)
        } else {
            "Add a chapter after chapter 1 that ".to_string()
        };
        self.chat_input.update(cx, |i, cx| i.set_text(text, cx));
        let handle = gpui::Focusable::focus_handle(self.chat_input.read(cx), cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Stop the assistant working on `video`.
    pub fn stop_assistant(&mut self, video: (usize, usize), cx: &mut Context<Self>) {
        for r in self.running.iter().filter(|r| r.video == video) {
            r.turn.cancel();
        }
        cx.notify();
    }

    fn on_agent_event(&mut self, turn_id: u64, ev: AgentEvent, cx: &mut Context<Self>) {
        let Some(ix) = self.running.iter().position(|r| r.turn.id() == turn_id) else { return };
        let video = self.running[ix].video;
        let provider = self.running[ix].provider;
        let kind = self.running[ix].kind.clone();
        let quiet = matches!(kind, TurnKind::Plan | TurnKind::Revise);
        match ev {
            AgentEvent::Started { session } => {
                if !quiet
                    && let Some(v) = self.video_mut(video) {
                        v.session = Some((provider, session));
                    }
            }
            AgentEvent::Text(t) => {
                if quiet {
                    self.running[ix].text.push_str(&t);
                    self.running[ix].text.push('\n');
                } else {
                    self.push_msg(video, Msg::Assistant(t));
                }
            }
            AgentEvent::FileChanged { path } => {
                let dir = self.projects[video.0].videos[video.1].dir.clone().unwrap_or_default();
                let abs = if Path::new(&path).is_absolute() { PathBuf::from(&path) } else { dir.join(&path) };
                if !self.running[ix].changed.contains(&abs) {
                    self.running[ix].changed.push(abs);
                    // The status line says what kind of work is happening, not which files.
                    let done = self.running[ix].changed.len();
                    match kind {
                        TurnKind::Build => {
                            let of = self.projects[video.0].videos[video.1].storyboard.as_ref().map_or(0, |s| s.scenes.len());
                            self.set_working_progress(video, Some("Creating chapters".into()), (of > 0).then_some((done.min(of), of)));
                        }
                        TurnKind::Repair { .. } => {}
                        _ => self.set_working(video, Some("Editing".into())),
                    }
                }
            }
            AgentEvent::Tool { .. } => {}
            AgentEvent::Usage { message, tokens } => {
                let r = &mut self.running[ix];
                match r.usage.iter_mut().find(|(m, _)| *m == message) {
                    Some(slot) => slot.1 = tokens,
                    None => r.usage.push((message, tokens)),
                }
                // "Ask before a big turn": chat edits stop at the budget and offer to go on.
                // (Builds show their estimate before approval; repairs are already bounded.)
                let spent: u64 = r.usage.iter().map(|(_, t)| t.billed()).sum();
                let budget = crate::model::TURN_BUDGETS[self.prefs.turn_budget.min(crate::model::TURN_BUDGETS.len() - 1)].1;
                if let (TurnKind::Edit, Some(limit), false) = (&r.kind, budget, r.over_budget)
                    && spent > limit as u64 {
                        r.over_budget = true;
                        r.turn.cancel();
                    }
            }
            AgentEvent::RateLimit(rl) => self.apply_rate_limits(provider, rl),
            AgentEvent::Done { session, tokens, error, .. } => {
                let r = self.running.remove(ix);
                if let (Some(s), false) = (session, quiet)
                    && let Some(v) = self.video_mut(video) {
                        v.session = Some((provider, s));
                    }
                self.after_turn(r, tokens, error, cx);
            }
        }
        cx.notify();
    }

    fn usage_line(&self, provider: Provider, tokens: Tokens, secs: f32) -> Option<String> {
        if tokens.billed() == 0 {
            return None;
        }
        let mut s = format!("{} used {} tokens", provider.name(), format_tokens(tokens.billed()));
        if tokens.cached > 0 {
            s.push_str(&format!(" (+{} cached)", format_tokens(tokens.cached)));
        }
        if let Some(w) = self.providers.iter().find(|p| p.provider == provider).and_then(|p| p.window_used) {
            s.push_str(&format!(" · plan window at {}%", (w * 100.).round() as u32));
        }
        s.push_str(&format!(" · {secs:.0} s"));
        Some(s)
    }

    fn after_turn(&mut self, r: Running, tokens: Tokens, error: Option<String>, cx: &mut Context<Self>) {
        let video = r.video;
        let secs = r.started.elapsed().as_secs_f32();
        let usage = self.usage_line(r.provider, tokens, secs);
        if r.over_budget {
            self.set_working(video, None);
            let limit = crate::model::TURN_BUDGETS[self.prefs.turn_budget.min(crate::model::TURN_BUDGETS.len() - 1)].0;
            self.push_msg(video, Msg::Activity {
                icon: IconName::Info,
                ok: false,
                text: format!("Paused: this turn passed {limit}. Say “continue” to let it finish, or ask for something smaller."),
                meta: String::new(),
            });
            if let Some(v) = self.video_mut(video) {
                v.busy = None;
                // "continue" should pick up where it stopped.
                v.resume_next = true;
            }
            self.persist(video);
            cx.notify();
            return;
        }
        if let Some(e) = error {
            // Cancelled from the app ("Stop"): no explanation needed.
            if r.turn.cancelled() {
                self.set_working(video, None);
                self.push_msg(video, Msg::Activity { icon: IconName::Info, ok: false, text: "Stopped".into(), meta: String::new() });
            } else {
                return self.fail_problem(video, r.provider, e, Some(r.retry), cx);
            }
            if let Some(v) = self.video_mut(video) {
                v.busy = None;
            }
            self.reset_building(video, r.kind.is_build());
            self.persist(video);
            cx.notify();
            return;
        }
        match r.kind {
            TurnKind::Plan | TurnKind::Revise => {
                let mut refresh_voice = false;
                // A video made from a teacher's script: the reply is cue marks, and the
                // teacher's own words are checked against it.
                let mut told: Vec<String> = Vec::new();
                let teacher = self.projects[video.0].videos[video.1].dir.as_deref().and_then(crate::teacher::load);
                let parsed = match teacher {
                    Some(saved) => {
                        let before = self.projects[video.0].videos[video.1].storyboard.clone();
                        let (parsed, lines) = crate::teacher::from_reply(&saved, before.as_ref(), matches!(r.kind, TurnKind::Plan), &r.text);
                        told = lines;
                        parsed
                    }
                    None => prompt::parse_storyboard(&r.text).map_err(|e| e.to_string()),
                };
                let v = &mut self.projects[video.0].videos[video.1];
                v.busy = None;
                for text in told {
                    v.chat.push(Msg::Activity { icon: IconName::Info, ok: true, text, meta: String::new() });
                }
                match parsed {
                    Ok(sb) => {
                        v.title = sb.title.clone();
                        v.grade = sb.grade.clone();
                        // A revised plan keeps the scripts, notes, thumbnails and voices of the
                        // scenes that are still there (the folder's files, in order).
                        let old = std::mem::take(&mut v.scenes);
                        v.scenes = match v.dir.clone() {
                            Some(dir) => {
                                let notes: Vec<Option<String>> = old.iter().map(|s| s.note.clone()).collect();
                                store::scenes_for(&dir, Some(&sb), &notes)
                            }
                            None => sb.scenes.iter().map(crate::model::Scene::from_beat).collect(),
                        };
                        let leftover = carry_engine(&mut v.scenes, old);
                        let scene_count = v.scenes.len();
                        v.storyboard = Some(sb);
                        self.stage.dead.extend(leftover);
                        if video == self.current && self.scene >= scene_count {
                            self.scene = scene_count.saturating_sub(1);
                        }
                        let v = &mut self.projects[video.0].videos[video.1];
                        v.updated = store::now();
                        refresh_voice = true;
                        if let Some(u) = usage {
                            v.chat.push(Msg::Usage(format!("Script · {u}")));
                        }
                    }
                    Err(e) => {
                        v.chat.push(Msg::Activity {
                            icon: IconName::Info,
                            ok: false,
                            text: format!("The plan didn't come back in the right shape ({e}). Try again."),
                            meta: String::new(),
                        });
                        self.notice = Some("The script didn't come back in the right shape. Try again or revise the request.".into());
                    }
                }
                self.persist(video);
                if refresh_voice {
                    self.refresh_voice(video, cx);
                }
            }
            TurnKind::Build | TurnKind::Edit | TurnKind::Repair { .. } | TurnKind::Review { .. } => {
                if let Some(u) = usage {
                    self.push_msg(video, Msg::Usage(u));
                }
                self.rescan_scenes(video);
                // Edits that sent no file event (shell commands) show up in the stat.
                let mut changed = r.changed;
                if let Some(dir) = self.projects[video.0].videos[video.1].dir.clone() {
                    for p in changed_between(&r.before, &snapshot_scripts(&dir)) {
                        if !changed.contains(&p) {
                            changed.push(p);
                        }
                    }
                }
                self.check_scenes(video, r.kind, changed, cx);
            }
        }
        cx.notify();
    }

    /// Re-pair storyboard beats with the scene files after the assistant wrote some.
    fn rescan_scenes(&mut self, video: (usize, usize)) {
        let v = &mut self.projects[video.0].videos[video.1];
        let Some(dir) = v.dir.clone() else { return };
        let notes: Vec<Option<String>> = v.scenes.iter().map(|s| s.note.clone()).collect();
        let mut fresh = store::scenes_for(&dir, v.storyboard.as_ref(), &notes);
        // Keep what the engine already knows about unchanged scenes.
        let leftover = carry_engine(&mut fresh, std::mem::take(&mut v.scenes));
        v.scenes = fresh;
        self.stage.dead.extend(leftover);
        let v = &self.projects[video.0].videos[video.1];
        if video == self.current && self.scene >= v.scenes.len() {
            self.scene = v.scenes.len().saturating_sub(1);
        }
    }

    /// Run the engine check on the scenes a turn touched (all scenes after a build or
    /// a change to lib/), then repair failures up to the configured number of tries.
    fn check_scenes(&mut self, video: (usize, usize), kind: TurnKind, changed: Vec<PathBuf>, cx: &mut Context<Self>) {
        let v = &self.projects[video.0].videos[video.1];
        let Some(dir) = v.dir.clone() else {
            if let Some(v) = self.video_mut(video) {
                v.busy = None;
            }
            return;
        };
        let scenes_dir = dir.join("scenes");
        let all = matches!(kind, TurnKind::Build) || changed.iter().any(|p| !p.starts_with(&scenes_dir));
        // Canonicalized once, not once per (scene x changed file).
        let changed_canon: Vec<Option<PathBuf>> = if all { Vec::new() } else { changed.iter().map(|c| c.canonicalize().ok()).collect() };
        let targets: Vec<(usize, OpenScene)> = v
            .scenes
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.engine.file.as_ref().is_some_and(|f| all || {
                    let f = f.canonicalize().ok();
                    changed_canon.contains(&f)
                })
            })
            .filter_map(|(i, s)| {
                Some((
                    i,
                    OpenScene {
                        path: s.engine.file.clone()?,
                        root: dir.clone(),
                        theme_json: studio_render::style::json(self.video_style(video)).to_string(),
                        scene: None,
                        globals: vec![],
                        narration: s.engine.voice.as_ref().map(|v| v.seconds as f64),
                        narration_lines: s.engine.voice.as_ref().map(crate::model::VoiceLine::lines).unwrap_or_default(),
                        narration_cues: s.engine.voice.as_ref().map(|v| v.cues.clone()).unwrap_or_default(),
                    },
                ))
            })
            .collect();
        if targets.is_empty() {
            // Nothing changed on disk (a reply with no edits): leave the stage and the
            // thumbnails alone.
            self.finish_assistant_work_with(video, kind, true, !changed.is_empty(), cx);
            return;
        }
        self.set_working(video, Some(format!("Checking {} chapter{}", targets.len(), if targets.len() == 1 { "" } else { "s" })));
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    targets
                        .into_iter()
                        .map(|(i, req)| {
                            let started = Instant::now();
                            let r = studio_render::preview::check(&req).map_err(|e| format!("{e:#}"));
                            (i, req.path, r, started.elapsed().as_secs_f32())
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| this.on_checked(video, kind, results, cx)).ok();
        })
        .detach();
    }

    fn on_checked(
        &mut self,
        video: (usize, usize),
        kind: TurnKind,
        results: Vec<(usize, PathBuf, Result<studio_render::TimelineInfo, String>, f32)>,
        cx: &mut Context<Self>,
    ) {
        let mut failures: Vec<(usize, PathBuf, String, Option<u32>)> = Vec::new();
        for (i, path, r, _secs) in &results {
            let issue = match r {
                Ok(t) if t.problems.is_empty() => None,
                Ok(t) => {
                    let p = &t.problems[0];
                    let found: Vec<(&str, f64)> = t.problems.iter().map(|p| (p.message.as_str(), p.time)).collect();
                    Some((prompt::problems(&found), None, Issue::Layout { count: t.problems.len(), first: p.message.clone() }))
                }
                Err(e) => {
                    let short = studio_render::short_error(e);
                    let line = studio_render::error_line(e);
                    Some((short.clone(), line, Issue::Error { message: short, line }))
                }
            };
            if let Some(s) = self.video_mut(video).and_then(|v| v.scenes.get_mut(*i)) {
                s.engine.checked = true;
                s.engine.issue = issue.as_ref().map(|x| x.2.clone());
                if let Ok(t) = r {
                    s.engine.duration = Some(t.duration as f32);
                }
            }
            if let Some((msg, line, _)) = issue {
                failures.push((*i, path.clone(), msg, line));
            }
        }
        // Repair every failing scene in one turn, each with its diagnostic and the lines involved.
        let tries = self.prefs.repair_tries;
        let attempt = match kind {
            TurnKind::Repair { k, .. } => k.saturating_add(1),
            _ => 1,
        };
        if let Some(&(scene, ..)) = failures.first() {
            if attempt <= tries {
                let dir = self.projects[video.0].videos[video.1].dir.clone().unwrap_or_default();
                let list: Vec<prompt::Failure> = failures
                    .iter()
                    .map(|(_, path, msg, line)| prompt::Failure {
                        file: path.strip_prefix(&dir).unwrap_or(path).to_string_lossy().replace('\\', "/"),
                        diagnostic: msg.clone(),
                        excerpt: prompt::repair_excerpt(path, *line).or_else(|| prompt::layout_excerpt(path, msg)),
                    })
                    .collect();
                let what = if list.len() == 1 { format!("chapter {}", scene + 1) } else { format!("{} chapters", list.len()) };
                let base = self.prefs.build_reasoning.effort(Some(Effort::High));
                let effort = if self.prefs.escalate_repairs && attempt > 1 {
                    Some(base.unwrap_or(Effort::Medium).raise())
                } else {
                    base
                };
                if let Some(v) = self.video_mut(video) {
                    v.busy = Some(Busy::Fixing { scene: scene + 1, k: attempt, of: tries });
                }
                self.set_working(video, Some(format!("Fixing {what} · try {attempt} of {tries}")));
                self.start_turn(video, TurnKind::Repair { scene, k: attempt, build: kind.is_build() }, prompt::repair_all(&list), false, effort, cx);
                return;
            }
            // Out of tries: one plain sentence instead of a row per check.
            let names: Vec<String> = failures.iter().map(|(i, ..)| (i + 1).to_string()).collect();
            let text = match names.as_slice() {
                [one] => format!("Chapter {one} still has a problem: {}. Open Code to see it, or describe the fix.", failures[0].2),
                many => format!(
                    "Chapters {} and {} still have problems. Open Code to see them, or describe the fix.",
                    many[..many.len() - 1].join(", "),
                    many[many.len() - 1]
                ),
            };
            self.push_msg(video, Msg::Activity { icon: IconName::Info, ok: false, text, meta: String::new() });
            self.finish_assistant_work(video, kind, false, cx);
            return;
        }
        // A video that was just created gets a look-over before it is called done.
        if self.prefs.visual_review && kind.is_build() && !self.reviewed.contains(&video) {
            return self.review_look(video, true, cx);
        }
        self.finish_assistant_work(video, kind, true, cx);
    }

    /// The visual review: draw frames of each chapter, then ask the assistant to look at
    /// them and fix what is wrong (overlapping or cut-off text, crowding). `build`: it
    /// follows the creation of the video.
    pub fn review_look(&mut self, video: (usize, usize), build: bool, cx: &mut Context<Self>) {
        let kind = TurnKind::Review { build };
        let v = &self.projects[video.0].videos[video.1];
        let Some(dir) = v.dir.clone() else { return self.finish_assistant_work(video, kind, true, cx) };
        let Some(ffmpeg) = studio_render::ffmpeg_path() else {
            // Pictures need ffmpeg; without it the video is simply not looked over.
            return self.finish_assistant_work(video, kind, true, cx);
        };
        let theme_json = studio_render::style::json(self.video_style(video)).to_string();
        let jobs: Vec<(usize, PathBuf, OpenScene)> = v
            .scenes
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let file = s.engine.file.clone()?;
                let req = OpenScene {
                    path: file.clone(),
                    root: dir.clone(),
                    theme_json: theme_json.clone(),
                    scene: None,
                    globals: vec![],
                    narration: s.engine.voice.as_ref().map(|v| v.seconds as f64),
                    narration_lines: s.engine.voice.as_ref().map(crate::model::VoiceLine::lines).unwrap_or_default(),
                    narration_cues: s.engine.voice.as_ref().map(|v| v.cues.clone()).unwrap_or_default(),
                };
                Some((i, file, req))
            })
            .collect();
        if jobs.is_empty() {
            return self.finish_assistant_work(video, kind, true, cx);
        }
        self.reviewed.insert(video);
        if let Some(v) = self.video_mut(video) {
            v.busy = Some(Busy::Reviewing);
        }
        self.set_working(video, Some("Checking how it looks".into()));
        // The video is made: show it while it is looked over.
        if video == self.current && build {
            self.mode = Mode::Studio;
            self.scene = 0;
            self.stage.open = None;
            self.sync_preview();
        }
        if build {
            self.refresh_thumbs_for(video);
        }
        cx.notify();
        let frames_dir = dir.join(".cache").join("review");
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let dir2 = dir.clone();
            let made = cx
                .background_executor()
                .spawn(async move {
                    let _ = std::fs::remove_dir_all(&frames_dir);
                    std::fs::create_dir_all(&frames_dir).map_err(|e| format!("{e}"))?;
                    let mut out: Vec<prompt::ReviewFrame> = Vec::new();
                    for (i, file, req) in jobs {
                        // A chapter that can't be drawn is the repair loop's business, not the review's.
                        let Ok(frames) = studio_render::preview::review_frames(&req, studio_render::Output::new(1280, 720, 30.0)) else { continue };
                        for (k, (at, frame)) in frames.into_iter().enumerate() {
                            let img = frames_dir.join(format!("{:02}-{}.jpg", i + 1, k + 1));
                            studio_render::slides::frame_to_jpeg(&ffmpeg, &frame, &img).map_err(|e| format!("{e:#}"))?;
                            let rel = |p: &Path| p.strip_prefix(&dir2).unwrap_or(p).to_string_lossy().replace('\\', "/");
                            out.push(prompt::ReviewFrame { image: rel(&img), file: rel(&file), chapter: i + 1, at });
                        }
                    }
                    Ok::<_, String>(out)
                })
                .await;
            this.update(cx, |this, cx| this.on_review_frames(video, build, made, cx)).ok();
        })
        .detach();
    }

    fn on_review_frames(&mut self, video: (usize, usize), build: bool, made: Result<Vec<prompt::ReviewFrame>, String>, cx: &mut Context<Self>) {
        let kind = TurnKind::Review { build };
        // Stopped, or the app moved on, while the pictures were drawn.
        if !matches!(self.projects[video.0].videos[video.1].busy, Some(Busy::Reviewing)) {
            return;
        }
        let frames = match made {
            Ok(f) if !f.is_empty() => f,
            // Nothing to look at, or the pictures could not be made: the video is still fine.
            _ => return self.finish_assistant_work(video, kind, true, cx),
        };
        let dir = self.projects[video.0].videos[video.1].dir.clone().unwrap_or_default();
        let images: Vec<PathBuf> = frames.iter().map(|f| dir.join(&f.image)).collect();
        let effort = self.prefs.build_reasoning.effort(Some(Effort::Low));
        self.set_working(video, Some("Checking how it looks".into()));
        self.start_turn_with(video, kind, prompt::review(&frames), false, effort, images, cx);
    }

    fn finish_assistant_work(&mut self, video: (usize, usize), kind: TurnKind, ok: bool, cx: &mut Context<Self>) {
        self.finish_assistant_work_with(video, kind, ok, true, cx);
    }

    /// `reload`: the turn touched files, so the stage and the thumbnails are stale (a build
    /// always reloads).
    fn finish_assistant_work_with(&mut self, video: (usize, usize), kind: TurnKind, ok: bool, reload: bool, cx: &mut Context<Self>) {
        self.set_working(video, None);
        self.reviewed.remove(&video);
        let was_build = {
            let v = &mut self.projects[video.0].videos[video.1];
            // By what the turn was for, not the video's status: a render sets Building too.
            let build = kind.is_build();
            v.busy = None;
            v.updated = store::now();
            build
        };
        let has_scripts = self.projects[video.0].videos[video.1].scenes.iter().any(|s| s.engine.file.is_some());
        // A render of this video owns its status until it ends.
        if !self.render_active(video) {
            if has_scripts {
                self.projects[video.0].videos[video.1].status = if ok { VideoStatus::Ready } else { VideoStatus::Failed };
            } else {
                // A build that wrote no scenes isn't still building.
                self.reset_building(video, was_build);
            }
        }
        self.persist(video);
        let reload = reload || was_build;
        // The script on screen may have changed.
        self.probes.get_mut().clear();
        if video == self.current {
            if was_build && has_scripts {
                self.mode = Mode::Studio;
                self.scene = 0;
            }
            if reload {
                self.stage.open = None;
                self.sync_preview();
            }
        }
        if reload {
            self.refresh_thumbs_for(video);
        }
        if ok && self.prefs.auto_render && was_build && has_scripts {
            self.start_render_for(video, cx);
        }
        cx.notify();
    }

    pub fn persist(&self, video: (usize, usize)) {
        if let Some(v) = self.projects.get(video.0).and_then(|p| p.videos.get(video.1)) {
            store::save_video(v);
            store::save_chat(v);
        }
    }

    // ------------------------------------------------------------------ flows

    /// Home composer: create a video and plan its storyboard.
    pub fn start_plan(&mut self, request: String, cx: &mut Context<Self>) {
        let mut request = request.trim().to_string();
        // Documents alone are a request: plan the lesson from them.
        if request.is_empty() && self.attachments.is_empty() {
            return;
        }
        if request.is_empty() {
            request = "Make a lesson from the attached documents.".into();
        }
        // While detection is still running the turn waits for it (see `start_turn`).
        if self.pick_provider().is_none() && !self.checking_providers() {
            self.notice = Some("Connect an assistant first: Settings → Assistants & usage.".into());
            self.settings_page = crate::app::SettingsPage::Assistants;
            self.go(Screen::Settings, cx);
            return;
        }
        let pi = match self.home_target() {
            Some(pi) => pi,
            None => {
                let Some(p) = store::new_project("My videos") else {
                    self.notice = Some("Couldn't create the video folder.".into());
                    cx.notify();
                    return;
                };
                self.projects.push(p);
                self.projects.len() - 1
            }
        };
        self.home_project = None;
        let title: String = match self.attachments.first().and_then(|d| d.file_stem()) {
            Some(stem) if request == "Make a lesson from the attached documents." => stem.to_string_lossy().into_owned(),
            _ => request.split_whitespace().take(6).collect::<Vec<_>>().join(" "),
        };
        let Some(mut video) = store::new_video(&self.projects[pi], &title, &request) else {
            self.notice = Some("Couldn't create the video folder.".into());
            cx.notify();
            return;
        };
        video.busy = Some(Busy::Planning);
        video.settings.style = Some(self.prefs.house_style);
        let docs = std::mem::take(&mut self.attachments);
        if !docs.is_empty() {
            video.source = Some(
                docs.iter().map(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect::<Vec<_>>().join(", "),
            );
        }
        let dir = video.dir.clone();
        self.projects[pi].videos.push(video);
        self.projects[pi].expanded = true;
        let vi = self.projects[pi].videos.len() - 1;
        self.home_input.update(cx, |i, cx| i.clear(cx));
        self.open_video(pi, vi, cx);
        self.mode = Mode::Storyboard;
        let effort = self.prefs.planning_reasoning.effort(Some(Effort::Low));
        if docs.is_empty() {
            let p = self.plan_prompt(&request, &[], self.prefs.house_style);
            self.start_turn((pi, vi), TurnKind::Plan, p, true, effort, cx);
            return;
        }
        // Copy the documents into the video and read their text off the UI thread; the
        // plan condenses them into source notes that later turns reuse.
        self.set_working((pi, vi), Some(format!("Reading {} document{}", docs.len(), if docs.len() == 1 { "" } else { "s" })));
        let sources_dir = dir.as_ref().map(|d| d.join("sources"));
        let style = self.prefs.house_style;
        let asked = request.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let read = cx
                .background_executor()
                .spawn(async move {
                    let mut texts = Vec::new();
                    let mut errors = Vec::new();
                    // The documents' whole text: a teacher's script is read from it, not from the condensed part.
                    let mut full = Vec::new();
                    // Pictures next to the documents: a script's logo.
                    let images: Vec<PathBuf> = docs.iter().filter(|d| crate::teacher::is_image(d)).cloned().collect();
                    let docs: Vec<PathBuf> = docs.into_iter().filter(|d| !crate::teacher::is_image(d)).collect();
                    // About 24k characters across all documents keeps the plan turn small.
                    let budget = 24_000 / docs.len().max(1);
                    for d in &docs {
                        let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        if let Some(sd) = &sources_dir {
                            let _ = crate::attach::copy_into(d, sd);
                        }
                        match studio_docs::extract(d) {
                            Ok(t) if !t.trim().is_empty() => {
                                // The full text, once, for later turns to read on demand.
                                if let Some(sd) = &sources_dir {
                                    let stem = d.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "document".into());
                                    let _ = std::fs::write(sd.join(format!("{stem}.md")), format!("# {name}\n\n{}\n", t.trim()));
                                }
                                texts.push(format!("[{name}]\n{}", studio_docs::truncate(&t, budget)));
                                full.push(t);
                            }
                            Ok(_) => errors.push(format!("{name} has no readable text (a scanned PDF?)")),
                            Err(e) => errors.push(format!("{name}: {e:#}")),
                        }
                    }
                    // A teacher's own script is used as written, not summarised.
                    let script = crate::teacher::prepare(&asked, &full, &images, dir.as_deref(), style);
                    (texts, errors, script)
                })
                .await;
            this.update(cx, |this, cx| {
                let (texts, errors, script) = read;
                for e in errors {
                    this.push_msg((pi, vi), Msg::Activity { icon: IconName::Info, ok: false, text: e, meta: String::new() });
                }
                this.set_working((pi, vi), None);
                let p = match script {
                    Some(prepared) => {
                        if let Some(dir) = this.video_mut((pi, vi)).and_then(|v| v.dir.clone()) {
                            crate::teacher::save(&dir, &prepared.saved);
                        }
                        for (ok, text) in prepared.lines {
                            this.push_msg((pi, vi), Msg::Activity { icon: if ok { IconName::Check } else { IconName::Info }, ok, text, meta: String::new() });
                        }
                        prepared.saved.plan_prompt()
                    }
                    None => this.plan_prompt(&request, &texts, this.prefs.house_style),
                };
                this.start_turn((pi, vi), TurnKind::Plan, p, true, effort, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The storyboard-planning prompt for `request`, with the composer's options.
    fn plan_prompt(&self, request: &str, sources: &[String], style: usize) -> String {
        use crate::model::{APPROACHES, ENDINGS, LEVELS, PACES};
        prompt::plan(&PlanOptions {
            request,
            sources,
            target_seconds: self.prefs.video_seconds.clamp(crate::model::LENGTH_RANGE.0, crate::model::LENGTH_RANGE.1),
            style_name: studio_render::style::STYLES[style.min(studio_render::style::STYLES.len() - 1)].name,
            approach: APPROACHES[self.prefs.approach.min(APPROACHES.len() - 1)].guide,
            level: LEVELS[self.prefs.level.min(LEVELS.len() - 1)].guide,
            words_per_second: PACES[self.prefs.pace.min(PACES.len() - 1)].1,
            ending: ENDINGS[self.prefs.ending.min(ENDINGS.len() - 1)].guide,
        })
    }

    /// Plan the open video again after its first plan failed (same request, and the
    /// documents it was given, read back from `sources/`).
    pub fn retry_plan(&mut self, cx: &mut Context<Self>) {
        let video = self.current;
        let v = self.video();
        let (Some(request), None) = (v.request.clone(), v.storyboard.as_ref()) else { return };
        if self.busy(video) {
            return;
        }
        if self.pick_provider().is_none() && !self.checking_providers() {
            self.notice = Some("Connect an assistant first: Settings → Assistants & usage.".into());
            self.settings_page = crate::app::SettingsPage::Assistants;
            self.go(Screen::Settings, cx);
            return;
        }
        let style = v.settings.style.unwrap_or(self.prefs.house_style);
        // A teacher's script read before: plan from it again.
        if let Some(saved) = v.dir.as_deref().and_then(crate::teacher::load) {
            let p = saved.plan_prompt();
            if let Some(v) = self.video_mut(video) {
                v.busy = Some(Busy::Planning);
            }
            let effort = self.prefs.planning_reasoning.effort(Some(Effort::Low));
            self.start_turn(video, TurnKind::Plan, p, true, effort, cx);
            cx.notify();
            return;
        }
        let mut texts: Vec<String> = v
            .dir
            .as_ref()
            .and_then(|d| std::fs::read_dir(d.join("sources")).ok())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .collect();
        let budget = 24_000 / texts.len().max(1);
        for t in &mut texts {
            *t = studio_docs::truncate(t, budget);
        }
        if let Some(v) = self.video_mut(video) {
            v.busy = Some(Busy::Planning);
        }
        let effort = self.prefs.planning_reasoning.effort(Some(Effort::Low));
        let p = self.plan_prompt(&request, &texts, style);
        self.start_turn(video, TurnKind::Plan, p, true, effort, cx);
        cx.notify();
    }

    /// The last error the assistant reported for `video`, if its latest message is one.
    pub fn last_error(&self, video: &crate::model::Video) -> Option<String> {
        video.chat.iter().rev().find_map(|m| match m {
            Msg::Activity { ok: false, text, .. } => Some(text.clone()),
            _ => None,
        })
    }

    /// The latest failure in `video`'s chat, if the assistant failed after the last thing
    /// the teacher asked.
    pub fn last_problem(video: &crate::model::Video) -> Option<(usize, Provider, String)> {
        for (i, m) in video.chat.iter().enumerate().rev() {
            match m {
                Msg::Problem { provider, raw } => return Some((i, *provider, raw.clone())),
                Msg::User(_) => return None,
                _ => {}
            }
        }
        None
    }

    /// Storyboard revise box. True when the note was taken (the box can be cleared); false
    /// when the assistant is busy or there is nothing to revise, so the text must stay.
    pub fn revise_storyboard(&mut self, note: String, cx: &mut Context<Self>) -> bool {
        let video = self.current;
        let Some(sb) = self.video().storyboard.clone() else { return false };
        if note.trim().is_empty() || self.busy(video) {
            return false;
        }
        if let Some(v) = self.video_mut(video) {
            v.busy = Some(Busy::Revising);
        }
        let effort = self.prefs.planning_reasoning.effort(Some(Effort::Low));
        let dir = self.video().dir.clone();
        let prompt = match dir.as_deref().and_then(|d| Some((d, crate::teacher::load(d)?))) {
            Some((dir, mut saved)) => {
                // Notes the teacher asks for now are kept for the rest of the video.
                if saved.ask_for(&note) {
                    crate::teacher::save(dir, &saved);
                }
                studio_agent::teacher::revise(&saved.script, &sb, &note)
            }
            None => prompt::revise(&sb, &note),
        };
        self.start_turn(video, TurnKind::Revise, prompt, true, effort, cx);
        true
    }

    /// Approve & build: write the scenes for the storyboard.
    pub fn build_video(&mut self, cx: &mut Context<Self>) {
        let video = self.current;
        let Some(sb) = self.video().storyboard.clone() else {
            // Sample and older videos are already built.
            self.mode = Mode::Studio;
            self.sync_preview();
            cx.notify();
            return;
        };
        if self.busy(video) {
            return;
        }
        if let Some(v) = self.video_mut(video) {
            v.busy = Some(Busy::Building);
            v.status = VideoStatus::Building;
        }
        self.mode = Mode::Studio;
        self.right_tab = crate::app::RightTab::Assistant;
        let n = sb.scenes.len();
        self.push_msg(video, Msg::User(format!("Create the video from the script ({n} chapters).")));
        self.set_working_progress(video, Some("Creating chapters".into()), Some((0, n)));
        let effort = self.prefs.build_reasoning.effort(self.auto_build_effort());
        self.start_turn(video, TurnKind::Build, prompt::build(&sb), false, effort, cx);
    }

    /// The Studio chat. True when the message was taken (the box can be cleared); false
    /// when the assistant is busy (or the text is empty), so what was typed stays.
    pub fn send_chat(&mut self, text: String, cx: &mut Context<Self>) -> bool {
        let video = self.current;
        let text = text.trim().to_string();
        if text.is_empty() || self.busy(video) {
            return false;
        }
        if let Some(v) = self.video_mut(video) { v.chat.retain(|m| !matches!(m, Msg::Usage(_))) }
        self.push_msg(video, Msg::User(text.clone()));
        let v = self.video();
        if v.dir.is_none() {
            self.fail_early(video, true, "This video has no folder yet.".into(), cx);
            return true;
        }
        let scope = v.scenes.get(self.scene).filter(|_| !self.edit_whole).and_then(|s| {
            let f = s.engine.file.as_ref()?;
            let rel = f.strip_prefix(v.dir.as_ref()?).ok()?.to_string_lossy().replace('\\', "/");
            Some((rel, format!("Scene {} · {}", self.scene + 1, s.title), s.say.clone()))
        });
        let assets = self.asset_paths(video);
        let p = prompt::edit(&text, scope.as_ref().map(|(f, t, say)| (f.as_str(), t.as_str(), say.as_str())), &assets);
        if let Some(v) = self.video_mut(video) {
            v.busy = Some(Busy::Editing);
        }
        self.set_working(video, Some("Thinking".into()));
        let effort = self.prefs.build_reasoning.effort(self.auto_build_effort());
        self.start_turn(video, TurnKind::Edit, p, false, effort, cx);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::carry_engine;
    use crate::model::{Scene, VoiceLine};
    use studio_agent::prompt::Beat;

    fn scene(say: &str, file: Option<&str>) -> Scene {
        let beat = Beat { title: "t".into(), say: say.into(), visual: String::new(), show: String::new(), seconds: 5 };
        let mut s = Scene::from_beat(&beat);
        s.engine.file = file.map(Into::into);
        s
    }

    fn voice() -> VoiceLine {
        VoiceLine { wav: "track.wav".into(), seconds: 3.0, text: "Hello.".into(), parts: Vec::new(), starts: Vec::new(), cues: Vec::new() }
    }

    // A build gives storyboard scenes (no script yet, narration already spoken) their files:
    // the narration must survive, or the render comes out silent.
    #[test]
    fn build_keeps_narration_spoken_for_the_storyboard() {
        let mut planned = scene("Hello.", None);
        planned.engine.voice = Some(voice());
        let mut built = [scene("Hello.", Some("scenes/01.ts")), scene("Changed.", Some("scenes/02.ts"))];
        let mut other = scene("Before.", None);
        other.engine.voice = Some(voice());
        carry_engine(&mut built, vec![planned, other]);
        assert_eq!(built[0].engine.voice, Some(voice()));
        assert_eq!(built[1].engine.voice, None, "a changed line is spoken again");
    }
}
