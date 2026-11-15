//! `claude-usage-bridge`: the statusline command and low-quota guard hook that
//! Codex Usage Overlay registers with Claude Code.

use agent_bridge::claude as shared;
use agent_bridge::{read_json, read_stdin, record_block, run_chained, unix_now, BridgeConfig, GuardDecision, ShellMode};
use serde_json::{json, Value};
use std::io::Write;

#[derive(Clone, Copy)]
enum Hook {
    Prompt,
    Tool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["statusline"] => statusline(),
        ["hook", "prompt"] => hook(Hook::Prompt),
        ["hook", "tool"] => hook(Hook::Tool),
        _ => {
            eprintln!("usage: claude-usage-bridge statusline | hook prompt | hook tool");
            std::process::exit(1);
        }
    }
}

fn statusline() {
    let raw = read_stdin();
    let input: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let now = unix_now();
    let dir = shared::runtime_dir();
    let stored = shared::read_snapshot(&dir);
    let snapshot = match shared::parse_statusline(&input, now) {
        Some(mut next) => {
            next.account_id = shared::account_identity();
            if shared::should_write(stored.as_ref(), &next) {
                if let Err(error) = shared::write_json_atomic(&dir.join(shared::SNAPSHOT_FILE), &next) {
                    eprintln!("Could not store the Claude rate limits: {error}");
                }
            }
            Some(next)
        }
        None => stored,
    };

    let config: BridgeConfig = read_json(&dir.join(shared::BRIDGE_FILE)).unwrap_or_default();
    if let Some(output) = config
        .chained_command()
        .and_then(|command| run_chained(command, &raw, ShellMode::PreferGitBash))
    {
        let _ = std::io::stdout().write_all(&output);
        return;
    }
    println!("{}", default_line(&input, snapshot.as_ref(), now));
}

fn default_line(input: &Value, snapshot: Option<&shared::Snapshot>, now: i64) -> String {
    let mut parts = Vec::new();
    if let Some(model) = input.pointer("/model/display_name").and_then(Value::as_str) {
        parts.push(model.to_string());
    }
    if let Some(snapshot) = snapshot {
        for (label, window) in [("5h", snapshot.five_hour), ("7d", snapshot.seven_day)] {
            if let Some(window) = window {
                parts.push(format!("{label} {}%", window.at(now).remaining_percent()));
            }
        }
    }
    parts.join(" · ")
}

fn hook(kind: Hook) {
    let _ = read_stdin();
    let now = unix_now();
    let dir = shared::runtime_dir();
    let config: Option<shared::GuardConfig> = read_json(&dir.join(shared::GUARD_FILE));
    let snapshot = shared::read_snapshot(&dir);
    let GuardDecision::Block(reason) = shared::guard_decision(config.as_ref(), snapshot.as_ref(), now) else {
        return;
    };
    record_block(&dir, shared::GUARD_STATE_FILE, now);
    let output = match kind {
        Hook::Prompt => json!({ "decision": "block", "reason": reason }),
        Hook::Tool => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason
            }
        }),
    };
    println!("{output}");
}
