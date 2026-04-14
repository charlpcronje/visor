mod activity;
mod cli;
mod client;
mod daemon;
mod fileserver;
mod gui;
mod icons;
mod ipc;
mod job;
mod models;
mod process;
mod pty;
mod registry;
mod scanner;
mod supervisor;

use clap::Parser;
use cli::{AppCommands, Cli, Commands};
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

        Some(Commands::Restart) => {
            if !client::is_daemon_running() {
                println!("Daemon is not running. Nothing to restart.");
                match client::send_request(Request::Status) {
                    Ok(_) => { println!("Daemon started."); Ok(()) }
                    Err(e) => Err(e),
                }
            } else {
                println!("Shutting down daemon (tracked processes will keep running)...");
                // Send shutdown — response may not come back because daemon exits
                let _ = client::send_request(Request::Shutdown);

                // Wait for the daemon to actually exit (mutex released)
                for i in 0..40 {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    if !client::is_daemon_running() {
                        break;
                    }
                    if i == 39 {
                        eprintln!("Warning: daemon took a while to exit");
                    }
                }

                println!("Starting new daemon...");
                // Kick the new daemon by sending any request (auto-starts)
                match client::send_request(Request::Status) {
                    Ok(r) => {
                        client::print_response(&r, false);
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
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

        Some(Commands::Serve { path, port, name, ts }) => {
            let abs_path = std::path::Path::new(&path)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from(&path));
            let abs_path_str = strip_unc_prefix(&abs_path.to_string_lossy());

            // Pick a free port if none specified
            let port = port.unwrap_or_else(find_free_port);
            let name = name.unwrap_or_else(|| format!("fileserver-{port}"));

            let exe = std::env::current_exe()
                .unwrap_or_else(|_| std::path::PathBuf::from("visor"));
            let mut serve_args = vec![
                "serve-internal".to_string(),
                "--path".to_string(),
                abs_path_str.clone(),
                "--port".to_string(),
                port.to_string(),
            ];
            if ts {
                serve_args.push("--ts".to_string());
                // Resolve transpiler path NOW (CLI has user's PATH) so the
                // daemon subprocess can find it even without the full PATH.
                if let Some(t) = resolve_transpiler_path() {
                    serve_args.push("--transpiler".to_string());
                    serve_args.push(t);
                }
            }
            let resp = client::send_request(Request::Start {
                cmd: exe.to_string_lossy().to_string(),
                args: serve_args,
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

        Some(Commands::ServeInternal { path, port, ts, transpiler }) => {
            fileserver::run(&path, port, ts, transpiler.as_deref())
        }

        Some(Commands::App(app_cmd)) => handle_app_command(app_cmd),

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

fn handle_app_command(cmd: AppCommands) -> anyhow::Result<()> {
    use models::{AppCommand, AppProfile, Request};

    match cmd {
        AppCommands::Add { name, path, desc, tag } => {
            let abs_path = std::path::Path::new(&path)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from(&path));
            let abs_path_str = strip_unc_prefix(&abs_path.to_string_lossy());

            // Default name to folder name
            let name = name.unwrap_or_else(|| {
                abs_path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "app".to_string())
            });

            // Auto-scan for commands
            let projects = scanner::scan(&abs_path_str);
            let mut commands: Vec<AppCommand> = Vec::new();

            for project in &projects {
                if project.path == abs_path_str {
                    for pc in &project.commands {
                        let category = guess_category(&pc.label);
                        commands.push(AppCommand {
                            label: pc.label.clone(),
                            category,
                            cmd: pc.cmd.clone(),
                            args: pc.args.clone(),
                            cwd: pc.cwd.clone(),
                        });
                    }
                }
            }

            // Add standard quick actions
            commands.push(AppCommand {
                label: "Open Terminal".to_string(),
                category: "terminal".to_string(),
                cmd: "cmd".to_string(),
                args: vec!["/k".to_string(), format!("cd /d {abs_path_str}")],
                cwd: abs_path_str.clone(),
            });
            commands.push(AppCommand {
                label: "Open in VS Code".to_string(),
                category: "vscode".to_string(),
                cmd: "code".to_string(),
                args: vec![abs_path_str.clone()],
                cwd: abs_path_str.clone(),
            });

            let profile = AppProfile {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.clone(),
                path: abs_path_str,
                description: desc,
                tags: tag,
                commands,
                created_at: chrono::Utc::now().to_rfc3339(),
                icon: None,
            };

            let resp = client::send_request(Request::AppAdd { profile })?;
            client::print_response(&resp, false);
            Ok(())
        }

        AppCommands::List => {
            let resp = client::send_request(Request::AppList)?;
            match &resp {
                models::Response::AppProfiles { profiles } => {
                    if profiles.is_empty() {
                        println!("No saved apps. Use 'visor app add' to add one.");
                    } else {
                        println!("{:<20} {:<12} {:<6} {}", "NAME", "KIND", "CMDS", "PATH");
                        println!("{}", "-".repeat(80));
                        for p in profiles {
                            let tags = if p.tags.is_empty() {
                                String::new()
                            } else {
                                format!(" [{}]", p.tags.join(", "))
                            };
                            println!(
                                "{:<20} {:<12} {:<6} {}{}",
                                p.name,
                                p.tags.first().unwrap_or(&"-".to_string()),
                                p.commands.len(),
                                p.path,
                                tags,
                            );
                        }
                    }
                }
                _ => client::print_response(&resp, false),
            }
            Ok(())
        }

        AppCommands::Get { name } => {
            let resp = client::send_request(Request::AppGet { name })?;
            match &resp {
                models::Response::AppProfile { profile } => {
                    println!("{}", serde_json::to_string_pretty(profile).unwrap_or_default());
                }
                _ => client::print_response(&resp, false),
            }
            Ok(())
        }

        AppCommands::Remove { name } => {
            let resp = client::send_request(Request::AppRemove { name })?;
            client::print_response(&resp, false);
            Ok(())
        }

        AppCommands::Run { name, cmd } => {
            // First get the app to find the command
            let resp = client::send_request(Request::AppGet { name: name.clone() })?;
            match resp {
                models::Response::AppProfile { profile } => {
                    // Find command by category or label substring
                    let idx = profile.commands.iter().position(|c| {
                        c.category == cmd || c.label.to_lowercase().contains(&cmd.to_lowercase())
                    });
                    match idx {
                        Some(i) => {
                            let resp = client::send_request(Request::AppRunCmd {
                                app_name: name,
                                cmd_index: i,
                            })?;
                            client::print_response(&resp, false);
                        }
                        None => {
                            eprintln!("No command matching '{}'. Available:", cmd);
                            for c in &profile.commands {
                                eprintln!("  [{}] {}", c.category, c.label);
                            }
                        }
                    }
                }
                _ => client::print_response(&resp, false),
            }
            Ok(())
        }

        AppCommands::Scan { path, depth } => {
            let abs_path = std::path::Path::new(&path)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from(&path));
            let root = strip_unc_prefix(&abs_path.to_string_lossy());

            println!("Scanning {} (depth {depth})...", root);
            let found = bulk_scan_projects(&root, depth)?;
            println!("Added {found} app(s).");
            Ok(())
        }

        AppCommands::ScanExes { path, depth } => {
            let abs_path = std::path::Path::new(&path)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from(&path));
            let root = strip_unc_prefix(&abs_path.to_string_lossy());

            println!("Scanning for executables in {} (depth {depth})...", root);
            let found = bulk_scan_exes(&root, depth)?;
            println!("Added {found} app(s).");
            Ok(())
        }
    }
}

/// Walk a directory tree up to `max_depth` levels, find projects, add as saved apps.
/// First-level subdirectory becomes a tag.
fn bulk_scan_projects(root: &str, max_depth: usize) -> anyhow::Result<usize> {
    use models::{AppCommand, AppProfile, Request};

    let mut count = 0;
    let root_path = std::path::Path::new(root);

    walk_dirs(root_path, 0, max_depth, &mut |dir, depth| {
        let dir_str = strip_unc_prefix(&dir.to_string_lossy());
        let projects = scanner::scan(&dir_str);

        for project in &projects {
            if strip_unc_prefix(&project.path) != dir_str {
                continue;
            }

            let app_name = dir.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "app".to_string());

            // First-level subdir as tag
            let category_tag = if depth >= 1 {
                get_nth_parent_name(dir, depth - 1, root_path)
            } else {
                None
            };

            let mut tags = vec![project.kind.clone()];
            if let Some(cat) = category_tag {
                tags.insert(0, cat);
            }

            let mut commands: Vec<AppCommand> = project.commands.iter().map(|c| {
                AppCommand {
                    label: c.label.clone(),
                    category: guess_category(&c.label),
                    cmd: c.cmd.clone(),
                    args: c.args.clone(),
                    cwd: c.cwd.clone(),
                }
            }).collect();

            commands.push(AppCommand {
                label: "Open Terminal".to_string(),
                category: "terminal".to_string(),
                cmd: "cmd".to_string(),
                args: vec!["/k".to_string(), format!("cd /d {dir_str}")],
                cwd: dir_str.clone(),
            });
            commands.push(AppCommand {
                label: "Open in VS Code".to_string(),
                category: "vscode".to_string(),
                cmd: "code".to_string(),
                args: vec![dir_str.clone()],
                cwd: dir_str.clone(),
            });

            let profile = AppProfile {
                id: uuid::Uuid::new_v4().to_string(),
                name: app_name.clone(),
                path: dir_str.clone(),
                description: String::new(),
                tags,
                commands,
                created_at: chrono::Utc::now().to_rfc3339(),
                icon: None,
            };

            if let Ok(_) = client::send_request(Request::AppAdd { profile }) {
                println!("  + {app_name} ({dir_str})");
                count += 1;
            }
        }
    });

    Ok(count)
}

/// Walk a directory tree, find .exe files, add as saved apps with extracted icons.
fn bulk_scan_exes(root: &str, max_depth: usize) -> anyhow::Result<usize> {
    use models::{AppCommand, AppProfile, Request};

    let mut count = 0;
    let root_path = std::path::Path::new(root);

    walk_dirs(root_path, 0, max_depth, &mut |dir, depth| {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() { continue; }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext.to_lowercase() != "exe" { continue; }

            let exe_path = strip_unc_prefix(&path.to_string_lossy());
            let app_name = path.file_stem()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "app".to_string());

            let dir_str = strip_unc_prefix(&dir.to_string_lossy());

            // Category from first-level parent
            let category_tag = if depth >= 1 {
                get_nth_parent_name(dir, depth - 1, root_path)
            } else {
                None
            };

            let mut tags = vec!["exe".to_string()];
            if let Some(cat) = category_tag {
                tags.insert(0, cat);
            }

            // Extract icon
            let icon = icons::extract_icon(&exe_path);

            let commands = vec![
                AppCommand {
                    label: format!("Run {app_name}"),
                    category: "run".to_string(),
                    cmd: exe_path.clone(),
                    args: vec![],
                    cwd: dir_str.clone(),
                },
                AppCommand {
                    label: "Open folder".to_string(),
                    category: "terminal".to_string(),
                    cmd: "cmd".to_string(),
                    args: vec!["/k".to_string(), format!("cd /d {dir_str}")],
                    cwd: dir_str.clone(),
                },
            ];

            let profile = AppProfile {
                id: uuid::Uuid::new_v4().to_string(),
                name: app_name.clone(),
                path: dir_str.clone(),
                description: String::new(),
                tags,
                commands,
                created_at: chrono::Utc::now().to_rfc3339(),
                icon,
            };

            if let Ok(_) = client::send_request(Request::AppAdd { profile }) {
                println!("  + {app_name} ({exe_path})");
                count += 1;
            }
        }
    });

    Ok(count)
}

fn walk_dirs(dir: &std::path::Path, depth: usize, max_depth: usize, cb: &mut dyn FnMut(&std::path::Path, usize)) {
    if depth > max_depth { return; }

    cb(dir, depth);

    if depth >= max_depth { return; }

    let skip = ["node_modules", ".git", "target", "__pycache__", "vendor",
        ".venv", "venv", "dist", "build", ".next", ".cache", ".cargo"];

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.filter_map(|e| e.ok()) {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().to_string();
            if skip.contains(&name.as_str()) || name.starts_with('.') {
                continue;
            }
            walk_dirs(&entry.path(), depth + 1, max_depth, cb);
        }
    }
}

fn get_nth_parent_name(path: &std::path::Path, _n: usize, root: &std::path::Path) -> Option<String> {
    // Walk up from path, find the component that is n levels above
    let rel = path.strip_prefix(root).ok()?;
    let components: Vec<_> = rel.components().collect();
    if components.is_empty() { return None; }
    Some(components[0].as_os_str().to_string_lossy().to_string())
}

fn guess_category(label: &str) -> String {
    let l = label.to_lowercase();
    if l.contains("dev") || l.contains("serve") || l.contains("watch") { "dev".to_string() }
    else if l.contains("build") || l.contains("compile") { "build".to_string() }
    else if l.contains("test") { "test".to_string() }
    else if l.contains("start") || l.contains("run") { "run".to_string() }
    else if l.contains("install") || l.contains("tidy") { "setup".to_string() }
    else { "custom".to_string() }
}

/// Resolve a transpiler's full path using the current user's PATH.
fn resolve_transpiler_path() -> Option<String> {
    for cmd in &["esbuild", "swc", "bun", "tsc"] {
        if let Ok(output) = std::process::Command::new("where")
            .arg(cmd)
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout);
                let first_line = path.lines().next().unwrap_or("").trim();
                if !first_line.is_empty() {
                    return Some(first_line.to_string());
                }
            }
        }
    }
    None
}

/// Strip the \\?\ prefix that Windows canonicalize adds.
fn strip_unc_prefix(s: &str) -> String {
    s.strip_prefix(r"\\?\").unwrap_or(s).to_string()
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
