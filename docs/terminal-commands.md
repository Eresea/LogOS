# LogOS terminal commands

These are the commands currently built into the LogOS terminal. Type a command and press Enter.

| Command | Behavior |
| --- | --- |
| `help()` / `help("command")` | Lists commands or shows a command manual. |
| `echo("text")` | Prints `text`. |
| `clear()` | Clears the terminal display. |
| `true()` | Succeeds without output. |
| `false()` | Fails without output. |
| `sys.version()` | Prints the LogOS version. |
| `sys.uname()` | Prints the operating-system name. |
| `sys.shutdown()` | Requests a system shutdown. |
| `sys.reboot()` | Requests a system reboot. |
| `fs.list()` / `fs.list("path")` | Lists the root or a directory's files. |
| `fs.create("path")` | Creates an empty file. |
| `fs.read("path")` | Prints a file's contents. |
| `fs.write("path", "data")` | Atomically replaces a file's contents. |
| `fs.remove("path")` | Removes a file. |
| `fs.move("from", "to")` | Renames a file. |
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
| `net.fetch("url", "path")` | Downloads a numeric-IPv4 HTTP/1.1 response and atomically stores it. |
| `net.interface["name"].status` | Reads the selected network interface state. |

Examples:

```text
help()
help("write")
echo("hello, LogOS")
clear()
true()
false()
sys.version()
sys.uname()
fs.list()
fs.create("/notes")
fs.write("/notes", "durable data")
fs.read("/notes")
fs.move("/notes", "/archive")
fs.remove("/archive")
service["storage"].status
service["storage"].restart()
net.status
net.ping("10.0.2.2")
net.tcp-probe("10.0.2.2", 80)
net.fetch("http://10.0.2.2:8080/readme", "/readme")
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

## Targeted completion

Completion is enabled by default for each Session, but it is an optional Commands-owned sub-service
over the existing Session↔Commands queues. Tab requests only the expression fragment at the cursor.
The selected candidate is shown as a dim inline suffix; `↑` / `↓` change the selected ghost,
Tab accepts, and Escape dismisses. Printable input dismisses the current suggestion and continues
editing.
Late responses are ignored when their request ID or line revision is stale.

The current targets are root expressions (`he` → `help()`, `serv` → `service["`), live service
registry names, service members (`status`, `name`, `version`, `start()`, `stop()`, `restart()`),
system members (`version()`, `uname()`, `shutdown()`, `reboot()`), filesystem members
(`list()`, `create()`, `read()`, `write()`, `remove()`, `move()`), network members (`status`,
`ping()`, `tcp-probe()`, `interface["`), and the fixed `eth0` interface entry. Filesystem paths
and arbitrary method arguments remain deferred. Candidate payloads are fixed-size and bounded. A
provider error or timeout prints `completion unavailable` once and disables completion for that
Session; command editing and execution continue normally.

## Fetch progress and cancellation

`net.fetch` owns one fixed operation. Session renders connection, request, download, staging, and
commit progress without opening a new prompt. While it runs, ordinary input is ignored; Ctrl-C sends
the active request ID to Fetch and the final result is `fetch cancelled`. Fetch accepts only numeric
IPv4 `http://` URLs, port 80 by default, bounded origin paths, and normalizes a relative destination
such as `readme` to `/readme`. It accepts 2xx responses, Content-Length or
chunked bodies, and the current Storage file limit. Redirects, TLS, compression, trailers, and
partial destinations are rejected.

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
