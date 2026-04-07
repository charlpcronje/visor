use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::os::windows::process::CommandExt;
use std::process::Command;
use uuid::Uuid;

use crate::ipc::PipeClient;
use crate::models::{AppRecord, IoMode, Request, Response};

const MAX_CONNECT_ATTEMPTS: u32 = 20;
const CONNECT_RETRY_MS: u64 = 250;

/// Send a request to the daemon, starting it if necessary.
pub fn send_request(request: Request) -> Result<Response> {
    match PipeClient::connect() {
        Ok(client) => return send_via_client(&client, &request),
        Err(_) => {
            start_daemon()?;
        }
    }

    for _ in 0..MAX_CONNECT_ATTEMPTS {
        std::thread::sleep(std::time::Duration::from_millis(CONNECT_RETRY_MS));
        if let Ok(client) = PipeClient::connect() {
            return send_via_client(&client, &request);
        }
    }

    anyhow::bail!("Failed to connect to visor daemon after starting it")
}

fn send_via_client(client: &PipeClient, request: &Request) -> Result<Response> {
    let req_bytes = serde_json::to_vec(request).context("Failed to serialize request")?;
    client.send_request(&req_bytes)?;
    let resp_bytes = client.read_response()?;
    let response: Response =
        serde_json::from_slice(&resp_bytes).context("Failed to parse response")?;
    Ok(response)
}

/// Run a process in transparent mode: CLI spawns the child with inherited stdio,
/// registers it with the daemon, waits for it to exit.
pub fn run_transparent(
    cmd: String,
    args: Vec<String>,
    name: String,
    agent: Option<String>,
    group: Option<String>,
    cwd: Option<String>,
    kill_code: Option<String>,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    let job_name = id.clone();

    // Register with daemon in a background thread so it doesn't delay the child
    let reg_id = id.clone();
    let reg_cmd = cmd.clone();
    let reg_args = args.clone();
    let reg_name = name.clone();
    let reg_agent = agent.clone();
    let reg_group = group.clone();
    let reg_cwd = cwd.clone();
    let reg_kill_code = kill_code.clone();
    let reg_job_name = job_name.clone();

    std::thread::spawn(move || {
        let _ = send_request(Request::Register {
            id: reg_id,
            pid: 0, // will be updated below via a second message, or daemon reconciles
            cmd: reg_cmd,
            args: reg_args,
            name: reg_name,
            agent: reg_agent,
            group: reg_group,
            cwd: reg_cwd,
            kill_code: reg_kill_code,
            io_mode: IoMode::Transparent,
            job_name: reg_job_name,
        });
    });

    // Use ConPTY: the child gets a real pseudo-terminal, so libraries like
    // rustyline, crossterm, etc. see a genuine TTY on all handles.
    let exit_code = crate::pty::run_with_pty(&cmd, &args, cwd.as_deref())?;

    std::process::exit(exit_code as i32);
}

/// Attach to a captured process's log output.
pub fn attach(name: Option<String>, id: Option<String>, history: bool) -> Result<()> {
    let resp = send_request(Request::Attach {
        name: name.clone(),
        id: id.clone(),
    })?;

    match resp {
        Response::AttachInfo { log_path, name } => {
            eprintln!("Attached to '{}'. Press Ctrl+C to detach.", name);
            tail_file(&log_path, history)?;
        }
        Response::Error { message } => {
            eprintln!("Error: {message}");
        }
        _ => {
            eprintln!("Unexpected response");
        }
    }
    Ok(())
}

fn tail_file(path: &str, from_start: bool) -> Result<()> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open log file: {path}"))?;
    let mut reader = BufReader::new(file);

    if !from_start {
        reader.seek(SeekFrom::End(0))?;
    }

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // No new data — wait briefly
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Ok(_) => {
                print!("{}", line);
            }
            Err(_) => break,
        }
    }
    Ok(())
}

fn start_daemon() -> Result<()> {
    let exe = std::env::current_exe().context("Failed to get current exe path")?;

    // CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS | CREATE_NO_WINDOW
    let flags = 0x00000200 | 0x00000008 | 0x08000000;

    Command::new(exe)
        .arg("--daemon-internal")
        .creation_flags(flags)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn daemon process")?;

    Ok(())
}

pub fn is_daemon_running() -> bool {
    PipeClient::connect().is_ok()
}

pub fn print_response(response: &Response, json_mode: bool) {
    match response {
        Response::Started { id, name, pid } => {
            println!("Started '{name}' (id={id}, pid={pid})");
        }
        Response::AppList { apps } => {
            if json_mode {
                println!("{}", serde_json::to_string_pretty(apps).unwrap_or_default());
            } else if apps.is_empty() {
                println!("No tracked apps running.");
            } else {
                print_app_table(apps);
            }
        }
        Response::Stopped { count, names } => {
            if *count == 0 {
                println!("No apps matched.");
            } else {
                println!("Stopped {count} app(s): {}", names.join(", "));
            }
        }
        Response::Cleaned { removed } => {
            println!("Cleanup complete. Removed {removed} stale entry(ies).");
        }
        Response::Status {
            daemon_running,
            active_apps,
            active_agents,
            active_groups,
            db_path,
            pipe_name,
        } => {
            println!("Visor Daemon Status");
            println!("  Running:       {daemon_running}");
            println!("  Active apps:   {active_apps}");
            println!("  Agents:        {active_agents}");
            println!("  Groups:        {active_groups}");
            println!("  Database:      {db_path}");
            println!("  Pipe:          {pipe_name}");
        }
        Response::AttachInfo { log_path, name } => {
            println!("Log for '{name}': {log_path}");
        }
        Response::ScanResult { projects } => {
            println!("{}", serde_json::to_string_pretty(projects).unwrap_or_default());
        }
        Response::AppProfiles { profiles } => {
            println!("{}", serde_json::to_string_pretty(profiles).unwrap_or_default());
        }
        Response::AppProfile { profile } => {
            println!("Saved app '{}' at {}", profile.name, profile.path);
        }
        Response::AppActivityResult { activity } => {
            println!("{}", serde_json::to_string_pretty(activity).unwrap_or_default());
        }
        Response::AppMetrics { metrics } => {
            println!("{}", serde_json::to_string_pretty(metrics).unwrap_or_default());
        }
        Response::Ok { message } => {
            println!("{message}");
        }
        Response::Error { message } => {
            eprintln!("Error: {message}");
        }
    }
}

fn print_app_table(apps: &[AppRecord]) {
    println!(
        "{:<20} {:<8} {:<10} {:<12} {:<12} {:<12} {}",
        "NAME", "PID", "MODE", "AGENT", "GROUP", "STARTED", "COMMAND"
    );
    println!("{}", "-".repeat(100));

    for app in apps {
        let agent = app.agent.as_deref().unwrap_or("-");
        let group = app.group_name.as_deref().unwrap_or("-");
        let started = app.started_at.format("%Y-%m-%d %H:%M");
        let args: Vec<String> = serde_json::from_str(&app.args_json).unwrap_or_default();
        let full_cmd = if args.is_empty() {
            app.cmd.clone()
        } else {
            format!("{} {}", app.cmd, args.join(" "))
        };

        println!(
            "{:<20} {:<8} {:<10} {:<12} {:<12} {:<12} {}",
            app.name, app.pid, app.io_mode, agent, group, started, full_cmd
        );
    }
}
