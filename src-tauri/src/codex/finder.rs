use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub fn find_codex_executable() -> Option<PathBuf> {
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
        let bin_dir = Path::new(&local_app_data).join("OpenAI").join("Codex").join("bin");
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
        let direct_candidate = Path::new(&local_app_data).join("Programs").join("OpenAI Codex").join("codex.exe");
        if direct_candidate.is_file() {
            return Some(direct_candidate);
        }
    }

    // 4. Query Windows Appx package for OpenAI.Codex if available
    find_appx_codex()
}

fn find_appx_codex() -> Option<PathBuf> {
    // Fast powershell lookup for AppxPackage
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$p = Get-AppxPackage -Name OpenAI.Codex | Select-Object -First 1; if ($p) { Join-Path $p.InstallLocation 'app\\resources\\codex.exe' }",
        ])
        .output()
        .ok()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let path_str = stdout.trim();
        if !path_str.is_empty() {
            let p = PathBuf::from(path_str);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}
