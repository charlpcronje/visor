//! Activity monitoring: check last file modification and git commit times.

use crate::models::AppActivity;
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

/// Check activity for a project at the given path.
pub fn check_activity(app_id: &str, path: &str) -> AppActivity {
    let last_file = get_last_file_modified(path);
    let (last_commit, last_message) = get_last_git_commit(path);

    // Determine staleness based on the most recent activity
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let most_recent = [
        parse_timestamp(&last_file),
        parse_timestamp(&last_commit),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);

    let age_hours = if most_recent > 0 {
        (now.saturating_sub(most_recent)) / 3600
    } else {
        999
    };

    let staleness = if age_hours < 4 {
        "active".to_string()     // green
    } else if age_hours < 24 {
        "recent".to_string()     // yellow
    } else {
        "stale".to_string()      // red
    };

    AppActivity {
        app_id: app_id.to_string(),
        last_file_modified: last_file,
        last_git_commit: last_commit,
        last_git_message: last_message,
        staleness,
    }
}

fn get_last_file_modified(path: &str) -> Option<String> {
    let dir = Path::new(path);
    if !dir.is_dir() {
        return None;
    }

    let mut newest: Option<SystemTime> = None;

    // Walk top-level files and one level of subdirs (skip big dirs)
    let skip = ["node_modules", ".git", "target", "__pycache__", "vendor", ".venv", "dist", "build", ".next"];

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if skip.contains(&name.as_str()) {
                continue;
            }
            check_entry_time(&entry, &mut newest);

            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Ok(sub_entries) = std::fs::read_dir(entry.path()) {
                    for sub in sub_entries.filter_map(|e| e.ok()) {
                        check_entry_time(&sub, &mut newest);
                    }
                }
            }
        }
    }

    newest.and_then(|t| {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
            .flatten()
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
    })
}

fn check_entry_time(entry: &std::fs::DirEntry, newest: &mut Option<SystemTime>) {
    if let Ok(meta) = entry.metadata() {
        if let Ok(modified) = meta.modified() {
            if newest.map(|n| modified > n).unwrap_or(true) {
                *newest = Some(modified);
            }
        }
    }
}

fn get_last_git_commit(path: &str) -> (Option<String>, Option<String>) {
    let output = Command::new("git")
        .args(["log", "-1", "--format=%ci|||%s"])
        .current_dir(path)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let parts: Vec<&str> = text.splitn(2, "|||").collect();
            let date = parts.first().map(|s| s.to_string());
            let msg = parts.get(1).map(|s| s.to_string());
            (date, msg)
        }
        _ => (None, None),
    }
}

fn parse_timestamp(s: &Option<String>) -> Option<u64> {
    s.as_ref().and_then(|s| {
        chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S")
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(&s.trim()[..19], "%Y-%m-%d %H:%M:%S"))
            .ok()
            .map(|dt| dt.and_utc().timestamp() as u64)
    })
}
