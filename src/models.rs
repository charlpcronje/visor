use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRecord {
    pub id: String,
    pub name: String,
    pub pid: u32,
    pub agent: Option<String>,
    pub group_name: Option<String>,
    pub cmd: String,
    pub args_json: String,
    pub cwd: Option<String>,
    pub started_at: DateTime<Utc>,
    pub status: AppStatus,
    pub job_name: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub kill_code: Option<String>,
    pub io_mode: IoMode,
    pub log_path: Option<String>,
    pub restart: bool,
    pub watch_exe: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppStatus {
    Running,
    Stopped,
    Dead,
    Failed,
}

impl std::fmt::Display for AppStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppStatus::Running => write!(f, "running"),
            AppStatus::Stopped => write!(f, "stopped"),
            AppStatus::Dead => write!(f, "dead"),
            AppStatus::Failed => write!(f, "failed"),
        }
    }
}

impl AppStatus {
    pub fn from_str(s: &str) -> Self {
        match s {
            "running" => AppStatus::Running,
            "stopped" => AppStatus::Stopped,
            "dead" => AppStatus::Dead,
            "failed" => AppStatus::Failed,
            _ => AppStatus::Dead,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IoMode {
    Transparent,
    Capture,
}

impl std::fmt::Display for IoMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoMode::Transparent => write!(f, "transparent"),
            IoMode::Capture => write!(f, "capture"),
        }
    }
}

impl IoMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "capture" => IoMode::Capture,
            _ => IoMode::Transparent,
        }
    }
}

// IPC request/response types

#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    /// Daemon-spawned start (capture mode)
    Start {
        cmd: String,
        args: Vec<String>,
        name: String,
        agent: Option<String>,
        group: Option<String>,
        cwd: Option<String>,
        kill_code: Option<String>,
        io_mode: IoMode,
        restart: bool,
        watch_exe: Option<String>,
    },
    /// CLI-spawned register (transparent mode) — CLI already started the process
    Register {
        id: String,
        pid: u32,
        cmd: String,
        args: Vec<String>,
        name: String,
        agent: Option<String>,
        group: Option<String>,
        cwd: Option<String>,
        kill_code: Option<String>,
        io_mode: IoMode,
        job_name: String,
    },
    List {
        agent: Option<String>,
        group: Option<String>,
        json: bool,
    },
    Stop {
        name: Option<String>,
        id: Option<String>,
        pid: Option<u32>,
        agent: Option<String>,
        group: Option<String>,
        code: Option<String>,
    },
    StopAll {
        code: Option<String>,
    },
    Cleanup,
    Status,
    Shutdown,
    /// Get log path for attaching to a captured process
    Attach {
        name: Option<String>,
        id: Option<String>,
    },
    /// Scan directory for projects
    Scan {
        path: String,
    },
    /// Get log path for a process
    Logs {
        name: Option<String>,
        id: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Started {
        id: String,
        name: String,
        pid: u32,
    },
    AppList {
        apps: Vec<AppRecord>,
    },
    Stopped {
        count: usize,
        names: Vec<String>,
    },
    Cleaned {
        removed: usize,
    },
    Status {
        daemon_running: bool,
        active_apps: usize,
        active_agents: usize,
        active_groups: usize,
        db_path: String,
        pipe_name: String,
    },
    AttachInfo {
        log_path: String,
        name: String,
    },
    ScanResult {
        projects: Vec<crate::scanner::Project>,
    },
    Ok {
        message: String,
    },
    Error {
        message: String,
    },
}

pub const PIPE_NAME: &str = r"\\.\pipe\visor-control";
pub const DB_PATH: &str = r"C:\dev\scripts\visor.db";
pub const LOG_PATH: &str = r"C:\dev\scripts\visor.log";
pub const LOG_DIR: &str = r"C:\dev\scripts\visor-logs";
pub const PID_PATH: &str = r"C:\dev\scripts\visor.pid";
pub const MUTEX_NAME: &str = "Global\\VisorDaemonMutex";
pub const MASTER_KILL_CODE: &str = "4334";
