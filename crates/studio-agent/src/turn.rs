//! One assistant turn: start the CLI in the video folder, feed the prompt on stdin, and
//! stream parsed events until it exits. Exactly one `Done` is always delivered last.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use crate::events::{AgentEvent, Tokens};
use crate::process::{ProcessTree, own_group, quiet, tie_to_app};
use crate::{Effort, Provider, claude, codex};

/// File tools for building and editing scenes.
pub const EDIT_TOOLS: &[&str] = &["Read", "Edit", "Write", "Glob", "Grep"];

#[derive(Clone, Debug)]
pub struct TurnRequest {
    pub provider: Provider,
    pub exe: PathBuf,
    /// The video folder; the CLI may only touch files in here.
    pub cwd: PathBuf,
    /// System prompt file (Claude: `--system-prompt-file`; Codex reads `AGENTS.md` in `cwd`).
    pub system_file: PathBuf,
    pub prompt: String,
    pub model: Option<&'static str>,
    pub effort: Option<Effort>,
    /// Continue this conversation (Claude session id / Codex thread id).
    pub resume: Option<String>,
    /// Allowed tools; empty means answer only (planning).
    pub tools: Vec<&'static str>,
    /// Don't keep the conversation (one-off planning calls).
    pub ephemeral: bool,
}

/// Lock a mutex even if a panicking thread poisoned it (the data is plain state).
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A running turn.
pub struct Turn {
    pub events: async_channel::Receiver<AgentEvent>,
    child: Arc<Mutex<Option<Child>>>,
    tree: Arc<ProcessTree>,
    cancelled: std::sync::atomic::AtomicBool,
    id: u64,
}

impl Turn {
    /// A process-unique, stable id for this turn (never reused), so the app can route
    /// events and cancellation to the right turn.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Stop the turn: kills the CLI and everything it started (npm shims leave node
    /// running otherwise).
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.tree.kill();
        if let Some(c) = lock(&self.child).as_mut() {
            let _ = c.kill();
        }
    }
}

impl Turn {
    /// [`Turn::cancel`] was called (the turn didn't fail on its own).
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn start(req: TurnRequest) -> Result<Turn> {
    let mut cmd = match req.provider {
        Provider::Claude => claude::command(&req.exe, &req),
        Provider::Codex => codex::command(&req.exe, &req),
    };
    cmd.current_dir(&req.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    own_group(quiet(&mut cmd));
    let mut child = cmd.spawn().with_context(|| format!("starting {}", req.exe.display()))?;
    tie_to_app(&child);
    let tree = Arc::new(ProcessTree::attach(&child));
    let mut stdin = child.stdin.take().context("no stdin")?;
    let stdout = child.stdout.take().context("no stdout")?;
    let mut stderr = child.stderr.take().context("no stderr")?;
    let prompt = req.prompt.clone();
    std::thread::spawn(move || {
        let _ = stdin.write_all(prompt.as_bytes());
    });
    // stderr arrives through a channel so a stray descendant holding the pipe open
    // can't block the final `Done` (see the bounded wait after the process exits).
    let (err_tx, err_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        let _ = err_tx.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    let child = Arc::new(Mutex::new(Some(child)));
    // The direct child is waited on by its own thread, so a descendant that keeps stdout
    // open can't stop the turn from noticing the CLI exited: the watcher then kills the
    // tree, which closes the pipe and ends the read loop below. It polls under the lock
    // (instead of blocking in `wait`) so `cancel` can still kill the child.
    let (status_tx, status_rx) = std::sync::mpsc::channel::<Option<std::process::ExitStatus>>();
    {
        let c = child.clone();
        let t = tree.clone();
        std::thread::spawn(move || {
            let status = loop {
                {
                    let mut guard = lock(&c);
                    match guard.as_mut().map(|ch| ch.try_wait()) {
                        None => break None,
                        Some(Ok(Some(st))) => {
                            guard.take();
                            break Some(st);
                        }
                        Some(Ok(None)) => {}
                        Some(Err(_)) => {
                            guard.take();
                            break None;
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
            };
            // The CLI is gone: end any descendants still holding the pipes. Kill before
            // marking the leader reaped, since on Unix `kill` is a no-op afterwards. The
            // group id can't be handed to a new process while group members remain.
            t.kill();
            t.reaped();
            let _ = status_tx.send(status);
        });
    }
    let (tx, events) = async_channel::unbounded();
    let provider = req.provider;
    let c = child.clone();
    let t = tree.clone();
    std::thread::spawn(move || {
        let parse = match provider {
            Provider::Claude => claude::parse,
            Provider::Codex => codex::parse,
        };
        let mut session: Option<String> = None;
        let mut done: Option<AgentEvent> = None;
        // A panic while parsing must not leave the app waiting for a `Done` forever.
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(_) => {
                        // Stop reading but don't leave the CLI blocked on a full pipe.
                        t.kill();
                        if let Some(ch) = lock(&c).as_mut() {
                            let _ = ch.kill();
                        }
                        break;
                    }
                }
                let line = String::from_utf8_lossy(&buf);
                for ev in parse(line.trim_end_matches(['\r', '\n'])) {
                    match ev {
                        AgentEvent::Started { session: ref s } => {
                            session = Some(s.clone());
                            let _ = tx.send_blocking(ev);
                        }
                        // Hold `Done` until the process exits, so it is always last.
                        AgentEvent::Done { .. } => done = Some(ev),
                        ev => {
                            let _ = tx.send_blocking(ev);
                        }
                    }
                }
            }
        }));
        if read.is_err() {
            t.kill();
            if let Some(ch) = lock(&c).as_mut() {
                let _ = ch.kill();
            }
            done = Some(AgentEvent::Done {
                session: None,
                tokens: Tokens::default(),
                cost_usd: None,
                error: Some("couldn't read the assistant's output".into()),
            });
        }
        let status = status_rx.recv().ok().flatten();
        // stderr hits EOF once every holder exits; don't wait forever if one escaped.
        let err = err_rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap_or_default();
        let ok = matches!(status, Some(s) if s.success());
        let final_ev = match done {
            Some(AgentEvent::Done { session: s, tokens, cost_usd, error }) => {
                AgentEvent::Done { session: s.or(session), tokens, cost_usd, error }
            }
            _ => {
                let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
                let error = if last.is_empty() {
                    if ok { "the assistant ended without an answer".to_string() } else { "the assistant stopped".to_string() }
                } else {
                    last
                };
                AgentEvent::Done { session, tokens: Tokens::default(), cost_usd: None, error: Some(error) }
            }
        };
        let _ = tx.send_blocking(final_ev);
    });
    Ok(Turn { events, child, tree, cancelled: Default::default(), id: NEXT_ID.fetch_add(1, Ordering::Relaxed) })
}

/// Run a turn to the end and collect its text (for planning calls).
pub fn run_to_end(req: TurnRequest) -> Result<(String, AgentEvent)> {
    let turn = start(req)?;
    let mut text = String::new();
    while let Ok(ev) = turn.events.recv_blocking() {
        match ev {
            AgentEvent::Text(t) => {
                text.push_str(&t);
                text.push('\n');
            }
            done @ AgentEvent::Done { .. } => return Ok((text, done)),
            _ => {}
        }
    }
    anyhow::bail!("the assistant stopped")
}
