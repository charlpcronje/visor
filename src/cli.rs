use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "visor", version, about = "Windows process supervisor for AI-launched apps")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Internal flag: run as background daemon (not for normal use)
    #[arg(long = "daemon-internal", hide = true)]
    pub daemon_internal: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Show daemon health and summary
    Status,

    /// Start the background daemon
    Daemon,

    /// Start a process through visor
    Start {
        /// Command to run
        cmd: String,

        /// Arguments to pass to the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,

        /// Friendly name for this app
        #[arg(long)]
        name: String,

        /// Agent that launched this app
        #[arg(long)]
        agent: Option<String>,

        /// Group this app belongs to
        #[arg(long)]
        group: Option<String>,

        /// Working directory for the process
        #[arg(long)]
        cwd: Option<String>,

        /// 4-digit code required to stop this app (protects against accidental kills)
        #[arg(long)]
        kill_code: Option<String>,

        /// I/O mode: "transparent" (default, inherits terminal) or "capture" (logs to file, runs backgrounded)
        #[arg(long, default_value = "transparent")]
        mode: String,
    },

    /// List running tracked apps
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

    /// Stop a tracked app by name
    Stop {
        /// App name to stop
        name: Option<String>,

        /// Stop by internal id
        #[arg(long)]
        id: Option<String>,

        /// Stop by PID
        #[arg(long)]
        pid: Option<u32>,

        /// Stop all apps for this agent
        #[arg(long)]
        agent: Option<String>,

        /// Stop all apps in this group
        #[arg(long)]
        group: Option<String>,

        /// Kill code (required if the app was started with --kill-code)
        #[arg(long)]
        code: Option<String>,
    },

    /// Stop all tracked apps
    StopAll {
        /// Kill code (required for code-protected apps; master code stops all)
        #[arg(long)]
        code: Option<String>,
    },

    /// Force reconciliation and remove stale entries
    Cleanup,

    /// Attach to a running app's output (capture mode only)
    Attach {
        /// App name
        name: Option<String>,

        /// App id
        #[arg(long)]
        id: Option<String>,

        /// Show full history from start (default: tail from current position)
        #[arg(long)]
        history: bool,
    },

    /// Open the GUI dashboard
    Gui {
        /// Port for the local HTTP API (default: 9847)
        #[arg(long, default_value = "9847")]
        port: u16,
    },

    /// Show log file path for a captured app
    Logs {
        /// App name
        name: Option<String>,

        /// App id
        #[arg(long)]
        id: Option<String>,
    },
}
