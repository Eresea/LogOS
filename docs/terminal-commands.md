# LogOS terminal commands

These are the commands currently built into the LogOS terminal. Type a command and press Enter.

| Command | Behavior |
| --- | --- |
| `help [command]` | Lists commands or shows a command manual. |
| `echo <text>` | Prints `<text>`. |
| `clear` | Clears the terminal display. |
| `true` | Succeeds without output. |
| `false` | Fails without output. |
| `version` | Prints the LogOS version. |
| `uname` | Prints the operating-system name. |
| `ls [path]` | Lists the root or a directory's files. |
| `touch <path>` | Creates an empty file. |
| `cat <path>` | Prints a file's contents. |
| `write <path> <data>` | Atomically replaces a file's contents. |
| `rm <path>` | Removes a file. |
| `mv <from> <to>` | Renames a file. |
| `service list` | Lists managed services and their states. |
| `service status <name>` | Shows one managed service state. |
| `service start <name>` | Starts a stopped managed service when dependencies are running. |
| `service stop <name>` | Stops a managed service without active dependents. |
| `service restart <name>` | Restarts a service and its running dependents. |
| `net status` | Reports Network state, profile, and readiness. |
| `net ping <ipv4>` | Sends one bounded ICMP echo request through Network. |
| `net tcp-probe <ipv4> <port>` | Opens one bounded TCP probe through Network. |

Examples:

```text
help
help write
echo hello, LogOS
clear
true
false
version
uname
ls /
touch /notes
write /notes durable data
cat /notes
mv /notes /archive
rm /archive
service list
service status storage
service restart storage
net status
net ping 10.0.2.2
net tcp-probe 10.0.2.2 80
```

The command line is bounded to 256 bytes and command output to 512 bytes. Pipelines, redirection,
environment variables, directories, and external programs are not implemented yet. File paths are
root-relative when written without a leading `/`; there is no current-directory state. Each mutating
file command commits one bounded Storage transaction; transaction controls are API-only. Unknown
commands print `command not found`. Network commands remain available when Network is disabled or
restarting; they return a bounded status such as `disabled`, `unavailable`, or `configuring`.

## Deferred command progress

Long-running commands currently wait for their service response before producing output. A later
command-transaction slice should give each action a bounded request ID, stage, and deadline, then
publish an immediate progress state to Terminal (for example, a loading indicator) before replacing
it with the final result. The design must keep one fixed command slot, explicit timeout/cancel
outcomes, and stale-response rejection; this is deferred until the command/Terminal IPC contract is
extended.
