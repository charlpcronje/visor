use anyhow::{Context, Result};
use std::fs;

use crate::ipc::PipeServer;
use crate::models::{Request, Response, MUTEX_NAME, PID_PATH, LOG_PATH};
use crate::supervisor::Supervisor;

/// Run the daemon main loop.
pub fn run() -> Result<()> {
    // Try to acquire singleton mutex
    let _mutex = acquire_singleton_mutex()?;

    // Write PID file
    let pid = std::process::id();
    let _ = fs::write(PID_PATH, pid.to_string());

    // Initialize log file
    let _ = fs::write(LOG_PATH, format!("visor daemon started, pid={pid}\n"));

    let supervisor = Supervisor::new()?;

    // Initial reconciliation
    let removed = supervisor.reconcile()?;
    log(&format!("Initial reconciliation: removed {removed} stale entries"));

    log("Daemon ready, listening for commands...");

    // Main loop: create pipe, accept client, handle request, repeat
    loop {
        let server = match PipeServer::create() {
            Ok(s) => s,
            Err(e) => {
                log(&format!("Failed to create pipe: {e}"));
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
        };

        if server.wait_for_client().is_err() {
            continue;
        }

        let msg = match server.read_message() {
            Ok(m) => m,
            Err(e) => {
                log(&format!("Failed to read message: {e}"));
                server.disconnect();
                continue;
            }
        };

        let request: Request = match serde_json::from_slice(&msg) {
            Ok(r) => r,
            Err(e) => {
                log(&format!("Invalid request: {e}"));
                let err = Response::Error {
                    message: format!("Invalid request: {e}"),
                };
                let _ = server.write_message(&serde_json::to_vec(&err).unwrap());
                server.disconnect();
                continue;
            }
        };

        let is_shutdown = matches!(request, Request::Shutdown);

        log(&format!("Handling request: {request:?}"));
        let response = supervisor.handle_request(request);

        let resp_bytes = serde_json::to_vec(&response).unwrap_or_default();
        let _ = server.write_message(&resp_bytes);
        server.disconnect();

        if is_shutdown {
            log("Shutdown requested, exiting daemon.");
            break;
        }
    }

    // Cleanup
    let _ = fs::remove_file(PID_PATH);
    Ok(())
}

fn log(msg: &str) {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!("[{timestamp}] {msg}\n");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_PATH)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(line.as_bytes())
        });
}

struct MutexGuard {
    handle: windows::Win32::Foundation::HANDLE,
}

unsafe impl Send for MutexGuard {}

impl Drop for MutexGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

fn acquire_singleton_mutex() -> Result<MutexGuard> {
    use windows::core::PCSTR;
    use windows::Win32::System::Threading::CreateMutexA;
    use windows::Win32::Foundation::GetLastError;

    let name = format!("{MUTEX_NAME}\0");
    unsafe {
        let handle = CreateMutexA(None, true, PCSTR(name.as_ptr()))
            .context("CreateMutexA failed")?;

        let last_error = GetLastError();
        // ERROR_ALREADY_EXISTS = 183
        if last_error.0 == 183 {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            anyhow::bail!("Another visor daemon is already running");
        }

        Ok(MutexGuard { handle })
    }
}
