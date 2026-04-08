use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

use crate::models::{AppProfile, AppRecord, AppStatus, IoMode};

pub struct Registry {
    conn: Mutex<Connection>,
}

impl Registry {
    pub fn open(db_path: &str) -> Result<Self> {
        if let Some(parent) = Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn_inner = Connection::open(db_path)
            .with_context(|| format!("Failed to open database at {db_path}"))?;

        conn_inner.execute_batch(
            "CREATE TABLE IF NOT EXISTS apps (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                pid         INTEGER NOT NULL,
                agent       TEXT,
                group_name  TEXT,
                cmd         TEXT NOT NULL,
                args_json   TEXT NOT NULL,
                cwd         TEXT,
                started_at  TEXT NOT NULL,
                status      TEXT NOT NULL,
                job_name    TEXT,
                last_seen_at TEXT,
                kill_code   TEXT,
                io_mode     TEXT NOT NULL DEFAULT 'transparent',
                log_path    TEXT,
                restart     INTEGER NOT NULL DEFAULT 0,
                watch_exe   TEXT
            );",
        )
        .context("Failed to create apps table")?;

        conn_inner.execute_batch(
            "CREATE TABLE IF NOT EXISTS saved_apps (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL UNIQUE,
                path        TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                tags_json   TEXT NOT NULL DEFAULT '[]',
                commands_json TEXT NOT NULL DEFAULT '[]',
                created_at  TEXT NOT NULL,
                icon        TEXT
            );",
        )
        .context("Failed to create saved_apps table")?;

        Ok(Self { conn: Mutex::new(conn_inner) })
    }

    pub fn insert_app(&self, app: &AppRecord) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO apps (id, name, pid, agent, group_name, cmd, args_json, cwd, started_at, status, job_name, last_seen_at, kill_code, io_mode, log_path, restart, watch_exe)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                app.id,
                app.name,
                app.pid,
                app.agent,
                app.group_name,
                app.cmd,
                app.args_json,
                app.cwd,
                app.started_at.to_rfc3339(),
                app.status.to_string(),
                app.job_name,
                app.last_seen_at.map(|t| t.to_rfc3339()),
                app.kill_code,
                app.io_mode.to_string(),
                app.log_path,
                app.restart as i32,
                app.watch_exe,
            ],
        )?;
        Ok(())
    }

    pub fn update_pid_and_status(&self, id: &str, pid: u32, status: &AppStatus, job_name: Option<&str>) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE apps SET pid = ?1, status = ?2, last_seen_at = ?3, job_name = ?4 WHERE id = ?5",
            params![pid as i64, status.to_string(), Utc::now().to_rfc3339(), job_name, id],
        )?;
        Ok(())
    }

    pub fn update_status(&self, id: &str, status: &AppStatus) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE apps SET status = ?1, last_seen_at = ?2 WHERE id = ?3",
            params![status.to_string(), Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn list_running(&self) -> Result<Vec<AppRecord>> {
        self.list_by_status("running")
    }

    pub fn list_by_status(&self, status: &str) -> Result<Vec<AppRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, pid, agent, group_name, cmd, args_json, cwd, started_at, status, job_name, last_seen_at, kill_code, io_mode, log_path, restart, watch_exe
             FROM apps WHERE status = ?1"
        )?;
        let rows = stmt.query_map(params![status], |row| {
            Ok(AppRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                pid: row.get::<_, i64>(2)? as u32,
                agent: row.get(3)?,
                group_name: row.get(4)?,
                cmd: row.get(5)?,
                args_json: row.get(6)?,
                cwd: row.get(7)?,
                started_at: chrono::DateTime::parse_from_rfc3339(
                    &row.get::<_, String>(8)?
                )
                .unwrap_or_default()
                .with_timezone(&chrono::Utc),
                status: AppStatus::from_str(&row.get::<_, String>(9)?),
                job_name: row.get(10)?,
                last_seen_at: row.get::<_, Option<String>>(11)?
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc)),
                kill_code: row.get(12)?,
                io_mode: IoMode::from_str(&row.get::<_, String>(13).unwrap_or_else(|_| "transparent".to_string())),
                log_path: row.get(14)?,
                restart: row.get::<_, i32>(15).unwrap_or(0) != 0,
                watch_exe: row.get(16)?,
            })
        })?;

        let mut apps = Vec::new();
        for row in rows {
            apps.push(row?);
        }
        Ok(apps)
    }

    pub fn find_by_name(&self, name: &str) -> Result<Option<AppRecord>> {
        let apps = self.list_running()?;
        Ok(apps.into_iter().find(|a| a.name == name))
    }

    pub fn find_by_id(&self, id: &str) -> Result<Option<AppRecord>> {
        let apps = self.list_running()?;
        Ok(apps.into_iter().find(|a| a.id == id))
    }

    pub fn find_by_pid(&self, pid: u32) -> Result<Option<AppRecord>> {
        let apps = self.list_running()?;
        Ok(apps.into_iter().find(|a| a.pid == pid))
    }

    pub fn find_by_agent(&self, agent: &str) -> Result<Vec<AppRecord>> {
        let apps = self.list_running()?;
        Ok(apps.into_iter().filter(|a| a.agent.as_deref() == Some(agent)).collect())
    }

    pub fn find_by_group(&self, group: &str) -> Result<Vec<AppRecord>> {
        let apps = self.list_running()?;
        Ok(apps
            .into_iter()
            .filter(|a| a.group_name.as_deref() == Some(group))
            .collect())
    }

    pub fn count_distinct_agents(&self) -> Result<usize> {
        let apps = self.list_running()?;
        let agents: std::collections::HashSet<_> = apps
            .iter()
            .filter_map(|a| a.agent.as_deref())
            .collect();
        Ok(agents.len())
    }

    pub fn count_distinct_groups(&self) -> Result<usize> {
        let apps = self.list_running()?;
        let groups: std::collections::HashSet<_> = apps
            .iter()
            .filter_map(|a| a.group_name.as_deref())
            .collect();
        Ok(groups.len())
    }

    // --- Saved Apps ---

    pub fn save_app(&self, profile: &AppProfile) -> Result<()> {
        let tags_json = serde_json::to_string(&profile.tags)?;
        let cmds_json = serde_json::to_string(&profile.commands)?;
        self.conn.lock().unwrap().execute(
            "INSERT OR REPLACE INTO saved_apps (id, name, path, description, tags_json, commands_json, created_at, icon)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                profile.id,
                profile.name,
                profile.path,
                profile.description,
                tags_json,
                cmds_json,
                profile.created_at,
                profile.icon,
            ],
        )?;
        Ok(())
    }

    pub fn list_saved_apps(&self) -> Result<Vec<AppProfile>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, path, description, tags_json, commands_json, created_at, icon FROM saved_apps ORDER BY name"
        )?;
        let rows = stmt.query_map([], |row| {
            let tags_str: String = row.get(4)?;
            let cmds_str: String = row.get(5)?;
            Ok(AppProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                description: row.get(3)?,
                tags: serde_json::from_str(&tags_str).unwrap_or_default(),
                commands: serde_json::from_str(&cmds_str).unwrap_or_default(),
                created_at: row.get(6)?,
                icon: row.get(7)?,
            })
        })?;
        let mut profiles = Vec::new();
        for row in rows {
            profiles.push(row?);
        }
        Ok(profiles)
    }

    pub fn get_saved_app(&self, name: &str) -> Result<Option<AppProfile>> {
        let all = self.list_saved_apps()?;
        Ok(all.into_iter().find(|a| a.name == name))
    }

    pub fn remove_saved_app(&self, name: &str) -> Result<bool> {
        let count = self.conn.lock().unwrap().execute(
            "DELETE FROM saved_apps WHERE name = ?1",
            params![name],
        )?;
        Ok(count > 0)
    }
}
