use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn find_on_disk() -> Option<PathBuf> {
    // 1. Check CODEX_CLI_PATH environment variable
    if let Ok(cli_path) = env::var("CODEX_CLI_PATH") {
        let p = PathBuf::from(cli_path.trim());
        if p.is_file() {
            return Some(p);
        }
    }

    // 2. Check PATH environment variable
    if let Ok(paths) = env::var("PATH") {
        for dir in env::split_paths(&paths) {
            let candidate = dir.join("codex.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // 3. Check %LOCALAPPDATA%\OpenAI\Codex\bin\<hash>\codex.exe
    if let Ok(local_app_data) = env::var("LOCALAPPDATA") {
        let bin_dir = Path::new(&local_app_data)
            .join("OpenAI")
            .join("Codex")
            .join("bin");
        if bin_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&bin_dir) {
                let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
                for entry in entries.flatten() {
                    let exe_path = entry.path().join("codex.exe");
                    if exe_path.is_file() {
                        let mtime = entry
                            .metadata()
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::UNIX_EPOCH);
                        candidates.push((mtime, exe_path));
                    }
                }
                candidates.sort_by(|a, b| b.0.cmp(&a.0));
                if let Some((_, newest_exe)) = candidates.into_iter().next() {
                    return Some(newest_exe);
                }
            }
        }

        // Check fallback direct paths in LocalAppData
        let direct_candidate = Path::new(&local_app_data)
            .join("Programs")
            .join("OpenAI Codex")
            .join("codex.exe");
        if direct_candidate.is_file() {
            return Some(direct_candidate);
        }
    }

    None
}

pub async fn find_codex_executable(
    deadline: tokio::time::Instant,
) -> Result<Option<PathBuf>, super::reader::ReadFailure> {
    use super::reader::{FailureKind, ReadFailure, ReadStage};
    use tokio::time::{timeout_at, Instant};
    // The worker only finds a path; it can never spawn a late usage reader.
    let deadline = deadline.min(Instant::now() + std::time::Duration::from_secs(5));
    let path = timeout_at(deadline, tokio::task::spawn_blocking(find_on_disk))
        .await
        .map_err(|_| ReadFailure::timeout(ReadStage::Discovery))?
        .map_err(|_| ReadFailure::new(ReadStage::Discovery, FailureKind::MissingExecutable))?;
    if path.is_some() {
        return Ok(path);
    }
    if Instant::now() >= deadline {
        return Err(ReadFailure::timeout(ReadStage::Discovery));
    }
    // Default transport lookup is owned and cancellable, just like the reader.
    let mut command = tokio::process::Command::new("powershell.exe");
    command
        .creation_flags(0x08000000)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let output = timeout_at(deadline, command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$p = Get-AppxPackage -Name OpenAI.Codex | Select-Object -First 1; if ($p) { Join-Path $p.InstallLocation 'app\\resources\\codex.exe' }",
        ])
        .output()).await.map_err(|_| ReadFailure::timeout(ReadStage::Discovery))?;
    let Ok(output) = output else {
        return Ok(None);
    };

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let path_str = stdout.trim();
        if !path_str.is_empty() {
            let p = PathBuf::from(path_str);
            if p.is_file() {
                return Ok(Some(p));
            }
        }
    }
    Ok(None)
}
