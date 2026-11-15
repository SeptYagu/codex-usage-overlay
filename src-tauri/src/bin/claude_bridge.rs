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
        ["renew", "plugin-list", cli] => std::process::exit(renew(cli, &["plugin", "list"])),
        ["renew", "doctor", cli] => std::process::exit(renew(cli, &["doctor"])),
        _ => {
            eprintln!("usage: claude-usage-bridge statusline | hook prompt | hook tool");
            std::process::exit(1);
        }
    }
}

/// Run one bounded login-renewal command inside a kill-on-close Windows job.
/// The job is created before the CLI starts, so a .cmd shim and every process
/// it spawns belong to this worker rather than accumulating under the overlay.
#[cfg(windows)]
fn renew(cli: &str, args: &[&str]) -> i32 {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    use windows::core::PCWSTR;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    const RENEW_TIMEOUT: Duration = Duration::from_secs(30);
    let job = match unsafe { CreateJobObjectW(None, PCWSTR::null()) } {
        Ok(job) => job,
        Err(_) => return 1,
    };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&limits) as u32,
        )
    };
    let assigned = configured.and_then(|_| unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) });
    if assigned.is_err() {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(job) };
        return 1; // Do not start an uncontained Claude process.
    }
    // Keep the only job handle open until process exit. Windows then kills any
    // CLI descendants even if the immediate child already exited.
    let mut command = Command::new(cli);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(home) = dirs::home_dir() {
        command.current_dir(home); // doctor reads project settings in cwd
    }
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let result = match command.spawn() {
        Ok(mut child) => {
            let started = Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => break if status.success() { 0 } else { 1 },
                    Ok(None) if started.elapsed() < RENEW_TIMEOUT => std::thread::sleep(Duration::from_millis(25)),
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break 1;
                    }
                }
            }
        }
        Err(_) => 1,
    };
    // main calls process::exit, which closes `job`. Do not close it here:
    // doing so would also terminate this worker while it is returning.
    result
}

#[cfg(not(windows))]
fn renew(_cli: &str, _args: &[&str]) -> i32 { 1 }

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
