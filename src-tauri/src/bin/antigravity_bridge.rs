//! `antigravity-usage-bridge`: the statusline command and `PreInvocation`
//! low-quota guard hook that Codex Usage Overlay registers with Antigravity CLI.
//!
//! `statusline` stores all Antigravity quota buckets, active model metadata,
//! and optional AI Credits from stdin into `antigravity-rate-limits.json`, then
//! chains any previous user `statusLine` command. Always exits `0`.
//! `hook invocation` evaluates `antigravity-guard.json` and emits `{}` to allow
//! (so tool permission prompts are never altered) or an injectSteps termination
//! payload to block low-quota requests.
#[allow(dead_code)]
#[path = "../antigravity/shared.rs"]
mod shared;

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use shared::{GuardDecision, GuardState};

const CHAINED_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_INPUT_BYTES: u64 = 1024 * 1024;

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

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn read_stdin() -> String {
    let mut input = String::new();
    let _ = std::io::stdin().take(MAX_INPUT_BYTES).read_to_string(&mut input);
    input
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

    let config: shared::BridgeConfig = read_json(&dir.join(shared::BRIDGE_FILE)).unwrap_or_default();
    if let Some(output) = config.chained_command().and_then(|command| run_chained(command, &raw)) {
        let _ = std::io::stdout().write_all(&output);
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn run_chained(command: &str, input: &str) -> Option<Vec<u8>> {
    let mut child = shell(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.read_to_end(&mut output);
        output
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < CHAINED_TIMEOUT => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    reader.join().ok().filter(|output| !output.is_empty())
}

fn shell(command: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let prefer_bash = command.trim_start().starts_with("bash ")
            || command.contains(".sh")
            || command.contains("~/");
        let mut shell = match (prefer_bash, git_bash()) {
            (true, Some(bash)) => {
                let mut shell = Command::new(bash);
                shell.arg("-c").arg(command);
                shell
            }
            _ => {
                let mut shell = Command::new("cmd");
                shell.arg("/C").arg(command);
                shell
            }
        };
        shell.creation_flags(CREATE_NO_WINDOW);
        shell
    }
    #[cfg(not(windows))]
    {
        let mut shell = Command::new("sh");
        shell.arg("-c").arg(command);
        shell
    }
}

#[cfg(windows)]
fn git_bash() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>());
    on_path
        .filter(|dir| {
            let dir = dir.to_string_lossy().to_ascii_lowercase();
            !dir.contains("\\windows\\system32") && !dir.contains("windowsapps")
        })
        .map(|dir| dir.join("bash.exe"))
        .chain(
            ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"]
                .into_iter()
                .filter_map(std::env::var_os)
                .flat_map(|root| {
                    let root = PathBuf::from(root);
                    [
                        root.join("Git").join("bin").join("bash.exe"),
                        root.join("Programs").join("Git").join("bin").join("bash.exe"),
                    ]
                }),
        )
        .find(|path| path.is_file())
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
    record_block(&dir, now);
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

fn record_block(dir: &Path, now: i64) {
    let path = dir.join(shared::GUARD_STATE_FILE);
    let mut state: GuardState = read_json(&path).unwrap_or_default();
    state.blocked_count = state.blocked_count.saturating_add(1);
    state.last_blocked_at = Some(now);
    let _ = shared::write_json_atomic(&path, &state);
}
