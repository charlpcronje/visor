mod cli;
mod client;
mod daemon;
mod gui;
mod ipc;
mod job;
mod models;
mod process;
mod registry;
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

    // No subcommand: show help
    if cli.command.is_none() {
        use clap::CommandFactory;
        Cli::command().print_help().ok();
        println!();
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
        }) => {
            let io_mode = IoMode::from_str(&mode);
            match io_mode {
                IoMode::Transparent => {
                    // CLI spawns the process directly with inherited stdio
                    client::run_transparent(cmd, args, name, agent, group, cwd, kill_code)
                }
                IoMode::Capture => {
                    // Daemon spawns with output captured to log file
                    let resp = client::send_request(Request::Start {
                        cmd,
                        args,
                        name,
                        agent,
                        group,
                        cwd,
                        kill_code,
                        io_mode,
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
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
