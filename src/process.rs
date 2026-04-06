use anyhow::{Context, Result};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use windows::Win32::Foundation::{CloseHandle, HANDLE, STILL_ACTIVE};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, TerminateProcess, PROCESS_QUERY_INFORMATION,
    PROCESS_TERMINATE, PROCESS_SET_QUOTA, CREATE_SUSPENDED,
};

/// Launch a process in suspended state with output to /dev/null (legacy/internal).
pub fn launch_suspended(
    cmd: &str,
    args: &[String],
    cwd: Option<&str>,
) -> Result<(std::process::Child, HANDLE)> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_SUSPENDED.0);

    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

    let child = command.spawn().with_context(|| format!("Failed to spawn '{cmd}'"))?;
    let pid = child.id();

    let handle = unsafe {
        OpenProcess(
            PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION | PROCESS_TERMINATE,
            false,
            pid,
        )
        .context("OpenProcess failed for newly spawned process")?
    };

    Ok((child, handle))
}

/// Launch a process in suspended state with stdout/stderr captured to a log file.
pub fn launch_suspended_captured(
    cmd: &str,
    args: &[String],
    cwd: Option<&str>,
    log_path: &Path,
) -> Result<(std::process::Child, HANDLE)> {
    // Ensure log directory exists
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let stdout_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("Failed to open log file: {}", log_path.display()))?;
    let stderr_file = stdout_file
        .try_clone()
        .context("Failed to clone log file handle")?;

    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .creation_flags(CREATE_SUSPENDED.0);

    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

    let child = command.spawn().with_context(|| format!("Failed to spawn '{cmd}'"))?;
    let pid = child.id();

    let handle = unsafe {
        OpenProcess(
            PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION | PROCESS_TERMINATE,
            false,
            pid,
        )
        .context("OpenProcess failed for newly spawned process")?
    };

    Ok((child, handle))
}

/// Resume a suspended process by its main thread.
pub fn resume_process(pid: u32) -> Result<()> {
    use windows::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, THREADENTRY32,
        TH32CS_SNAPTHREAD,
    };

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
            .context("CreateToolhelp32Snapshot failed")?;

        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };

        if Thread32First(snapshot, &mut entry).is_ok() {
            loop {
                if entry.th32OwnerProcessID == pid {
                    if let Ok(thread_handle) =
                        OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID)
                    {
                        ResumeThread(thread_handle);
                        let _ = CloseHandle(thread_handle);
                    }
                }
                if Thread32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }
    Ok(())
}

/// Check if a process with the given PID is still alive.
pub fn is_process_alive(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid);
        match handle {
            Ok(h) => {
                let mut exit_code: u32 = 0;
                let alive = GetExitCodeProcess(h, &mut exit_code).is_ok()
                    && exit_code == STILL_ACTIVE.0 as u32;
                let _ = CloseHandle(h);
                alive
            }
            Err(_) => false,
        }
    }
}

/// Terminate a specific process by PID.
pub fn terminate_process(pid: u32) -> Result<()> {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, false, pid)
            .context("OpenProcess for termination failed")?;
        let result = TerminateProcess(handle, 1);
        let _ = CloseHandle(handle);
        result.context("TerminateProcess failed")?;
    }
    Ok(())
}

