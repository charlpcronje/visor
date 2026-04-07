use anyhow::Result;
use chrono::Utc;
use std::path::Path;
use uuid::Uuid;

use crate::job::JobManager;
use crate::activity;
use crate::models::{AppProfile, AppRecord, AppStatus, IoMode, Request, Response, DB_PATH, LOG_DIR, MASTER_KILL_CODE, PIPE_NAME};
use crate::process;
use crate::registry::Registry;
use crate::scanner;

pub struct Supervisor {
    pub registry: Registry,
    pub jobs: JobManager,
}

impl Supervisor {
    pub fn new() -> Result<Self> {
        let registry = Registry::open(DB_PATH)?;
        let jobs = JobManager::new();
        // Ensure log directory exists
        let _ = std::fs::create_dir_all(LOG_DIR);
        Ok(Self { registry, jobs })
    }

    pub fn reconcile(&self) -> Result<usize> {
        let apps = self.registry.list_running()?;
        let mut removed = 0;
        for app in &apps {
            if !process::is_process_alive(app.pid) {
                self.registry.update_status(&app.id, &AppStatus::Dead)?;
                if let Some(ref jn) = app.job_name {
                    self.jobs.close_job(jn);
                }
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::Start {
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
            } => self.handle_start(cmd, args, name, agent, group, cwd, kill_code, io_mode, restart, watch_exe),
            Request::Register {
                id,
                pid,
                cmd,
                args,
                name,
                agent,
                group,
                cwd,
                kill_code,
                io_mode,
                job_name,
            } => self.handle_register(id, pid, cmd, args, name, agent, group, cwd, kill_code, io_mode, job_name),
            Request::List { agent, group, json } => self.handle_list(agent, group, json),
            Request::Stop {
                name,
                id,
                pid,
                agent,
                group,
                code,
            } => self.handle_stop(name, id, pid, agent, group, code),
            Request::StopAll { code } => self.handle_stop_all(code),
            Request::Cleanup => self.handle_cleanup(),
            Request::Status => self.handle_status(),
            Request::Shutdown => Response::Ok {
                message: "Daemon shutting down".to_string(),
            },
            Request::Attach { name, id } => self.handle_attach(name, id),
            Request::Logs { name, id } => self.handle_attach(name, id),
            Request::Scan { path } => self.handle_scan(path),
            Request::AppAdd { profile } => self.handle_app_add(profile),
            Request::AppList => self.handle_app_list(),
            Request::AppGet { name } => self.handle_app_get(name),
            Request::AppRemove { name } => self.handle_app_remove(name),
            Request::AppUpdate { profile } => self.handle_app_add(profile), // upsert
            Request::AppActivity { name } => self.handle_app_activity(name),
            Request::AppRunCmd { app_name, cmd_index } => self.handle_app_run_cmd(app_name, cmd_index),
        }
    }

    fn handle_start(
        &self,
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
    ) -> Response {
        if let Some(ref code) = kill_code {
            if code.len() != 4 || !code.chars().all(|c| c.is_ascii_digit()) {
                return Response::Error {
                    message: "Kill code must be exactly 4 digits".to_string(),
                };
            }
        }

        match self.registry.find_by_name(&name) {
            Ok(Some(_)) => {
                return Response::Error {
                    message: format!("An app named '{name}' is already running. Use a different name or stop it first."),
                };
            }
            Err(e) => {
                return Response::Error {
                    message: format!("Registry error: {e}"),
                };
            }
            _ => {}
        }

        let id = Uuid::new_v4().to_string();
        let job_name = id.clone();

        if let Err(e) = self.jobs.create_job(&job_name) {
            return Response::Error {
                message: format!("Failed to create job object: {e}"),
            };
        }

        // Determine log path for capture mode
        let log_path = match io_mode {
            IoMode::Capture => Some(format!("{}\\{}.log", LOG_DIR, &id)),
            IoMode::Transparent => None,
        };

        let effective_cwd = cwd.as_deref();
        let (child, proc_handle) = match io_mode {
            IoMode::Capture => {
                let lp = log_path.as_ref().unwrap();
                match process::launch_suspended_captured(&cmd, &args, effective_cwd, Path::new(lp)) {
                    Ok(v) => v,
                    Err(e) => {
                        self.jobs.close_job(&job_name);
                        return Response::Error {
                            message: format!("Failed to launch process: {e}"),
                        };
                    }
                }
            }
            IoMode::Transparent => {
                // Daemon-side transparent doesn't make sense (daemon has no terminal).
                // Fall back to null stdio (the real transparent path goes through Register).
                match process::launch_suspended(&cmd, &args, effective_cwd) {
                    Ok(v) => v,
                    Err(e) => {
                        self.jobs.close_job(&job_name);
                        return Response::Error {
                            message: format!("Failed to launch process: {e}"),
                        };
                    }
                }
            }
        };

        let pid = child.id();

        if let Err(e) = self.jobs.assign_process(&job_name, proc_handle) {
            let _ = process::terminate_process(pid);
            self.jobs.close_job(&job_name);
            return Response::Error {
                message: format!("Failed to assign process to job: {e}"),
            };
        }

        if let Err(e) = process::resume_process(pid) {
            self.jobs.terminate_job(&job_name).ok();
            return Response::Error {
                message: format!("Failed to resume process: {e}"),
            };
        }

        let record = AppRecord {
            id: id.clone(),
            name: name.clone(),
            pid,
            agent,
            group_name: group,
            cmd,
            args_json: serde_json::to_string(&args).unwrap_or_default(),
            cwd,
            started_at: Utc::now(),
            status: AppStatus::Running,
            job_name: Some(job_name),
            last_seen_at: Some(Utc::now()),
            kill_code,
            io_mode,
            log_path,
            restart,
            watch_exe: watch_exe.clone(),
        };

        if let Err(e) = self.registry.insert_app(&record) {
            return Response::Error {
                message: format!("Failed to persist record: {e}"),
            };
        }

        // Start file watcher if watch_exe is set
        if let Some(ref exe_path) = watch_exe {
            self.start_exe_watcher(&id, exe_path);
        }

        Response::Started { id, name, pid }
    }

    fn handle_register(
        &self,
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
    ) -> Response {
        if let Some(ref code) = kill_code {
            if code.len() != 4 || !code.chars().all(|c| c.is_ascii_digit()) {
                return Response::Error {
                    message: "Kill code must be exactly 4 digits".to_string(),
                };
            }
        }

        match self.registry.find_by_name(&name) {
            Ok(Some(_)) => {
                return Response::Error {
                    message: format!("An app named '{name}' is already running."),
                };
            }
            Err(e) => {
                return Response::Error {
                    message: format!("Registry error: {e}"),
                };
            }
            _ => {}
        }

        let record = AppRecord {
            id: id.clone(),
            name: name.clone(),
            pid,
            agent,
            group_name: group,
            cmd,
            args_json: serde_json::to_string(&args).unwrap_or_default(),
            cwd,
            started_at: Utc::now(),
            status: AppStatus::Running,
            job_name: Some(job_name),
            last_seen_at: Some(Utc::now()),
            kill_code,
            io_mode,
            log_path: None,
            restart: false,
            watch_exe: None,
        };

        if let Err(e) = self.registry.insert_app(&record) {
            return Response::Error {
                message: format!("Failed to persist record: {e}"),
            };
        }

        Response::Started { id, name, pid }
    }

    fn handle_list(
        &self,
        agent: Option<String>,
        group: Option<String>,
        _json: bool,
    ) -> Response {
        let _ = self.reconcile();

        let apps = if let Some(ref a) = agent {
            self.registry.find_by_agent(a)
        } else if let Some(ref g) = group {
            self.registry.find_by_group(g)
        } else {
            self.registry.list_running()
        };

        match apps {
            Ok(apps) => Response::AppList { apps },
            Err(e) => Response::Error {
                message: format!("Failed to list apps: {e}"),
            },
        }
    }

    fn handle_stop(
        &self,
        name: Option<String>,
        id: Option<String>,
        pid: Option<u32>,
        agent: Option<String>,
        group: Option<String>,
        code: Option<String>,
    ) -> Response {
        let _ = self.reconcile();

        let targets: Vec<AppRecord> = if let Some(ref n) = name {
            match self.registry.find_by_name(n) {
                Ok(Some(app)) => vec![app],
                Ok(None) => {
                    return Response::Error {
                        message: format!("No running app named '{n}'"),
                    }
                }
                Err(e) => {
                    return Response::Error {
                        message: e.to_string(),
                    }
                }
            }
        } else if let Some(ref i) = id {
            match self.registry.find_by_id(i) {
                Ok(Some(app)) => vec![app],
                Ok(None) => {
                    return Response::Error {
                        message: format!("No running app with id '{i}'"),
                    }
                }
                Err(e) => {
                    return Response::Error {
                        message: e.to_string(),
                    }
                }
            }
        } else if let Some(p) = pid {
            match self.registry.find_by_pid(p) {
                Ok(Some(app)) => vec![app],
                Ok(None) => {
                    return Response::Error {
                        message: format!("No visor-tracked app with PID {p}"),
                    }
                }
                Err(e) => {
                    return Response::Error {
                        message: e.to_string(),
                    }
                }
            }
        } else if let Some(ref a) = agent {
            self.registry.find_by_agent(a).unwrap_or_default()
        } else if let Some(ref g) = group {
            self.registry.find_by_group(g).unwrap_or_default()
        } else {
            return Response::Error {
                message: "No target specified for stop".to_string(),
            };
        };

        let is_master = code.as_deref() == Some(MASTER_KILL_CODE);
        let mut stopped_names = Vec::new();
        let mut rejected_names = Vec::new();

        for app in &targets {
            if let Some(ref app_code) = app.kill_code {
                if !is_master && code.as_deref() != Some(app_code.as_str()) {
                    rejected_names.push(app.name.clone());
                    continue;
                }
            }
            self.stop_app(app);
            stopped_names.push(app.name.clone());
        }

        if !rejected_names.is_empty() {
            return Response::Error {
                message: format!(
                    "Kill code required or incorrect for: {}. Stopped: {}",
                    rejected_names.join(", "),
                    if stopped_names.is_empty() { "none".to_string() } else { stopped_names.join(", ") }
                ),
            };
        }

        Response::Stopped {
            count: stopped_names.len(),
            names: stopped_names,
        }
    }

    fn handle_stop_all(&self, code: Option<String>) -> Response {
        let _ = self.reconcile();
        let apps = self.registry.list_running().unwrap_or_default();
        let is_master = code.as_deref() == Some(MASTER_KILL_CODE);
        let mut stopped_names = Vec::new();
        let mut rejected_names = Vec::new();

        for app in &apps {
            if let Some(ref app_code) = app.kill_code {
                if !is_master && code.as_deref() != Some(app_code.as_str()) {
                    rejected_names.push(app.name.clone());
                    continue;
                }
            }
            self.stop_app(app);
            stopped_names.push(app.name.clone());
        }

        if !rejected_names.is_empty() {
            return Response::Error {
                message: format!(
                    "Kill code required for protected apps: {}. Stopped: {}",
                    rejected_names.join(", "),
                    if stopped_names.is_empty() { "none".to_string() } else { stopped_names.join(", ") }
                ),
            };
        }

        Response::Stopped {
            count: stopped_names.len(),
            names: stopped_names,
        }
    }

    fn handle_cleanup(&self) -> Response {
        match self.reconcile() {
            Ok(removed) => Response::Cleaned { removed },
            Err(e) => Response::Error {
                message: format!("Cleanup failed: {e}"),
            },
        }
    }

    fn handle_status(&self) -> Response {
        let _ = self.reconcile();
        let active = self.registry.list_running().map(|a| a.len()).unwrap_or(0);
        let agents = self.registry.count_distinct_agents().unwrap_or(0);
        let groups = self.registry.count_distinct_groups().unwrap_or(0);

        Response::Status {
            daemon_running: true,
            active_apps: active,
            active_agents: agents,
            active_groups: groups,
            db_path: DB_PATH.to_string(),
            pipe_name: PIPE_NAME.to_string(),
        }
    }

    fn handle_attach(&self, name: Option<String>, id: Option<String>) -> Response {
        let _ = self.reconcile();

        let app = if let Some(ref n) = name {
            self.registry.find_by_name(n).ok().flatten()
        } else if let Some(ref i) = id {
            self.registry.find_by_id(i).ok().flatten()
        } else {
            None
        };

        match app {
            Some(app) => {
                if app.io_mode == IoMode::Transparent {
                    return Response::Error {
                        message: format!("'{}' is running in transparent mode — output is in the original terminal, not captured.", app.name),
                    };
                }
                match app.log_path {
                    Some(ref lp) => Response::AttachInfo {
                        log_path: lp.clone(),
                        name: app.name,
                    },
                    None => Response::Error {
                        message: format!("No log file for '{}'", app.name),
                    },
                }
            }
            None => Response::Error {
                message: "App not found".to_string(),
            },
        }
    }

    fn handle_scan(&self, path: String) -> Response {
        let projects = scanner::scan(&path);
        Response::ScanResult { projects }
    }

    fn handle_app_add(&self, profile: AppProfile) -> Response {
        match self.registry.save_app(&profile) {
            Ok(()) => Response::AppProfile { profile },
            Err(e) => Response::Error { message: format!("Failed to save app: {e}") },
        }
    }

    fn handle_app_list(&self) -> Response {
        match self.registry.list_saved_apps() {
            Ok(profiles) => Response::AppProfiles { profiles },
            Err(e) => Response::Error { message: format!("Failed to list apps: {e}") },
        }
    }

    fn handle_app_get(&self, name: String) -> Response {
        match self.registry.get_saved_app(&name) {
            Ok(Some(profile)) => Response::AppProfile { profile },
            Ok(None) => Response::Error { message: format!("No saved app '{name}'") },
            Err(e) => Response::Error { message: e.to_string() },
        }
    }

    fn handle_app_remove(&self, name: String) -> Response {
        match self.registry.remove_saved_app(&name) {
            Ok(true) => Response::Ok { message: format!("Removed '{name}'") },
            Ok(false) => Response::Error { message: format!("No saved app '{name}'") },
            Err(e) => Response::Error { message: e.to_string() },
        }
    }

    fn handle_app_activity(&self, name: String) -> Response {
        match self.registry.get_saved_app(&name) {
            Ok(Some(profile)) => {
                let act = activity::check_activity(&profile.id, &profile.path);
                Response::AppActivityResult { activity: act }
            }
            Ok(None) => Response::Error { message: format!("No saved app '{name}'") },
            Err(e) => Response::Error { message: e.to_string() },
        }
    }

    fn handle_app_run_cmd(&self, app_name: String, cmd_index: usize) -> Response {
        let profile = match self.registry.get_saved_app(&app_name) {
            Ok(Some(p)) => p,
            Ok(None) => return Response::Error { message: format!("No saved app '{app_name}'") },
            Err(e) => return Response::Error { message: e.to_string() },
        };

        let cmd = match profile.commands.get(cmd_index) {
            Some(c) => c,
            None => return Response::Error { message: format!("Command index {cmd_index} out of range") },
        };

        // Generate a unique run name
        let run_name = format!("{}-{}", app_name, cmd.category);

        self.handle_start(
            cmd.cmd.clone(),
            cmd.args.clone(),
            run_name,
            None,
            Some(app_name),
            Some(cmd.cwd.clone()),
            None,
            IoMode::Capture,
            false,
            None,
        )
    }

    /// Reconcile and auto-restart apps that have restart=true.
    /// Returns list of app IDs that were restarted.
    pub fn reconcile_and_restart(&self) -> Vec<String> {
        let apps = self.registry.list_running().unwrap_or_default();
        let mut restarted = Vec::new();

        for app in &apps {
            if !process::is_process_alive(app.pid) {
                if let Some(ref jn) = app.job_name {
                    self.jobs.close_job(jn);
                }

                if app.restart {
                    // Relaunch the process with the same command
                    if let Ok(()) = self.restart_app(app) {
                        restarted.push(app.id.clone());
                        continue;
                    }
                }

                let _ = self.registry.update_status(&app.id, &AppStatus::Dead);
            }
        }

        restarted
    }

    pub fn restart_app(&self, app: &AppRecord) -> Result<()> {
        let args: Vec<String> = serde_json::from_str(&app.args_json).unwrap_or_default();
        let job_name = app.id.clone();

        // Create new job object
        self.jobs.create_job(&job_name)?;

        let log_path = app.log_path.clone();
        let effective_cwd = app.cwd.as_deref();

        let (child, proc_handle) = match app.io_mode {
            IoMode::Capture => {
                if let Some(ref lp) = log_path {
                    process::launch_suspended_captured(&app.cmd, &args, effective_cwd, Path::new(lp))?
                } else {
                    process::launch_suspended(&app.cmd, &args, effective_cwd)?
                }
            }
            IoMode::Transparent => {
                process::launch_suspended(&app.cmd, &args, effective_cwd)?
            }
        };

        let pid = child.id();
        self.jobs.assign_process(&job_name, proc_handle)?;
        process::resume_process(pid)?;

        self.registry.update_pid_and_status(&app.id, pid, &AppStatus::Running, Some(&job_name))?;

        Ok(())
    }

    /// Start a background thread that watches an exe file and restarts the app when it changes.
    fn start_exe_watcher(&self, _app_id: &str, _exe_path: &str) {
        // The daemon loop handles this via poll-based checking.
        // See daemon.rs watch_exe_poll().
    }

    pub fn stop_app_public(&self, app: &AppRecord) {
        self.stop_app(app);
    }

    fn stop_app(&self, app: &AppRecord) {
        if let Some(ref jn) = app.job_name {
            let _ = self.jobs.terminate_job(jn);
        } else {
            let _ = process::terminate_process(app.pid);
        }
        let _ = self.registry.update_status(&app.id, &AppStatus::Stopped);
    }
}
