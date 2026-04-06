//! Project scanner: finds project files and extracts runnable commands.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub path: String,
    pub kind: String,
    pub name: String,
    pub commands: Vec<ProjectCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectCommand {
    pub label: String,
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: String,
}

/// Scan a directory (and immediate subdirs) for projects.
pub fn scan(root: &str) -> Vec<Project> {
    let root = Path::new(root);
    let mut projects = Vec::new();

    // Scan root itself
    scan_dir(root, &mut projects);

    // Scan one level of subdirectories
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip common non-project dirs
                if matches!(name.as_str(), "node_modules" | ".git" | "target" | "__pycache__" | "vendor" | ".venv" | "venv" | "dist" | "build") {
                    continue;
                }
                scan_dir(&entry.path(), &mut projects);
            }
        }
    }

    projects
}

fn scan_dir(dir: &Path, projects: &mut Vec<Project>) {
    let dir_str = dir.to_string_lossy().to_string();

    // Node.js / npm
    let pkg_json = dir.join("package.json");
    if pkg_json.is_file() {
        if let Some(p) = scan_npm(&pkg_json, &dir_str) {
            projects.push(p);
        }
    }

    // Deno
    let deno_json = dir.join("deno.json");
    let deno_jsonc = dir.join("deno.jsonc");
    let deno_file = if deno_json.is_file() { Some(deno_json) }
        else if deno_jsonc.is_file() { Some(deno_jsonc) }
        else { None };
    if let Some(f) = deno_file {
        if let Some(p) = scan_deno(&f, &dir_str) {
            projects.push(p);
        }
    }

    // Rust / Cargo
    let cargo_toml = dir.join("Cargo.toml");
    if cargo_toml.is_file() {
        if let Some(p) = scan_cargo(&cargo_toml, &dir_str) {
            projects.push(p);
        }
    }

    // Python
    if let Some(p) = scan_python(dir, &dir_str) {
        projects.push(p);
    }

    // Go
    let go_mod = dir.join("go.mod");
    if go_mod.is_file() {
        if let Some(p) = scan_go(&go_mod, &dir_str) {
            projects.push(p);
        }
    }

    // PHP / Composer
    let composer_json = dir.join("composer.json");
    if composer_json.is_file() {
        if let Some(p) = scan_composer(&composer_json, &dir_str) {
            projects.push(p);
        }
    }
}

fn scan_npm(path: &Path, cwd: &str) -> Option<Project> {
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    let name = json.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("node-project")
        .to_string();

    let mut commands = Vec::new();

    // Extract scripts
    if let Some(scripts) = json.get("scripts").and_then(|v| v.as_object()) {
        // Detect package manager
        let pm = detect_node_pm(Path::new(cwd));

        for (key, val) in scripts {
            if let Some(_script) = val.as_str() {
                commands.push(ProjectCommand {
                    label: format!("{pm} run {key}"),
                    cmd: pm.clone(),
                    args: vec!["run".to_string(), key.clone()],
                    cwd: cwd.to_string(),
                });
            }
        }

        // Add install command
        commands.insert(0, ProjectCommand {
            label: format!("{pm} install"),
            cmd: pm.clone(),
            args: vec!["install".to_string()],
            cwd: cwd.to_string(),
        });
    }

    if commands.is_empty() { return None; }

    Some(Project {
        path: cwd.to_string(),
        kind: "node".to_string(),
        name,
        commands,
    })
}

fn detect_node_pm(dir: &Path) -> String {
    if dir.join("pnpm-lock.yaml").is_file() { "pnpm".to_string() }
    else if dir.join("yarn.lock").is_file() { "yarn".to_string() }
    else if dir.join("bun.lockb").is_file() { "bun".to_string() }
    else { "npm".to_string() }
}

fn scan_deno(path: &Path, cwd: &str) -> Option<Project> {
    let content = std::fs::read_to_string(path).ok()?;
    // Strip jsonc comments (simple: remove // lines)
    let cleaned: String = content.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>().join("\n");
    let json: serde_json::Value = serde_json::from_str(&cleaned).ok()?;

    let name = json.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("deno-project")
        .to_string();

    let mut commands = Vec::new();

    if let Some(tasks) = json.get("tasks").and_then(|v| v.as_object()) {
        for (key, val) in tasks {
            if let Some(_task) = val.as_str() {
                commands.push(ProjectCommand {
                    label: format!("deno task {key}"),
                    cmd: "deno".to_string(),
                    args: vec!["task".to_string(), key.clone()],
                    cwd: cwd.to_string(),
                });
            }
        }
    }

    if commands.is_empty() {
        // Default deno run for main.ts
        let main = Path::new(cwd).join("main.ts");
        if main.is_file() {
            commands.push(ProjectCommand {
                label: "deno run main.ts".to_string(),
                cmd: "deno".to_string(),
                args: vec!["run".to_string(), "--allow-all".to_string(), "main.ts".to_string()],
                cwd: cwd.to_string(),
            });
        }
    }

    if commands.is_empty() { return None; }

    Some(Project {
        path: cwd.to_string(),
        kind: "deno".to_string(),
        name,
        commands,
    })
}

fn scan_cargo(path: &Path, cwd: &str) -> Option<Project> {
    let content = std::fs::read_to_string(path).ok()?;

    // Simple TOML parsing for name
    let name = content.lines()
        .find(|l| l.starts_with("name"))
        .and_then(|l| l.split('=').nth(1))
        .map(|v| v.trim().trim_matches('"').to_string())
        .unwrap_or_else(|| "rust-project".to_string());

    let commands = vec![
        ProjectCommand {
            label: "cargo build".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
            cwd: cwd.to_string(),
        },
        ProjectCommand {
            label: "cargo build --release".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string(), "--release".to_string()],
            cwd: cwd.to_string(),
        },
        ProjectCommand {
            label: "cargo run".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["run".to_string()],
            cwd: cwd.to_string(),
        },
        ProjectCommand {
            label: "cargo run --release".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["run".to_string(), "--release".to_string()],
            cwd: cwd.to_string(),
        },
        ProjectCommand {
            label: "cargo test".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["test".to_string()],
            cwd: cwd.to_string(),
        },
        ProjectCommand {
            label: "cargo check".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["check".to_string()],
            cwd: cwd.to_string(),
        },
    ];

    Some(Project {
        path: cwd.to_string(),
        kind: "rust".to_string(),
        name,
        commands,
    })
}

fn scan_python(dir: &Path, cwd: &str) -> Option<Project> {
    let mut commands = Vec::new();
    let mut name = "python-project".to_string();

    // Check pyproject.toml
    let pyproject = dir.join("pyproject.toml");
    if pyproject.is_file() {
        if let Ok(content) = std::fs::read_to_string(&pyproject) {
            if let Some(n) = content.lines()
                .find(|l| l.starts_with("name"))
                .and_then(|l| l.split('=').nth(1))
                .map(|v| v.trim().trim_matches('"').to_string())
            {
                name = n;
            }

            // Check for scripts in pyproject.toml
            if content.contains("[tool.poetry.scripts]") || content.contains("[project.scripts]") {
                commands.push(ProjectCommand {
                    label: "pip install -e .".to_string(),
                    cmd: "pip".to_string(),
                    args: vec!["install".to_string(), "-e".to_string(), ".".to_string()],
                    cwd: cwd.to_string(),
                });
            }
        }
    }

    // Check requirements.txt
    let requirements = dir.join("requirements.txt");
    if requirements.is_file() {
        commands.push(ProjectCommand {
            label: "pip install -r requirements.txt".to_string(),
            cmd: "pip".to_string(),
            args: vec!["install".to_string(), "-r".to_string(), "requirements.txt".to_string()],
            cwd: cwd.to_string(),
        });
    }

    // Look for common entry points
    for entry in &["main.py", "app.py", "server.py", "manage.py", "run.py"] {
        if dir.join(entry).is_file() {
            let label = format!("python {entry}");
            commands.push(ProjectCommand {
                label,
                cmd: "python".to_string(),
                args: vec![entry.to_string()],
                cwd: cwd.to_string(),
            });
        }
    }

    // manage.py special commands
    if dir.join("manage.py").is_file() {
        commands.push(ProjectCommand {
            label: "python manage.py runserver".to_string(),
            cmd: "python".to_string(),
            args: vec!["manage.py".to_string(), "runserver".to_string()],
            cwd: cwd.to_string(),
        });
    }

    if commands.is_empty() { return None; }

    Some(Project {
        path: cwd.to_string(),
        kind: "python".to_string(),
        name,
        commands,
    })
}

fn scan_go(path: &Path, cwd: &str) -> Option<Project> {
    let content = std::fs::read_to_string(path).ok()?;

    let name = content.lines()
        .find(|l| l.starts_with("module "))
        .map(|l| l.strip_prefix("module ").unwrap_or("go-project").trim().to_string())
        .unwrap_or_else(|| "go-project".to_string());

    // Shorten module name to last path segment
    let short_name = name.rsplit('/').next().unwrap_or(&name).to_string();

    let commands = vec![
        ProjectCommand {
            label: "go run .".to_string(),
            cmd: "go".to_string(),
            args: vec!["run".to_string(), ".".to_string()],
            cwd: cwd.to_string(),
        },
        ProjectCommand {
            label: "go build".to_string(),
            cmd: "go".to_string(),
            args: vec!["build".to_string(), ".".to_string()],
            cwd: cwd.to_string(),
        },
        ProjectCommand {
            label: "go test ./...".to_string(),
            cmd: "go".to_string(),
            args: vec!["test".to_string(), "./...".to_string()],
            cwd: cwd.to_string(),
        },
        ProjectCommand {
            label: "go mod tidy".to_string(),
            cmd: "go".to_string(),
            args: vec!["mod".to_string(), "tidy".to_string()],
            cwd: cwd.to_string(),
        },
    ];

    Some(Project {
        path: cwd.to_string(),
        kind: "go".to_string(),
        name: short_name,
        commands,
    })
}

fn scan_composer(path: &Path, cwd: &str) -> Option<Project> {
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    let name = json.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("php-project")
        .to_string();

    let mut commands = Vec::new();

    // Composer scripts
    if let Some(scripts) = json.get("scripts").and_then(|v| v.as_object()) {
        for key in scripts.keys() {
            commands.push(ProjectCommand {
                label: format!("composer run {key}"),
                cmd: "composer".to_string(),
                args: vec!["run".to_string(), key.clone()],
                cwd: cwd.to_string(),
            });
        }
    }

    // PHP built-in server
    let public_dir = if Path::new(cwd).join("public").is_dir() { "public" }
        else if Path::new(cwd).join("web").is_dir() { "web" }
        else { "." };

    commands.push(ProjectCommand {
        label: format!("php -S localhost:8000 -t {public_dir}"),
        cmd: "php".to_string(),
        args: vec!["-S".to_string(), "localhost:8000".to_string(), "-t".to_string(), public_dir.to_string()],
        cwd: cwd.to_string(),
    });

    commands.insert(0, ProjectCommand {
        label: "composer install".to_string(),
        cmd: "composer".to_string(),
        args: vec!["install".to_string()],
        cwd: cwd.to_string(),
    });

    Some(Project {
        path: cwd.to_string(),
        kind: "php".to_string(),
        name,
        commands,
    })
}
