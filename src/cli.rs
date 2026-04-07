use clap::{Parser, Subcommand};

const MAIN_HELP: &str = "Windows process supervisor for AI-launched apps.

By default, apps run transparently (stdin/stdout/stderr inherited, just like running the command directly).
Do NOT use --mode capture unless you specifically want to background the process and lose stdin.

Quick start:
  visor start --name api python server.py       # runs transparently (default)
  visor list                                     # show running apps
  visor stop api                                 # stop by name
  visor status                                   # daemon health

Run 'visor help <command>' for details on any command.
Run 'visor help-all' for full reference with all options and examples.";

pub const FULL_HELP: &str = r#"VISOR - Full Command Reference
==============================

IMPORTANT: The default mode is TRANSPARENT. Do NOT add --mode capture unless
you specifically want to background the process. Capture mode has NO stdin,
so interactive apps (REPLs, prompts) will exit immediately.

STARTING APPS
  Flags go BEFORE the command. Everything after <cmd> is passed as arguments.

  Default (transparent) - the process inherits your terminal exactly as if
  you ran the command directly. stdin, stdout, stderr all work normally:
    visor start --name api python server.py
    visor start --name dev npm run dev
    visor start --name server --cwd C:\projects\api cargo run
    visor start --name certdb C:\app\certdb.exe --web:8080

  Capture mode - ONLY use when you want a fully backgrounded process with
  no terminal interaction. stdin is /dev/null. Output goes to a log file:
    visor start --name worker --mode capture node worker.js
    visor start --name bg-api --mode capture --agent claude python server.py

  Protect from accidental kills with a 4-digit code:
    visor start --name critical --kill-code 1234 python server.py

  All start flags:
    --name <NAME>        Required. Friendly name (must be unique among running apps)
    --mode <MODE>        "transparent" (default) or "capture" (no stdin, backgrounded)
    --agent <AGENT>      Tag with an agent name (e.g. claude, copilot)
    --group <GROUP>      Tag with a group name for bulk operations
    --cwd <PATH>         Working directory for the process
    --kill-code <CODE>   4-digit code required to stop this app

LISTING APPS
  visor list                    Show all running apps
  visor list --json             JSON output
  visor list --agent claude     Filter by agent
  visor list --group myproject  Filter by group

STOPPING APPS
  By name:    visor stop api
  By ID:      visor stop --id <uuid>
  By PID:     visor stop --pid 12345
  By agent:   visor stop --agent claude        (stops ALL for that agent)
  By group:   visor stop --group myproject     (stops ALL in that group)
  Stop all:   visor stop-all

  Kill code protected apps require --code:
    visor stop critical --code 1234
    visor stop-all --code 1234

  Master code 4334 overrides any kill code:
    visor stop critical --code 4334

VIEWING OUTPUT (capture mode only)
  visor attach api              Tail live output from current position
  visor attach api --history    Show full output from the start
  visor logs api                Show the log file path

GUI DASHBOARD
  visor gui                     Open WebView2 dashboard on port 9847
  visor gui --port 4173         Custom port
  Dashboard includes: live process list, stats, history, xterm.js terminal

DAEMON
  visor status                  Show daemon health + active app count
  visor daemon                  Explicitly start daemon (auto-starts on any command)
  visor cleanup                 Force-remove stale/dead entries from the registry

I/O MODES
  transparent  DEFAULT. Process inherits your terminal stdin/stdout/stderr.
    (default)  CLI blocks until the process exits. The app behaves exactly
               as if you ran the command directly. Use this for everything
               unless you have a specific reason not to.

  capture      Process runs backgrounded with NO stdin. stdout/stderr saved
               to C:\dev\scripts\visor-logs\<id>.log. Only use for headless
               workers, background services, or daemons that don't need
               terminal input. Interactive apps WILL exit immediately.

SAFETY
  Visor uses Windows Job Objects. Stop operations only kill the target process
  and its descendants - never parent shells, VS Code, or unrelated processes.
"#;

#[derive(Parser, Debug)]
#[command(
    name = "visor",
    version,
    about = "Windows process supervisor for AI-launched apps",
    long_about = MAIN_HELP,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(long = "daemon-internal", hide = true)]
    pub daemon_internal: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Show daemon health and summary
    Status,

    /// Start the background daemon (auto-starts on any command)
    Daemon,

    /// Start a process (transparent by default - inherits your terminal)
    #[command(
        after_help = "The default mode is transparent: the app gets your terminal's stdin/stdout/stderr.\nDo NOT use --mode capture unless you want a backgrounded process with no stdin.\n\nExamples:\n  visor start --name api python server.py\n  visor start --name dev npm run dev\n  visor start --name app C:\\path\\to\\app.exe --flag:value\n  visor start --name bg --mode capture node worker.js   # backgrounded, no stdin"
    )]
    Start {
        /// Command to run
        cmd: String,

        /// Arguments passed to the command (everything after <cmd>)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,

        /// Required. Unique friendly name for this app
        #[arg(long)]
        name: String,

        /// Tag with an agent name (for filtering/bulk stop)
        #[arg(long)]
        agent: Option<String>,

        /// Tag with a group name (for filtering/bulk stop)
        #[arg(long)]
        group: Option<String>,

        /// Working directory for the process
        #[arg(long)]
        cwd: Option<String>,

        /// 4-digit code required to stop this app. Master code: 4334
        #[arg(long)]
        kill_code: Option<String>,

        /// Default: transparent (inherits terminal). Only use "capture" for headless/background processes (no stdin!)
        #[arg(long, default_value = "transparent")]
        mode: String,

        /// Auto-restart when the process exits
        #[arg(long)]
        restart: bool,

        /// Watch the executable file and restart when it's overwritten (hot-reload for compiled apps)
        #[arg(long)]
        watch_exe: Option<String>,
    },

    /// List running tracked apps
    #[command(
        after_help = "Examples:\n  visor list\n  visor list --json\n  visor list --agent claude\n  visor list --group myproject"
    )]
    List {
        /// Filter by agent
        #[arg(long)]
        agent: Option<String>,

        /// Filter by group
        #[arg(long)]
        group: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Stop a tracked app. Specify ONE of: name, --id, --pid, --agent, --group
    #[command(
        after_help = "Examples:\n  visor stop api\n  visor stop --agent claude\n  visor stop --group myproject\n  visor stop protected --code 1234\n  visor stop protected --code 4334   (master code)"
    )]
    Stop {
        /// App name to stop
        name: Option<String>,

        /// Stop by internal id (uuid)
        #[arg(long)]
        id: Option<String>,

        /// Stop by PID (must be visor-tracked)
        #[arg(long)]
        pid: Option<u32>,

        /// Stop ALL apps tagged with this agent
        #[arg(long)]
        agent: Option<String>,

        /// Stop ALL apps in this group
        #[arg(long)]
        group: Option<String>,

        /// Kill code (required if app has --kill-code). Master: 4334
        #[arg(long)]
        code: Option<String>,
    },

    /// Stop all tracked apps. Use --code for protected apps (master: 4334)
    StopAll {
        /// Kill code for protected apps. Master code: 4334
        #[arg(long)]
        code: Option<String>,
    },

    /// Remove dead/stale entries from the registry
    Cleanup,

    /// Attach to a running app's live output (capture mode only)
    #[command(
        after_help = "Examples:\n  visor attach api\n  visor attach api --history"
    )]
    Attach {
        /// App name
        name: Option<String>,

        /// App id (uuid)
        #[arg(long)]
        id: Option<String>,

        /// Show full output from start (default: tail from now)
        #[arg(long)]
        history: bool,
    },

    /// Open the WebView2 GUI dashboard with live process view and terminal
    Gui {
        /// Localhost port for the dashboard
        #[arg(long, default_value = "9847")]
        port: u16,
    },

    /// Show log file path for a captured app
    Logs {
        /// App name
        name: Option<String>,

        /// App id (uuid)
        #[arg(long)]
        id: Option<String>,
    },

    /// Start a static file server (tracked as a visor process)
    #[command(
        after_help = "Each invocation picks a random free port unless --port is given.\n\nExamples:\n  visor serve                                   # random port\n  visor serve --path C:\\projects\\site --port 3000 --name mysite"
    )]
    Serve {
        /// Directory to serve (default: current directory)
        #[arg(long, default_value = ".")]
        path: String,

        /// Port to listen on (default: random free port)
        #[arg(long)]
        port: Option<u16>,

        /// Friendly name for this server (default: fileserver-<port>)
        #[arg(long)]
        name: Option<String>,
    },

    /// Internal: run the file server directly (used by daemon)
    #[command(hide = true)]
    ServeInternal {
        #[arg(long)]
        path: String,
        #[arg(long)]
        port: u16,
    },

    /// Manage saved apps
    #[command(subcommand)]
    App(AppCommands),

    /// Show full reference with all options, examples, and modes explained
    HelpAll,
}

#[derive(Subcommand, Debug)]
pub enum AppCommands {
    /// Add or update a saved app (auto-scans for commands)
    #[command(after_help = "Examples:\n  visor app add --name myapi --path C:\\projects\\api\n  visor app add --name myapi --path C:\\projects\\api --tag backend --tag api --desc \"REST API server\"")]
    Add {
        /// App name (unique identifier)
        #[arg(long)]
        name: String,

        /// Path to the project directory
        #[arg(long)]
        path: String,

        /// Description (markdown)
        #[arg(long, default_value = "")]
        desc: String,

        /// Tags (can be repeated)
        #[arg(long)]
        tag: Vec<String>,
    },

    /// List all saved apps
    List,

    /// Show details of a saved app
    Get {
        /// App name
        name: String,
    },

    /// Remove a saved app
    Remove {
        /// App name
        name: String,
    },

    /// Run a command from a saved app
    #[command(after_help = "Examples:\n  visor app run myapi dev\n  visor app run frontend build")]
    Run {
        /// App name
        name: String,

        /// Command category to run (dev, build, run, test, or label text)
        cmd: String,
    },
}
