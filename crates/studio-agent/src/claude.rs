//! Claude Code: `claude -p --output-format stream-json --verbose`, one JSON object per
//! stdout line (`system`/`init`, `rate_limit_event`, `assistant`, `user`, `result`).

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::events::{AgentEvent, RateLimits, Tokens};
use crate::turn::TurnRequest;

/// The command for one turn. The prompt goes to stdin.
pub fn command(exe: &Path, req: &TurnRequest) -> Command {
    let mut cmd = Command::new(exe);
    cmd.args(["-p", "--output-format", "stream-json", "--verbose"])
        // Lean context: our own short system prompt instead of the coding-agent default,
        // no MCP servers, no user hooks or plugins.
        .arg("--system-prompt-file")
        .arg(&req.system_file)
        .args(["--strict-mcp-config", "--setting-sources", "project"])
        // A video folder's own .claude/settings.json may not add hooks (the assistant can
        // write files there).
        .args(["--settings", r#"{"disableAllHooks":true}"#]);
    if let Some(m) = req.model {
        cmd.args(["--model", m]);
    }
    if let Some(e) = req.effort {
        cmd.args(["--effort", e.claude()]);
    }
    if req.tools.is_empty() {
        cmd.args(["--tools", ""]);
    } else {
        cmd.arg("--tools").arg(req.tools.join(","));
        cmd.args(["--permission-mode", "acceptEdits"]);
    }
    match &req.resume {
        Some(id) => {
            cmd.args(["--resume", id]);
        }
        None if req.ephemeral => {
            cmd.arg("--no-session-persistence");
        }
        None => {}
    }
    cmd
}

fn num(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

/// Parse one stdout line.
pub fn parse(line: &str) -> Vec<AgentEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return vec![] };
    let mut out = Vec::new();
    match v.get("type").and_then(Value::as_str) {
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
            if let Some(s) = v.get("session_id").and_then(Value::as_str) {
                out.push(AgentEvent::Started { session: s.to_string() });
            }
        }
        Some("rate_limit_event") => {
            let w = &v["rate_limit_info"]["unifiedWindows"];
            let util = |k: &str| w.get(k).and_then(|x| x.get("utilization")).and_then(Value::as_f64).map(|f| f as f32);
            out.push(AgentEvent::RateLimit(RateLimits {
                window: util("five_hour"),
                week: util("seven_day"),
                window_resets_at: w["five_hour"].get("resetsAt").and_then(Value::as_u64),
                week_resets_at: w["seven_day"].get("resetsAt").and_then(Value::as_u64),
            }));
        }
        Some("assistant") => {
            let u = &v["message"]["usage"];
            if let Some(id) = v["message"].get("id").and_then(Value::as_str) {
                out.push(AgentEvent::Usage {
                    message: id.to_string(),
                    tokens: Tokens {
                        input: num(u, "input_tokens") + num(u, "cache_creation_input_tokens"),
                        output: num(u, "output_tokens"),
                        cached: num(u, "cache_read_input_tokens"),
                        cache_write: num(u, "cache_creation_input_tokens"),
                    },
                });
            }
            for c in v["message"]["content"].as_array().into_iter().flatten() {
                match c.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let t = c["text"].as_str().unwrap_or_default().trim();
                        if !t.is_empty() {
                            out.push(AgentEvent::Text(t.to_string()));
                        }
                    }
                    Some("tool_use") => {
                        let name = c["name"].as_str().unwrap_or("tool").to_string();
                        let target = c["input"].get("file_path").and_then(Value::as_str).map(str::to_string);
                        if let (true, Some(path)) = (matches!(name.as_str(), "Write" | "Edit" | "MultiEdit"), &target) {
                            out.push(AgentEvent::FileChanged { path: path.clone() });
                        }
                        out.push(AgentEvent::Tool { name, target });
                    }
                    _ => {}
                }
            }
        }
        Some("result") => {
            let u = &v["usage"];
            let tokens = Tokens {
                input: num(u, "input_tokens") + num(u, "cache_creation_input_tokens"),
                output: num(u, "output_tokens"),
                cached: num(u, "cache_read_input_tokens"),
                cache_write: num(u, "cache_creation_input_tokens"),
            };
            let failed = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            let error = failed.then(|| {
                v.get("result")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .or_else(|| v.get("subtype").and_then(Value::as_str))
                    .unwrap_or("the assistant stopped")
                    .to_string()
            });
            out.push(AgentEvent::Done {
                session: v.get("session_id").and_then(Value::as_str).map(str::to_string),
                tokens,
                cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
                error,
            });
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_turn() {
        let lines = [
            r#"{"type":"system","subtype":"init","session_id":"s1","tools":[]}"#,
            r#"{"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"five_hour":{"utilization":0.58,"resetsAt":1794545400},"seven_day":{"utilization":0.47}}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"C:\\v\\scenes\\a.ts","content":"x"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"done"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"session_id":"s1","total_cost_usd":0.001,"usage":{"input_tokens":4,"cache_creation_input_tokens":2983,"cache_read_input_tokens":2693,"output_tokens":156},"result":"done"}"#,
        ];
        let ev: Vec<AgentEvent> = lines.iter().flat_map(|l| parse(l)).collect();
        assert_eq!(ev[0], AgentEvent::Started { session: "s1".into() });
        assert!(matches!(ev[1], AgentEvent::RateLimit(RateLimits { window: Some(w), .. }) if (w - 0.58).abs() < 1e-6));
        let ev: Vec<AgentEvent> = ev.into_iter().filter(|e| !matches!(e, AgentEvent::Usage { .. })).collect();
        assert_eq!(ev[2], AgentEvent::FileChanged { path: "C:\\v\\scenes\\a.ts".into() });
        assert!(matches!(&ev[3], AgentEvent::Tool { name, .. } if name == "Write"));
        assert_eq!(ev[4], AgentEvent::Text("done".into()));
        match &ev[5] {
            AgentEvent::Done { tokens, error: None, .. } => assert_eq!(*tokens, Tokens { input: 2987, output: 156, cached: 2693, cache_write: 2983 }),
            e => panic!("{e:?}"),
        }
    }
}
