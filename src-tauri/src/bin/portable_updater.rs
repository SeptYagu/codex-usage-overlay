use std::{env, error::Error, fs, path::PathBuf, process::Command, thread, time::Duration};

#[cfg(windows)]
fn wait_for_process(pid: u32) -> Result<(), Box<dyn Error>> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
    };

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid)? };
    unsafe {
        WaitForSingleObject(process, INFINITE);
        let _ = CloseHandle(process);
    }
    Ok(())
}

#[cfg(not(windows))]
fn wait_for_process(_pid: u32) -> Result<(), Box<dyn Error>> {
    Err("The portable updater helper is only supported on Windows".into())
}

fn restore_backup(current: &PathBuf, backup: &PathBuf) {
    let _ = fs::remove_file(current);
    let _ = fs::rename(backup, current);
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let missing =
        || std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing updater argument");
    let pid = args
        .next()
        .ok_or_else(missing)?
        .to_string_lossy()
        .parse::<u32>()?;
    let current = PathBuf::from(args.next().ok_or_else(missing)?);
    let staged = PathBuf::from(args.next().ok_or_else(missing)?);
    let restart_args: Vec<_> = args.collect();

    wait_for_process(pid)?;

    let backup = current.with_extension("exe.previous");
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    fs::rename(&current, &backup)?;
    if let Err(error) = fs::rename(&staged, &current) {
        restore_backup(&current, &backup);
        return Err(error.into());
    }

    let mut updated_app = match Command::new(&current).args(&restart_args).spawn() {
        Ok(child) => child,
        Err(error) => {
            restore_backup(&current, &backup);
            return Err(error.into());
        }
    };

    for _ in 0..30 {
        if let Some(status) = updated_app.try_wait()? {
            if !status.success() {
                restore_backup(&current, &backup);
                let _ = Command::new(&current).args(&restart_args).spawn();
                return Err(format!("Updated application exited with {status}").into());
            }
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }

    let _ = fs::remove_file(backup);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Portable update failed: {error}");
        std::process::exit(1);
    }
}
