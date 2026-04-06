use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::ipc::PipeServer;
use crate::models::{Request, Response, MUTEX_NAME, PID_PATH, LOG_PATH};
use crate::supervisor::Supervisor;

/// Run the daemon main loop.
pub fn run() -> Result<()> {
    let _mutex = acquire_singleton_mutex()?;

    let pid = std::process::id();
    let _ = fs::write(PID_PATH, pid.to_string());
    let _ = fs::write(LOG_PATH, format!("visor daemon started, pid={pid}\n"));

    let supervisor = Arc::new(Supervisor::new()?);

    let removed = supervisor.reconcile()?;
    log(&format!("Initial reconciliation: removed {removed} stale entries"));

    // Background thread: restart dead apps, watch exe files
    let bg_supervisor = Arc::clone(&supervisor);
    std::thread::spawn(move || {
        background_loop(bg_supervisor);
    });

    log("Daemon ready, listening for commands...");

    // Main loop: pipe server
    loop {
        let server = match PipeServer::create() {
            Ok(s) => s,
            Err(e) => {
                log(&format!("Failed to create pipe: {e}"));
                std::thread::sleep(Duration::from_secs(1));
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

    let _ = fs::remove_file(PID_PATH);
    Ok(())
}

/// Background loop: checks every 2 seconds for dead apps to restart and exe file changes.
fn background_loop(supervisor: Arc<Supervisor>) {
    let mut exe_timestamps: HashMap<String, SystemTime> = HashMap::new();

    loop {
        std::thread::sleep(Duration::from_secs(2));

        // 1. Auto-restart dead apps with restart=true
        let restarted = supervisor.reconcile_and_restart();
        for id in &restarted {
            log(&format!("Auto-restarted app {id}"));
        }

        // 2. Watch exe files for changes
        let apps = supervisor.registry.list_running().unwrap_or_default();
        for app in &apps {
            if let Some(ref exe_path) = app.watch_exe {
                match fs::metadata(exe_path) {
                    Ok(meta) => {
                        if let Ok(modified) = meta.modified() {
                            let prev = exe_timestamps.get(&app.id).copied();
                            exe_timestamps.insert(app.id.clone(), modified);

                            if let Some(prev_time) = prev {
                                if modified > prev_time {
                                    log(&format!(
                                        "Exe changed for '{}' ({}), restarting...",
                                        app.name, exe_path
                                    ));
                                    // Stop the app
                                    supervisor.stop_app_public(app);
                                    // Wait for the file to stabilize
                                    wait_for_file_stable(exe_path);
                                    // Restart
                                    if let Err(e) = supervisor.restart_app(app) {
                                        log(&format!("Failed to restart '{}': {e}", app.name));
                                    } else {
                                        log(&format!("Restarted '{}' after exe change", app.name));
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => {
                        // File temporarily gone (being overwritten) — skip this tick
                    }
                }
            }
        }
    }
}

/// Wait for a file to stop changing (stable for 500ms).
fn wait_for_file_stable(path: &str) {
    let mut last_size = 0u64;
    let mut stable_count = 0;

    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(250));
        match fs::metadata(path) {
            Ok(meta) => {
                let size = meta.len();
                if size == last_size && size > 0 {
                    stable_count += 1;
                    if stable_count >= 2 {
                        return; // File stable for 500ms
                    }
                } else {
                    stable_count = 0;
                    last_size = size;
                }
            }
            Err(_) => {
                stable_count = 0;
            }
        }
    }
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
        if last_error.0 == 183 {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            anyhow::bail!("Another visor daemon is already running");
        }

        Ok(MutexGuard { handle })
    }
}
