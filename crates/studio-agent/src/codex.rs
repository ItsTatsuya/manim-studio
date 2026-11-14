//! Codex CLI: `codex exec --json`, one JSON event per stdout line (`thread.started`,
//! `item.completed` with agent messages, file changes and commands, `turn.completed`
//! with token usage, `turn.failed` / `error`). Codex reads `AGENTS.md` in the working
//! folder as its instructions, so the system prompt is written there.
//!
//! The parser is deliberately tolerant: unknown events are ignored, and rate limits are
//! picked up from any `rate_limits` object in an event.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::events::{AgentEvent, RateLimits, Tokens};
use crate::turn::TurnRequest;

/// The command for one turn. The prompt goes to stdin (`-`).
pub fn command(exe: &Path, req: &TurnRequest) -> Command {
    let mut cmd = Command::new(exe);
    cmd.arg("exec");
    if let Some(id) = req.resume.as_ref().filter(|id| !id.starts_with('-')) {
        cmd.args(["resume", id]);
    }
    cmd.args(["--json", "--skip-git-repo-check"]);
    cmd.args(["--sandbox", if req.tools.is_empty() { "read-only" } else { "workspace-write" }]);
    if let Some(m) = req.model {
        cmd.args(["--model", m]);
    }
    if let Some(e) = req.effort {
        cmd.arg("-c").arg(format!("model_reasoning_effort=\"{}\"", e.codex()));
    }
    cmd.arg("-");
    cmd
}

fn find_rate_limits(v: &Value) -> Option<&Value> {
    match v {
        Value::Object(m) => m.get("rate_limits").or_else(|| m.values().find_map(find_rate_limits)),
        Value::Array(a) => a.iter().find_map(find_rate_limits),
        _ => None,
    }
}

pub fn parse(line: &str) -> Vec<AgentEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return vec![] };
    let mut out = Vec::new();
    if let Some(rl) = find_rate_limits(&v) {
        let pct = |k: &str| rl.get(k).and_then(|x| x.get("used_percent")).and_then(Value::as_f64).map(|p| (p / 100.0) as f32);
        out.push(AgentEvent::RateLimit(RateLimits {
            window: pct("primary"),
            week: pct("secondary"),
            window_resets_at: rl["primary"].get("resets_at").and_then(Value::as_u64),
            week_resets_at: rl["secondary"].get("resets_at").and_then(Value::as_u64),
        }));
    }
    match v.get("type").and_then(Value::as_str) {
        Some("thread.started") => {
            if let Some(id) = v.get("thread_id").and_then(Value::as_str) {
                out.push(AgentEvent::Started { session: id.to_string() });
            }
        }
        Some("item.completed") => {
            let item = &v["item"];
            match item.get("type").and_then(Value::as_str) {
                Some("agent_message") => {
                    let t = item["text"].as_str().unwrap_or_default().trim();
                    if !t.is_empty() {
                        out.push(AgentEvent::Text(t.to_string()));
                    }
                }
                Some("file_change") => {
                    for c in item["changes"].as_array().into_iter().flatten() {
                        if let Some(p) = c.get("path").and_then(Value::as_str) {
                            out.push(AgentEvent::FileChanged { path: p.to_string() });
                            out.push(AgentEvent::Tool { name: "Edit".into(), target: Some(p.to_string()) });
                        }
                    }
                }
                Some("command_execution") => {
                    out.push(AgentEvent::Tool { name: "Shell".into(), target: item["command"].as_str().map(str::to_string) });
                }
                _ => {}
            }
        }
        Some("turn.completed") => {
            let u = &v["usage"];
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            let cached = n("cached_input_tokens");
            out.push(AgentEvent::Done {
                session: None,
                tokens: Tokens { input: n("input_tokens").saturating_sub(cached), output: n("output_tokens"), cached, cache_write: 0 },
                cost_usd: None,
                error: None,
            });
        }
        Some("turn.failed") | Some("error") => {
            let msg = v["error"]["message"].as_str().or_else(|| v["message"].as_str()).unwrap_or("Codex stopped");
            out.push(AgentEvent::Done { session: None, tokens: Tokens::default(), cost_usd: None, error: Some(msg.to_string()) });
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exec_json() {
        let lines = [
            r#"{"type":"thread.started","thread_id":"t1"}"#,
            r#"{"type":"item.completed","item":{"id":"i0","type":"file_change","changes":[{"path":"scenes/a.ts","kind":"update"}]}}"#,
            r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Done."}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":1200,"cached_input_tokens":1000,"output_tokens":80}}"#,
        ];
        let ev: Vec<AgentEvent> = lines.iter().flat_map(|l| parse(l)).collect();
        assert_eq!(ev[0], AgentEvent::Started { session: "t1".into() });
        assert_eq!(ev[1], AgentEvent::FileChanged { path: "scenes/a.ts".into() });
        assert_eq!(ev[3], AgentEvent::Text("Done.".into()));
        assert!(matches!(ev[4], AgentEvent::Done { tokens: Tokens { input: 200, output: 80, cached: 1000, .. }, .. }));
        let rl = parse(r#"{"type":"token_count","info":{"rate_limits":{"primary":{"used_percent":40.0},"secondary":{"used_percent":10.0}}}}"#);
        assert!(matches!(rl[0], AgentEvent::RateLimit(RateLimits { window: Some(w), .. }) if (w - 0.4).abs() < 1e-6));
    }
}
