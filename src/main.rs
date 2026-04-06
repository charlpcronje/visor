mod cli;
mod client;
mod daemon;
mod fileserver;
mod gui;
mod ipc;
mod job;
mod models;
mod process;
mod pty;
mod registry;
mod scanner;
mod supervisor;

use clap::Parser;
use cli::{Cli, Commands};
use models::{IoMode, Request};

fn main() {
    let cli = Cli::parse();

    // Hidden internal daemon mode
    if cli.daemon_internal {
        if let Err(e) = daemon::run() {
            eprintln!("Daemon error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // No subcommand: if interactive user, launch GUI. If process/agent, show help.
    if cli.command.is_none() {
        if is_interactive_terminal() {
            if let Err(e) = gui::run(9847) {
                eprintln!("GUI failed: {e}");
                std::process::exit(1);
            }
        } else {
            use clap::CommandFactory;
            Cli::command().print_help().ok();
            println!();
        }
        return;
    }

    // CLI client mode
    let result = match cli.command {
        None => unreachable!(),

        Some(Commands::Status) => {
            let resp = client::send_request(Request::Status);
            resp.map(|r| client::print_response(&r, false))
        }

        Some(Commands::Daemon) => {
            if client::is_daemon_running() {
                println!("Visor daemon is already running.");
                return;
            }
            match client::send_request(Request::Status) {
                Ok(_) => {
                    println!("Visor daemon started.");
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }

        Some(Commands::Start {
            cmd,
            args,
            name,
            agent,
            group,
            cwd,
            kill_code,
            mode,
            restart,
            watch_exe,
        }) => {
            let io_mode = IoMode::from_str(&mode);
            match io_mode {
                IoMode::Transparent => {
                    // CLI spawns the process directly with inherited stdio
                    client::run_transparent(cmd, args, name, agent, group, cwd, kill_code)
                }
                IoMode::Capture => {
                    let resp = client::send_request(Request::Start {
                        cmd,
                        args,
                        name,
                        agent,
                        group,
                        cwd,
                        kill_code,
                        io_mode,
                        restart,
                        watch_exe,
                    });
                    resp.map(|r| client::print_response(&r, false))
                }
            }
        }

        Some(Commands::List { agent, group, json }) => {
            let resp = client::send_request(Request::List { agent, group, json });
            resp.map(|r| client::print_response(&r, json))
        }

        Some(Commands::Stop {
            name,
            id,
            pid,
            agent,
            group,
            code,
        }) => {
            let resp = client::send_request(Request::Stop {
                name,
                id,
                pid,
                agent,
                group,
                code,
            });
            resp.map(|r| client::print_response(&r, false))
        }

        Some(Commands::StopAll { code }) => {
            let resp = client::send_request(Request::StopAll { code });
            resp.map(|r| client::print_response(&r, false))
        }

        Some(Commands::Cleanup) => {
            let resp = client::send_request(Request::Cleanup);
            resp.map(|r| client::print_response(&r, false))
        }

        Some(Commands::Attach { name, id, history }) => {
            client::attach(name, id, history)
        }

        Some(Commands::Gui { port }) => gui::run(port),

        Some(Commands::Logs { name, id }) => {
            let resp = client::send_request(Request::Logs { name, id });
            resp.map(|r| client::print_response(&r, false))
        }

        Some(Commands::Serve { path, port, name }) => {
            let abs_path = std::path::Path::new(&path)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from(&path));
            let abs_path_str = abs_path.to_string_lossy().to_string();

            // Pick a free port if none specified
            let port = port.unwrap_or_else(find_free_port);
            let name = name.unwrap_or_else(|| format!("fileserver-{port}"));

            let exe = std::env::current_exe()
                .unwrap_or_else(|_| std::path::PathBuf::from("visor"));
            let resp = client::send_request(Request::Start {
                cmd: exe.to_string_lossy().to_string(),
                args: vec![
                    "serve-internal".to_string(),
                    "--path".to_string(),
                    abs_path_str.clone(),
                    "--port".to_string(),
                    port.to_string(),
                ],
                name: name.clone(),
                agent: None,
                group: Some("fileserver".to_string()),
                cwd: None,
                kill_code: None,
                io_mode: IoMode::Capture,
                restart: false,
                watch_exe: None,
            });
            match resp {
                Ok(r) => {
                    client::print_response(&r, false);
                    println!("Serving {} at http://127.0.0.1:{port}", abs_path_str);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }

        Some(Commands::ServeInternal { path, port }) => {
            fileserver::run(&path, port)
        }

        Some(Commands::HelpAll) => {
            print!("{}", cli::FULL_HELP);
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

/// Check if we're running in an interactive terminal (human user) vs piped/automated (agent).
fn is_interactive_terminal() -> bool {
    use windows::Win32::System::Console::{GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE, CONSOLE_MODE};
    unsafe {
        if let Ok(handle) = GetStdHandle(STD_INPUT_HANDLE) {
            let mut mode = CONSOLE_MODE::default();
            // GetConsoleMode succeeds only if stdin is a real console
            GetConsoleMode(handle, &mut mode).is_ok()
        } else {
            false
        }
    }
}

/// Bind to port 0 to let the OS assign a free port, then return it.
fn find_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(5000)
}
