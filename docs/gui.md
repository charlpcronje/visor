## Web UI Feature

Visor must support an optional GUI mode via:

```bash
visor --gui:<port>
```

Example:

```bash
visor --gui:4173
```

### Behavior

* Starts the Visor local web UI server on the specified localhost port
* Opens a desktop window using Microsoft WebView2
* The UI must connect to the existing Visor daemon, not create a second supervisor instance
* If the daemon is not running, it must start it first and then connect
* The GUI must be optional; Visor must still work fully from the CLI without it

### GUI Requirements

The GUI must display all currently running Visor-managed processes, including:

* name
* internal id
* pid
* agent
* group
* executable / command
* arguments
* working directory
* start time
* uptime
* CPU usage
* memory usage
* status
* stdio mode
* restart policy if present

### GUI Actions

For each running process, the GUI must allow:

* stop
* restart
* pause, if supported
* resume, if supported
* view logs
* open details view

### Process Details View

Each process details page or panel must show:

* full command
* args
* cwd
* pid
* launch time
* uptime
* last seen time
* CPU usage
* memory usage
* stdout/stderr log access
* exit code if stopped
* restart count if applicable
* historical runs for the same named app

### History

Visor must keep a history of previously run processes.

The history must include:

* name
* agent
* group
* command
* args
* cwd
* launched_at
* exited_at
* total uptime
* exit code
* final status
* restart count
* stdio mode
* whether it was manually stopped or exited naturally

The GUI must provide a history screen that allows:

* filtering
* sorting
* viewing details
* re-running a previous app from history

### Re-run From History

The GUI must allow a user to select a previous run and launch it again using the saved command, args, cwd, and metadata.

This should be equivalent to starting the app again through Visor normally.

### Storage

All persistent data must be stored in a SQLite database in the same folder as the executable.

Expected path example:

```text
C:\dev\scripts\visor.db
```

The database must contain both:

* active tracked process records
* historical run records

### Metrics

The GUI must show live resource usage where possible, including:

* CPU usage
* memory usage
* uptime

These values should refresh automatically on a timer.

### Web Stack

* Web UI served locally by Visor
* Desktop window hosted through Microsoft WebView2
* Localhost only
* No external internet dependency required

### Optional Routes / Views

Recommended pages:

* Dashboard
* Running Processes
* Process Details
* History
* Logs

### Suggested CLI Behavior

```bash
visor --gui:4173
```

### Terminal inside webview
When I click on a process inside the web view, I should be able to say "View" or "Open as a view process". It should then show me the current status of the app, like if I've run it from a terminal. It should open a terminal with xterm or something. It should show a web socket that actually shows me the app running as if it's running in a terminal, as if I executed it from a terminal, to show me the exact std in out of that app



This should:

1. ensure the Visor daemon is running
2. start the local web server if needed
3. open the WebView2 window
4. show the live dashboard

### Notes

* The GUI must be a management view over the existing Visor supervisor
* It must not become a second source of truth
* The daemon and SQLite database remain the canonical backend
