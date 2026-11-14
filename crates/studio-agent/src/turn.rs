//! One assistant turn: start the CLI in the video folder, feed the prompt on stdin, and
//! stream parsed events until it exits. Exactly one `Done` is always delivered last.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use crate::events::{AgentEvent, Tokens};
use crate::process::{quiet, tie_to_app};
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

/// A running turn.
pub struct Turn {
    pub events: async_channel::Receiver<AgentEvent>,
    child: Arc<Mutex<Option<Child>>>,
    cancelled: std::sync::atomic::AtomicBool,
}

impl Turn {
    pub fn cancel(&self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(c) = self.child.lock().unwrap().as_mut() {
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
    let mut child = quiet(&mut cmd).spawn().with_context(|| format!("starting {}", req.exe.display()))?;
    tie_to_app(&child);
    let mut stdin = child.stdin.take().context("no stdin")?;
    let stdout = child.stdout.take().context("no stdout")?;
    let mut stderr = child.stderr.take().context("no stderr")?;
    let prompt = req.prompt.clone();
    std::thread::spawn(move || {
        let _ = stdin.write_all(prompt.as_bytes());
    });
    let err_text = Arc::new(Mutex::new(String::new()));
    {
        let err_text = err_text.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            *err_text.lock().unwrap() = s;
        });
    }
    let child = Arc::new(Mutex::new(Some(child)));
    let (tx, events) = async_channel::unbounded();
    let provider = req.provider;
    let c = child.clone();
    std::thread::spawn(move || {
        let parse = match provider {
            Provider::Claude => claude::parse,
            Provider::Codex => codex::parse,
        };
        let mut session: Option<String> = None;
        let mut done: Option<AgentEvent> = None;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            for ev in parse(&line) {
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
        let taken = c.lock().unwrap().take();
        let status = taken.map(|mut ch| ch.wait());
        let ok = matches!(status, Some(Ok(s)) if s.success());
        let final_ev = match done {
            Some(AgentEvent::Done { session: s, tokens, cost_usd, error }) => {
                AgentEvent::Done { session: s.or(session), tokens, cost_usd, error }
            }
            _ => {
                let err = err_text.lock().unwrap().clone();
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
    Ok(Turn { events, child, cancelled: Default::default() })
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
