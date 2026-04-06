//! ConPTY (Windows Pseudo Console) support.
//!
//! Creates a real pseudo-terminal for a child process so that libraries like
//! rustyline, crossterm, etc. see a genuine TTY on stdin/stdout/stderr.
//! The PTY's I/O is relayed to/from visor's own console handles.

use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::ptr;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Console::{
    CreatePseudoConsole, ClosePseudoConsole, GetConsoleScreenBufferInfo,
    GetStdHandle, COORD, HPCON, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, InitializeProcThreadAttributeList, UpdateProcThreadAttribute,
    DeleteProcThreadAttributeList, WaitForSingleObject, GetExitCodeProcess,
    PROCESS_INFORMATION, STARTUPINFOEXW, EXTENDED_STARTUPINFO_PRESENT,
    LPPROC_THREAD_ATTRIBUTE_LIST, INFINITE,
};
use windows::core::PWSTR;


const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x00020016;

/// Run a process with a real pseudo-terminal. Returns the exit code.
pub fn run_with_pty(cmd: &str, args: &[String], cwd: Option<&str>) -> Result<u32> {
    unsafe { run_with_pty_inner(cmd, args, cwd) }
}

unsafe fn run_with_pty_inner(cmd: &str, args: &[String], cwd: Option<&str>) -> Result<u32> {
    // Get current console size for the PTY
    let size = get_console_size().unwrap_or(COORD { X: 120, Y: 30 });

    // Create pipes: visor reads from pty_out_read, writes to pty_in_write
    // PTY reads from pty_in_read, writes to pty_out_write
    let (pty_in_read, pty_in_write) = create_pipe()?;
    let (pty_out_read, pty_out_write) = create_pipe()?;

    // Create pseudo console
    let hpc = CreatePseudoConsole(size, pty_in_read, pty_out_write, 0)
        .context("CreatePseudoConsole failed")?;

    // Close the pipe ends that the PTY now owns
    let _ = CloseHandle(pty_in_read);
    let _ = CloseHandle(pty_out_write);

    // Build the command line string
    let cmdline = build_cmdline(cmd, args);
    let mut cmdline_wide: Vec<u16> = cmdline.encode_utf16().chain(std::iter::once(0)).collect();

    // Set up cwd
    let cwd_wide: Option<Vec<u16>> = cwd.map(|c| c.encode_utf16().chain(std::iter::once(0)).collect());

    // Initialize proc thread attribute list with the pseudo console
    let mut attr_size: usize = 0;
    let _ = InitializeProcThreadAttributeList(
        LPPROC_THREAD_ATTRIBUTE_LIST(ptr::null_mut()),
        1,
        0,
        &mut attr_size,
    );

    let attr_buf = vec![0u8; attr_size];
    let attr_list = LPPROC_THREAD_ATTRIBUTE_LIST(attr_buf.as_ptr() as *mut _);

    InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_size)
        .context("InitializeProcThreadAttributeList failed")?;

    UpdateProcThreadAttribute(
        attr_list,
        0,
        PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
        Some(hpc.0 as *const std::ffi::c_void),
        std::mem::size_of::<HPCON>(),
        None,
        None,
    )
    .context("UpdateProcThreadAttribute for PSEUDOCONSOLE failed")?;

    // Set up STARTUPINFOEXW
    let mut si = STARTUPINFOEXW::default();
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    si.lpAttributeList = attr_list;

    let mut pi = PROCESS_INFORMATION::default();

    let cwd_ptr = cwd_wide
        .as_ref()
        .map(|c| windows::core::PCWSTR(c.as_ptr()))
        .unwrap_or(windows::core::PCWSTR(ptr::null()));

    CreateProcessW(
        None,
        PWSTR(cmdline_wide.as_mut_ptr()),
        None,
        None,
        false,
        EXTENDED_STARTUPINFO_PRESENT,
        None,
        cwd_ptr,
        &si.StartupInfo,
        &mut pi,
    )
    .context("CreateProcessW with ConPTY failed")?;

    let _ = CloseHandle(pi.hThread);

    // Clean up attribute list
    DeleteProcThreadAttributeList(attr_list);

    // Spawn relay threads: stdin -> pty_in, pty_out -> stdout
    // Pass handles as usize to satisfy Send bounds (HANDLE is a raw pointer)
    let pty_in_raw = pty_in_write.0 as usize;
    let pty_out_raw = pty_out_read.0 as usize;
    let proc_raw = pi.hProcess.0 as usize;

    let _stdin_thread = std::thread::spawn(move || {
        relay_stdin_to_pty(
            HANDLE(pty_in_raw as *mut _),
            HANDLE(proc_raw as *mut _),
        );
    });

    let stdout_thread = std::thread::spawn(move || {
        // The relay reads from the pipe and writes to stdout.
        // It does NOT close the handle — the main thread does that.
        relay_pty_to_stdout_no_close(HANDLE(pty_out_raw as *mut _));
    });

    // Wait for child process to exit
    WaitForSingleObject(pi.hProcess, INFINITE);

    let mut exit_code: u32 = 0;
    let _ = GetExitCodeProcess(pi.hProcess, &mut exit_code);

    // Close pseudo console
    ClosePseudoConsole(hpc);

    // Close the pipe read handle to unblock the stdout relay thread's ReadFile
    let _ = CloseHandle(pty_out_read);

    // Also close the pty_in_write to unblock stdin relay if it's writing
    let _ = CloseHandle(pty_in_write);

    let _ = CloseHandle(pi.hProcess);

    // Wait briefly for stdout relay to drain, then move on
    let _ = stdout_thread.join();

    Ok(exit_code)
}

fn build_cmdline(cmd: &str, args: &[String]) -> String {
    let mut parts = Vec::new();
    parts.push(quote_arg(cmd));
    for arg in args {
        parts.push(quote_arg(arg));
    }
    parts.join(" ")
}

fn quote_arg(arg: &str) -> String {
    if arg.contains(' ') || arg.contains('"') {
        // Escape embedded quotes and wrap
        let escaped = arg.replace('"', "\\\"");
        format!("\"{}\"", escaped)
    } else {
        arg.to_string()
    }
}

unsafe fn get_console_size() -> Option<COORD> {
    let handle = GetStdHandle(STD_OUTPUT_HANDLE).ok()?;
    let mut info = std::mem::zeroed();
    GetConsoleScreenBufferInfo(handle, &mut info).ok()?;
    Some(COORD {
        X: info.srWindow.Right - info.srWindow.Left + 1,
        Y: info.srWindow.Bottom - info.srWindow.Top + 1,
    })
}

unsafe fn create_pipe() -> Result<(HANDLE, HANDLE)> {
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    CreatePipe(&mut read, &mut write, None, 0)
        .context("CreatePipe failed")?;
    Ok((read, write))
}

/// Read from real stdin and write to the PTY input pipe.
/// Also watches the child process handle to exit when the child dies.
fn relay_stdin_to_pty(pty_in: HANDLE, child_process: HANDLE) {
    use windows::Win32::System::Threading::WaitForSingleObject;

    // Check if stdin is a console. If not (piped), don't try to read —
    // just wait for the child to exit and close the pipe.
    let stdin_handle = unsafe {
        GetStdHandle(windows::Win32::System::Console::STD_INPUT_HANDLE)
    };
    let is_console = if let Ok(h) = stdin_handle {
        let mut mode = windows::Win32::System::Console::CONSOLE_MODE::default();
        unsafe {
            windows::Win32::System::Console::GetConsoleMode(h, &mut mode).is_ok()
        }
    } else {
        false
    };

    if !is_console {
        // stdin is piped — just wait for child to exit
        unsafe { WaitForSingleObject(child_process, INFINITE); }
        unsafe { let _ = CloseHandle(pty_in); }
        return;
    }

    // stdin is a real console — relay input to PTY
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut buf = [0u8; 4096];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if raw_write_handle(pty_in, &buf[..n]).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    unsafe { let _ = CloseHandle(pty_in); }
}

/// Read from the PTY output pipe and write to real stdout.
/// Does NOT close the handle — caller is responsible.
fn relay_pty_to_stdout_no_close(pty_out: HANDLE) {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let mut buf = [0u8; 4096];
    loop {
        match raw_read_handle(pty_out, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if stdout.write_all(&buf[..n]).is_err() {
                    break;
                }
                let _ = stdout.flush();
            }
            Err(_) => break,
        }
    }
}

fn raw_write_handle(handle: HANDLE, data: &[u8]) -> Result<()> {
    use windows::Win32::Storage::FileSystem::WriteFile;
    let mut written = 0u32;
    unsafe {
        WriteFile(handle, Some(data), Some(&mut written), None)
            .context("WriteFile failed")?;
    }
    Ok(())
}

fn raw_read_handle(handle: HANDLE, buf: &mut [u8]) -> Result<usize> {
    use windows::Win32::Storage::FileSystem::ReadFile;
    let mut read = 0u32;
    unsafe {
        match ReadFile(handle, Some(buf), Some(&mut read), None) {
            Ok(()) => Ok(read as usize),
            Err(_) => Ok(0), // pipe closed
        }
    }
}
