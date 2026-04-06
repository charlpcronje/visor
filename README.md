# Visor

A Windows process supervisor for AI-launched apps, built in Rust.

Visor is a **single-instance background supervisor with a thin CLI client**. It launches, tracks, and manages processes across terminal sessions. Anything started through Visor can be listed, checked, and stopped later without Visor needing to remain open in the foreground.

## Installation

```bash
cargo build --release
copy target\release\visor.exe C:\dev\scripts\visor.exe
```

Or use the build script:

```powershell
.\build.ps1
```

## Quick Start

```bash
# Just run visor — if you're at a terminal, the GUI dashboard opens automatically
visor

# Start a process (transparent by default — inherits your terminal)
visor start --name api python server.py

# Start backgrounded with output captured to a log file
visor start --name worker --mode capture node worker.js

# List running apps
visor list

# Stop an app
visor stop api
```

## Smart Launch

When you run `visor` with no arguments:
- **Interactive terminal** (you typing) → opens the GUI dashboard
- **Piped/automated** (agent or script) → shows the help text

This means you can just type `visor` to get the dashboard, while agents always get the CLI reference.

## Architecture

- **CLI client**: Short-lived command that sends instructions to the daemon and exits
- **Background daemon**: Persistent hidden process that owns state, tracking, and process control

The daemon starts automatically on first use. Subsequent CLI calls connect via a Windows named pipe (`\\.\pipe\visor-control`). State is persisted in SQLite (`C:\dev\scripts\visor.db`).

## Commands

| Command | Description |
|---------|-------------|
| `visor` | GUI (interactive) or help (piped) |
| `visor status` | Daemon health and summary |
| `visor start` | Start a process through visor |
| `visor list` | List running tracked apps |
| `visor stop` | Stop a tracked app |
| `visor stop-all` | Stop all tracked apps |
| `visor cleanup` | Remove stale entries |
| `visor attach` | Tail a captured app's live output |
| `visor logs` | Show log file path |
| `visor serve` | Start a static file server |
| `visor gui` | Open the GUI dashboard on a specific port |
| `visor help-all` | Full reference with all options and examples |

## Starting Processes

```bash
# Transparent mode (default) — inherits your terminal, stdin/stdout work normally
visor start --name api python server.py
visor start --name dev npm run dev
visor start --name app C:\path\to\app.exe --some-flag

# Capture mode — backgrounded, output logged, no stdin
visor start --name worker --mode capture node worker.js

# With metadata for filtering
visor start --name frontend --agent claude --group project-x npm run dev

# With kill code protection (4-digit code required to stop)
visor start --name critical --kill-code 1234 python server.py

# Auto-restart on exit
visor start --name resilient --restart --mode capture python server.py

# Hot-reload: watch an exe and restart when it's recompiled
visor start --name myapp --watch-exe C:\project\target\debug\myapp.exe --mode capture C:\project\target\debug\myapp.exe
```

## I/O Modes

| Mode | Behavior |
|------|----------|
| `transparent` (default) | Process gets a real pseudo-terminal (ConPTY). stdin/stdout/stderr work exactly like running the command directly. Libraries like rustyline, crossterm see a real TTY. |
| `capture` | Output saved to `C:\dev\scripts\visor-logs\<id>.log`. No stdin. Use `visor attach` to view live output. Only for headless workers and background services. |

**Do NOT use `--mode capture` for interactive apps** — they will exit immediately because there is no stdin.

## Auto-Restart

```bash
# Restart automatically when the process exits
visor start --name api --restart --mode capture python server.py
```

The daemon checks every 2 seconds and relaunches dead apps that have `--restart`.

## Hot-Reload (Watch Exe)

```bash
# Restart when the executable file is overwritten (e.g. after recompilation)
visor start --name myapp --watch-exe C:\project\target\debug\myapp.exe --mode capture C:\project\target\debug\myapp.exe
```

When the watched file changes:
1. Visor stops the running process
2. Waits for the file to stabilize (not mid-write)
3. Restarts with the same command and arguments

## Static File Server

```bash
# Serve current directory on a random free port
visor serve

# Specify path, port, and name
visor serve --path C:\projects\site --port 3000 --name mysite
```

Each `visor serve` picks a unique random port unless `--port` is given. Serves `index.html` if present, otherwise shows a directory listing.

## Kill Code Protection

```bash
# Protect an app with a 4-digit code
visor start --name important --kill-code 1234 --mode capture python server.py

# Stop requires the correct code
visor stop important --code 1234

# Master code (4334) always works
visor stop important --code 4334
visor stop-all --code 4334
```

## Filtering and Bulk Operations

```bash
visor list --agent claude         # filter by agent
visor list --group project-x      # filter by group
visor list --json                 # JSON output

visor stop --agent claude         # stop all for an agent
visor stop --group project-x      # stop all in a group
visor stop-all                    # stop everything
```

## Attaching to Captured Output

```bash
visor attach api                  # tail live output
visor attach api --history        # full output from start
visor logs api                    # show log file path
```

## GUI Dashboard

```bash
visor gui                         # open on default port 9847
visor gui --port 4173             # custom port
```

The dashboard includes:
- Live process list with stats (uptime, mode, agent, group)
- Stop/cleanup controls per-process and bulk
- History view of all past processes
- **Embedded xterm.js terminal** for viewing captured process output live
- **File server controls** — host any directory with one click
- **Project scanner** — detects package.json, Cargo.toml, pyproject.toml, go.mod, deno.json, composer.json and shows runnable commands (npm run dev, cargo build, etc.) that you can launch with a click

## Process Safety

Visor uses **Windows Job Objects** for safe process containment:

- Each tracked app gets its own Job Object
- Stop operations terminate only the target and its descendants
- **Never** kills parent shells, VS Code, or unrelated ancestors
- Transparent mode uses **ConPTY** for a real pseudo-terminal

## Module Layout

| File | Purpose |
|------|---------|
| `src/main.rs` | CLI entrypoint, smart launch detection |
| `src/cli.rs` | Command parsing (clap) with tiered help |
| `src/client.rs` | Connect to daemon, ConPTY transparent mode |
| `src/daemon.rs` | Daemon loop, auto-restart, exe watcher |
| `src/supervisor.rs` | Request handling, reconciliation, restart logic |
| `src/job.rs` | Windows Job Object management |
| `src/process.rs` | Process launching and state checks |
| `src/pty.rs` | ConPTY pseudo-terminal for transparent mode |
| `src/registry.rs` | SQLite persistence |
| `src/ipc.rs` | Named pipe transport |
| `src/models.rs` | Shared data structures |
| `src/gui.rs` | WebView2 GUI server + HTTP API |
| `src/dashboard.html` | Dashboard SPA (xterm.js, project scanner) |
| `src/fileserver.rs` | Static file server |
| `src/scanner.rs` | Project detector (node, rust, python, go, deno, php) |

## Requirements

- Windows 10/11
- Rust stable
- WebView2 Runtime (for GUI, pre-installed on Windows 11)

## License

MIT
