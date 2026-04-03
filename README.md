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
# Start a background process (capture mode - output logged to file)
visor start --name api --mode capture python server.py

# Start a transparent process (inherits your terminal)
visor start --name dev npm run dev

# List running apps
visor list

# Stop an app
visor stop api

# Check daemon status
visor status
```

## Architecture

Visor has two internal modes:

- **CLI client**: Short-lived command that sends instructions to the daemon and exits
- **Background daemon**: Persistent hidden process that owns state, tracking, and process control

The daemon starts automatically on first use. Subsequent CLI calls connect to the existing daemon via a Windows named pipe (`\\.\pipe\visor-control`). State is persisted in SQLite (`C:\dev\scripts\visor.db`).

## Commands

| Command | Description |
|---------|-------------|
| `visor` | Show help |
| `visor status` | Daemon health and summary |
| `visor daemon` | Start the background daemon explicitly |
| `visor start` | Start a process through visor |
| `visor list` | List running tracked apps |
| `visor stop` | Stop a tracked app |
| `visor stop-all` | Stop all tracked apps |
| `visor cleanup` | Force reconciliation, remove stale entries |
| `visor attach` | Attach to a captured app's live output |
| `visor logs` | Show log file path for a captured app |
| `visor gui` | Open the WebView2 GUI dashboard |

## Starting Processes

```bash
# Basic start (transparent mode - inherits terminal)
visor start --name myapp python server.py

# Capture mode (output saved to log file, runs backgrounded)
visor start --name api --mode capture python server.py

# With agent and group metadata
visor start --name frontend --agent claude --group project-x --mode capture npm run dev

# With kill code protection
visor start --name critical-api --kill-code 1234 --mode capture python server.py

# With custom working directory
visor start --name backend --cwd C:\projects\api --mode capture cargo run
```

## I/O Modes

| Mode | Behavior |
|------|----------|
| `transparent` (default) | Process inherits your terminal's stdin/stdout/stderr. Looks like running the command directly. CLI waits for the process to exit. |
| `capture` | Output is saved to a log file in `C:\dev\scripts\visor-logs\`. Process runs fully backgrounded. Use `visor attach` to view live output. |

## Kill Code Protection

Protect important processes from accidental termination:

```bash
# Start with a 4-digit kill code
visor start --name important-api --kill-code 1234 --mode capture python server.py

# Stop requires the correct code
visor stop important-api --code 1234

# Master code (4334) always works
visor stop important-api --code 4334
```

## Filtering

```bash
# List by agent
visor list --agent claude

# List by group
visor list --group project-x

# JSON output
visor list --json

# Stop all apps by agent
visor stop --agent claude

# Stop all apps in a group
visor stop --group project-x
```

## Attaching to Captured Output

```bash
# Tail live output (capture mode only)
visor attach api

# View full output history from start
visor attach api --history
```

## GUI Dashboard

```bash
# Open the WebView2 dashboard (default port 9847)
visor gui

# Custom port
visor gui --port 4173
```

The GUI provides:
- Live dashboard with all running processes
- Process stats (uptime, mode, agent, group)
- Stop/cleanup controls
- History view
- Embedded xterm.js terminal for viewing captured process output

## Process Safety

Visor uses **Windows Job Objects** for safe process containment:

- Each tracked app gets its own Job Object
- Stop operations terminate only the target process and its descendants
- Visor **never** kills parent shells, VS Code, or unrelated ancestor processes
- Only Visor-owned processes can be listed and stopped

## Module Layout

| File | Purpose |
|------|---------|
| `src/main.rs` | CLI entrypoint and bootstrap |
| `src/cli.rs` | Command parsing (clap) |
| `src/client.rs` | Connect to daemon, send commands |
| `src/daemon.rs` | Daemon event loop, singleton logic |
| `src/supervisor.rs` | Request handling, reconciliation |
| `src/job.rs` | Windows Job Object management |
| `src/process.rs` | Process launching and state checks |
| `src/registry.rs` | SQLite persistence |
| `src/ipc.rs` | Named pipe transport |
| `src/models.rs` | Shared data structures |
| `src/gui.rs` | WebView2 GUI server |

## Requirements

- Windows 10/11
- Rust stable
- WebView2 Runtime (for GUI mode, pre-installed on Windows 11)

## License

MIT
