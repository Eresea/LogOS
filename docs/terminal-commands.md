# LogOS terminal commands

These are the commands currently built into the LogOS terminal. Type a command and press Enter.

| Command | Behavior |
| --- | --- |
| `help()` / `help("command")` | Lists commands or shows a command manual. |
| `echo("text")` | Prints `text`. |
| `clear()` | Clears the terminal display. |
| `true()` | Succeeds without output. |
| `false()` | Fails without output. |
| `version()` | Prints the LogOS version. |
| `uname()` | Prints the operating-system name. |
| `ls()` / `ls("path")` | Lists the root or a directory's files. |
| `touch("path")` | Creates an empty file. |
| `cat("path")` | Prints a file's contents. |
| `write("path", "data")` | Atomically replaces a file's contents. |
| `rm("path")` | Removes a file. |
| `mv("from", "to")` | Renames a file. |
| `service["name"]` | Selects a managed service. |
| `service["name"].status` | Reads a managed service state. |
| `service["name"].name` | Reads a managed service name. |
| `service["name"].version` | Reads the service image version. |
| `service["name"].start()` | Starts a stopped managed service. |
| `service["name"].stop()` | Stops a managed service. |
| `service["name"].restart()` | Restarts a service and its running dependents. |
| `net.status` | Reads Network state and readiness. |
| `net.ping("ipv4")` | Sends one bounded ICMP echo request through Network. |
| `net.tcp-probe("ipv4", port)` | Opens one bounded TCP probe through Network. |
| `net.interface["name"].status` | Reads the selected network interface state. |

Examples:

```text
help()
help("write")
echo("hello, LogOS")
clear()
true()
false()
version()
uname()
ls()
touch("/notes")
write("/notes", "durable data")
cat("/notes")
mv("/notes", "/archive")
rm("/archive")
service["storage"].status
service["storage"].restart()
net.status
net.ping("10.0.2.2")
net.tcp-probe("10.0.2.2", 80)
net.interface["eth0"].status
```

Commands use a bounded expression grammar. `.` selects members, `[]` selects registry entries,
properties omit parentheses, methods use `()`, and strings require double quotes. Numeric literals
are currently accepted only for typed method arguments such as TCP ports. The command line is bounded
to 256 bytes and command output to 512 bytes. Pipelines, redirection, variables, conditionals,
directories, and external programs are not implemented yet. File paths are root-relative when
written without a leading `/`; there is no current-directory state. Each mutating file command
commits one bounded Storage transaction; transaction controls are API-only. Unknown commands print
`command not found`. Network commands remain available when Network is disabled or restarting; they
return a bounded status such as `disabled`, `unavailable`, or `configuring`.

## Deferred command progress

Long-running commands currently wait for their service response before producing output. A later
command-transaction slice should give each action a bounded request ID, stage, and deadline, then
publish an immediate progress state to Terminal (for example, a loading indicator) before replacing
it with the final result. The design must keep one fixed command slot, explicit timeout/cancel
outcomes, and stale-response rejection; this is deferred until the command/Terminal IPC contract is
extended.

## Deferred transient navigation mode

The terminal may offer a transient navigation UI alongside the programmable
Flow API. Holding `Alt` temporarily renders the contents of the current
filesystem location as an interactive TUI: `↑` / `↓` change selection, `→`
enters the selected directory or previews a file, and `←` moves to the parent.
The view updates immediately as traversal occurs; no `ls`, `cd`, or equivalent
commands enter terminal history. Releasing `Alt` hides the UI, returns to
command input, and keeps the navigated location.

This is the human interface (`Alt` + arrows), while Flow remains the explicit
programmable interface:

```text
fs.list()
fs["projects"].enter()
fs["notes.txt"].read()
```

The same contextual pattern could later support service browsing,
task/process and device inspection, actions on selected resources, and
autocomplete backed by live system objects. The terminal is an interactive
system interface, not merely a command line with richer syntax.
