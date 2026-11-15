//! `antigravity-usage-bridge`: the statusline command and `PreInvocation`
//! low-quota guard hook that Codex Usage Overlay registers with Antigravity CLI.

use agent_bridge::antigravity as shared;
use agent_bridge::{read_json, read_stdin, record_block, run_chained, unix_now, BridgeConfig, GuardDecision, ShellMode};
use serde_json::{json, Value};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["statusline"] => statusline(),
        ["hook", "invocation"] => hook_invocation(),
        _ => {
            eprintln!("usage: antigravity-usage-bridge statusline | hook invocation");
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
    if let Some(next) = shared::parse_statusline(&input, now) {
        if shared::should_write(stored.as_ref(), &next) {
            if let Err(error) = shared::write_json_atomic(&dir.join(shared::SNAPSHOT_FILE), &next) {
                eprintln!("Could not store the Antigravity rate limits: {error}");
            }
        }
    }

    let config: BridgeConfig = read_json(&dir.join(shared::BRIDGE_FILE)).unwrap_or_default();
    if let Some(output) = config
        .chained_command()
        .and_then(|command| run_chained(command, &raw, ShellMode::CmdUnlessBashSyntax))
    {
        let _ = std::io::stdout().write_all(&output);
    }
}

fn hook_invocation() {
    let raw = read_stdin();
    let input: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let now = unix_now();
    let dir = shared::runtime_dir();
    let config: Option<shared::GuardConfig> = read_json(&dir.join(shared::GUARD_FILE));
    let mut snapshot = shared::read_snapshot(&dir);
    if let Some(snap) = snapshot.as_mut() {
        if snap.apply_hook_model(&input) {
            let _ = shared::write_json_atomic(&dir.join(shared::SNAPSHOT_FILE), snap);
        }
    }
    let GuardDecision::Block(reason) = shared::guard_decision(config.as_ref(), snapshot.as_ref(), now) else {
        println!("{{}}");
        return;
    };
    record_block(&dir, shared::GUARD_STATE_FILE, now);
    let output = json!({
        "injectSteps": [
            {
                "errorMessage": {
                    "userMessage": reason
                }
            }
        ],
        "terminationBehavior": "terminate"
    });
    println!("{output}");
}
