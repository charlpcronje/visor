# Visor — Unified Specification and Usage

## Product Name

**visor**

Binary path:

`C:\dev\scripts\visor.exe`

The build process must produce the executable at exactly that path by default, or provide a build step that copies the final binary there automatically.

---

## Purpose

Visor is a Windows Rust process supervisor for AI-launched apps.

It must:

* launch apps
* track only the apps launched through Visor
* persist their records across runs
* run in the background without blocking the terminal
* allow later CLI calls to reconnect to the same existing background supervisor
* list only currently running tracked apps
* remove stale/dead entries automatically
* kill only the intended target processes and their descendants
* never kill the parent shell, VS Code terminal host, or unrelated ancestor processes

Core guarantee:

**Anything started through Visor can be listed, checked, and stopped later without Visor needing to remain open in the foreground.**

---

## High-Level Behavior

Visor is not just a CLI utility. It is a **single-instance background supervisor with a CLI front-end**.

When `visor` is run from PowerShell, Command Prompt, VS Code terminal, or by another app:

* it must connect to the already-running Visor background instance if one exists
* if none exists, it must start the background instance and then hand off the command to it
* the terminal command must return promptly
* it must not sit there waiting
* it must not require Ctrl+C
* it must not open duplicate supervisor instances

Visor therefore has two modes internally:

### CLI client mode

A short-lived command invocation that sends instructions to the supervisor and exits.

### Background daemon mode

A persistent hidden local process that keeps the registry, process tracking, and control state alive.

The user interacts with the CLI. The background daemon does the actual work.

---

## Core Process Model

Visor must only manage processes that were started through Visor.

It must not attempt to own the entire Windows process tree.

Each launched app must have:

* internal id
* name
* pid
* optional agent
* optional group
* command
* args
* cwd
* start timestamp
* state
* job object association

Tracked processes must be persisted to disk.

When Visor starts or reconnects, it must re-check all saved records and remove or mark any that are no longer running.

The default list view must show only apps that are actually still alive.

---

## Single-Instance Background Supervisor

Visor must behave as a singleton.

If `visor` is run multiple times:

* only one background supervisor instance may exist
* subsequent calls must attach to the existing instance
* they must send the requested command to that instance
* they must then exit

Implementation requirement:

Use one local single-instance locking mechanism, such as:

* named mutex
* lock file plus PID validation
* named pipe endpoint presence check

Recommended Windows-native design:

* named mutex for singleton
* named pipe for command transport between CLI invocations and daemon
* SQLite for persistent registry

---

## Background Operation Requirements

When the user runs a command like:

`visor start python server.py --name api`

the behavior must be:

1. CLI process starts
2. it connects to existing Visor daemon, or starts one if missing
3. command is sent to daemon
4. daemon launches and tracks process
5. CLI prints result
6. CLI exits immediately

The daemon remains alive in the background.

The terminal must not be held open.

The user must never need to press Ctrl+C just because Visor was used.

---

## Persistence Requirements

Visor must persist all tracked session metadata to disk.

Recommended persistent storage:

* SQLite database for authoritative registry
* optional JSON export for debugging or recovery

Recommended location:

`C:\dev\scripts\visor.db`

Optional auxiliary files:

* `C:\dev\scripts\visor.log`
* `C:\dev\scripts\visor.pid`

On each daemon startup or reconnect cycle, Visor must:

1. load saved tracked records
2. check whether each PID still exists
3. confirm the process is still the same process where possible
4. remove or mark dead entries
5. only expose currently running apps in normal list output

The live list must therefore always reflect reality, not stale history.

---

## Parent/Ancestor Safety Requirement

This is critical.

Visor must **never kill parent processes or unrelated ancestor processes**.

If a tracked child process was launched from a terminal inside VS Code, and the user later stops that tracked app, Visor must kill:

* the target process itself
* optionally its descendant processes that it spawned

Visor must **not** kill:

* the parent PowerShell
* the parent cmd.exe
* the VS Code terminal host
* VS Code itself
* any unrelated parent or sibling processes

Visor is only allowed to terminate downward into the target’s own controlled descendants, never upward.

To enforce this:

* Visor must use Windows Job Objects for its launched targets
* kill operations must target the specific tracked process or the specific Visor-created Job Object
* Visor must not do naive “kill process tree upward/downward by parent PID walking”
* Visor must not terminate based on shell ancestry
* Visor must not attach parent shells to job termination logic

This is non-negotiable.

---

## Job Object Model

Each launched app must be placed into a Visor-controlled Windows Job Object.

Recommended model:

* one Job Object per named app instance, or
* one Job Object per logical group if group-wide lifecycle is desired

Default recommendation:

* one Job Object per launched app record
* optional grouping metadata for bulk operations

This provides:

* clean descendant containment
* safe subtree termination
* no ancestor termination
* reliable cleanup

Required Windows APIs:

* `CreateJobObject`
* `AssignProcessToJobObject`
* `TerminateJobObject`
* `OpenProcess`
* `TerminateProcess`

---

## App Naming and Handles

Each started app should have a friendly name.

Example:

`visor start python server.py --name api`

That name becomes the primary user-facing handle.

Visor must support resolving:

* by name
* by internal id
* by pid
* by agent
* by group

Recommended rule:
names should be unique among currently running tracked apps unless explicitly overridden.

If duplicate names are allowed, then commands must either:

* fail with ambiguity
* or require id/group filtering

Safer default:
disallow duplicate active names unless `--allow-duplicate-name` is explicitly passed.

---

## CLI Command Design

Binary name:

`visor`

Executable path:

`C:\dev\scripts\visor.exe`

Primary usage style:

```bash
visor help
visor version
visor status
visor daemon
visor start <cmd> [args...] --name <name> [--agent <agent>] [--group <group>] [--cwd <path>]
visor list
visor list --agent <agent>
visor list --group <group>
visor list --json
visor stop <name>
visor stop --id <id>
visor stop --pid <pid>
visor stop --agent <agent>
visor stop --group <group>
visor stop-all
visor cleanup
```

---

## Command Semantics

### `visor daemon`

Starts the background daemon if not already running.

If already running:

* return success
* optionally say daemon already active

This command should not start a second instance.

### `visor start <cmd> [args...] --name <name>`

Starts a process through the daemon.

Behavior:

* connect to daemon
* create tracked record
* launch process
* assign it to Job Object
* persist metadata
* return name, id, pid

Example:

```bash
visor start python server.py --name api
visor start npm run dev --name frontend --agent claude --group project-x
visor start "C:\Program Files\MyApp\app.exe" --name myapp
```

### `visor list`

Lists only currently running Visor-tracked apps.

Before returning results:

* validate all persisted entries
* remove stale entries from active set

Default columns:

* name
* id
* pid
* agent
* group
* command
* cwd
* started_at

### `visor stop <name>`

Stops the tracked app with that name.

Behavior:

1. resolve tracked record
2. attempt polite shutdown if appropriate
3. if needed, terminate the tracked process or its exact Job Object
4. remove dead entry from active list
5. do not affect parent shell or unrelated processes

### `visor stop --id <id>`

Stops a specific tracked record.

### `visor stop --pid <pid>`

Stops the tracked app with that PID if it is a Visor-owned PID.
Safer default:
if PID is not Visor-owned, reject unless `--force-untracked` exists and is explicitly used.

### `visor stop --agent <agent>`

Stops all tracked apps for that agent.

### `visor stop --group <group>`

Stops all tracked apps in that group.

### `visor stop-all`

Stops all Visor-tracked apps.

### `visor cleanup`

Forces reconciliation:

* checks all persisted entries
* removes dead entries
* leaves only truly running records

### `visor status`

Shows daemon health and summary:

* daemon running or not
* active app count
* active agent count
* active group count
* database path
* pipe/mutex health

---

## Daemon Startup and Reconnect Rules

When any Visor CLI command is executed:

1. check for existing daemon
2. if daemon exists, connect to it
3. if daemon does not exist, start daemon in background
4. wait briefly until daemon control endpoint is ready
5. send command
6. receive response
7. exit

Daemon startup must be non-blocking from user perspective.

Recommended implementation:

* spawn detached background process with `--daemon-internal`
* daemon hides console window or runs without interactive console
* client waits only long enough to hand off command

---

## Hidden Internal Mode

Visor may use an internal hidden mode not meant for normal user use:

```bash
visor --daemon-internal
```

That mode:

* starts the singleton daemon loop
* binds the named pipe or localhost endpoint
* loads and reconciles registry
* waits for commands
* never used directly by normal workflows

---

## Transport Layer

Preferred local IPC transport on Windows:

* named pipe

Possible pipe name:
`\\.\pipe\visor-control`

Alternative:

* localhost HTTP on loopback only

Named pipe is preferred because:

* Windows-native
* no open TCP port
* easy singleton command routing

---

## Registry and Storage Schema

Recommended SQLite DB path:

`C:\dev\scripts\visor.db`

Table: `apps`

Columns:

* `id` TEXT PRIMARY KEY
* `name` TEXT NOT NULL
* `pid` INTEGER NOT NULL
* `agent` TEXT NULL
* `group_name` TEXT NULL
* `cmd` TEXT NOT NULL
* `args_json` TEXT NOT NULL
* `cwd` TEXT NULL
* `started_at` TEXT NOT NULL
* `status` TEXT NOT NULL
* `job_name` TEXT NULL
* `last_seen_at` TEXT NULL

Status values:

* `running`
* `stopped`
* `dead`
* `failed`

Optional second table: `events`
for audit trail and debugging.

---

## Reconciliation Rules

Every time daemon starts, and optionally before every `list`, `status`, or `stop` operation:

1. iterate persisted active records
2. verify whether PID still exists
3. where possible, verify it still matches expected executable/start signature
4. if dead:

   * mark dead or remove from active set
5. refresh `last_seen_at`

Default user-facing behavior:
only show currently running records

Optional:
`visor list --all`
can show historical stopped/dead items if you want later

---

## Kill Logic

Kill logic must be careful and layered.

### Normal stop sequence

For one tracked target:

1. if app has a window, optionally attempt graceful close
2. wait a short timeout
3. if still alive, terminate exact tracked process or exact tracked Job Object
4. verify exit
5. mark stopped/dead

### Absolute safety rule

Visor must only kill:

* the exact tracked process
* or the exact Visor-created descendant container

It must never traverse upward.

It must never assume the parent shell should die too.

It must never kill VS Code, cmd.exe, PowerShell, Windows Terminal, or any ancestor unless that exact process was itself explicitly started and tracked by Visor under its own separate record and was the target requested by the user.

---

## Admin and Permissions

Visor should detect whether it is elevated.

Behavior:

* if not admin, continue where possible
* warn only when a requested action may fail due to permissions
* do not auto-bypass UAC
* do not silently relaunch as admin unless explicitly designed later

---

## Rust Implementation Requirements

Language:

* Rust stable

Platform:

* Windows only

Recommended crates:

* `windows`
* `rusqlite`
* `serde`
* `serde_json`
* `clap`
* `uuid`
* `chrono`
* `anyhow`
* `thiserror`

Recommended module layout:

### `src/main.rs`

CLI entrypoint and bootstrap

### `src/cli.rs`

Command parsing and client dispatch

### `src/client.rs`

Connect to daemon, send command, receive response

### `src/daemon.rs`

Daemon event loop, singleton logic, reconciliation, command handling

### `src/supervisor.rs`

Core orchestration logic

### `src/job.rs`

Windows Job Object creation, assignment, termination

### `src/process.rs`

Process launching, process state checks, graceful close handling

### `src/registry.rs`

SQLite reads/writes, reconciliation persistence

### `src/ipc.rs`

Named pipe transport

### `src/models.rs`

Shared request/response/data structures

---

## Build Requirements

Project should compile in release mode and produce:

`C:\dev\scripts\visor.exe`

Recommended build flow:

* cargo build --release
* copy resulting binary to `C:\dev\scripts\visor.exe`

Optional helper script:
`build.bat` or `build.ps1` that guarantees final placement.

---

## Usage Examples

### Start daemon implicitly

```bash
visor status
```

If daemon is not running yet:

* start it in background
* then return status

### Start an app

```bash
visor start python server.py --name api
```

### Start with grouping

```bash
visor start npm run dev --name frontend --agent claude --group project-x
```

### List running tracked apps

```bash
visor list
```

### List JSON

```bash
visor list --json
```

### Stop one app by name

```bash
visor stop api
```

### Stop all apps for one agent

```bash
visor stop --agent claude
```

### Stop one group

```bash
visor stop --group project-x
```

### Stop all tracked apps

```bash
visor stop-all
```

### Clean stale records

```bash
visor cleanup
```

---

## Expected UX

This must work:

```bash
visor start python server.py --name api
```

And then the shell prompt must return immediately.

Later:

```bash
visor list
```

Shows `api` if it is still running.

If `server.py` died by itself:

```bash
visor list
```

must not keep showing stale junk.
It should reconcile and remove or mark it dead automatically.

---

## Non-Goals

Visor is not:

* a system-wide Task Manager replacement
* a general process killer for all Windows processes
* a parent-shell killer
* a tool that should own arbitrary unrelated processes

It only manages what it started.

---

## Success Criteria

Visor is correct if:

1. only one daemon instance exists
2. CLI calls return quickly and do not block the terminal
3. tracked apps persist across CLI invocations
4. restarting or re-running `visor` reconnects to the same daemon/session
5. stale dead apps are cleaned automatically
6. `visor list` shows only real live tracked apps
7. stopping a tracked app does not kill the shell, VS Code, or other ancestors
8. all tracked apps can be stopped reliably by name, id, agent, group, or all

---

## Final Command Set

```bash
visor help
visor version
visor status
visor daemon
visor start <cmd> [args...] --name <name> [--agent <agent>] [--group <group>] [--cwd <path>]
visor list
visor list --agent <agent>
visor list --group <group>
visor list --json
visor stop <name>
visor stop --id <id>
visor stop --pid <pid>
visor stop --agent <agent>
visor stop --group <group>
visor stop-all
visor cleanup
```

---

## Final Design Statement

Visor is a **single-instance background Windows supervisor with a thin CLI client**.

The CLI is just a front door.
The daemon is the real owner of state.
Tracked apps are persisted.
Dead entries are cleaned automatically.
Only Visor-owned targets are listed and stopped.
No parent process is ever killed by mistake.
