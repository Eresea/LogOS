# LogOS terminal commands

These are the commands currently built into the LogOS terminal. Type a command and press Enter.

| Command | Behavior |
| --- | --- |
| `help` | Lists the built-in commands. |
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

Examples:

```text
help
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
```

The command line is bounded to 256 bytes and command output to 512 bytes. Pipelines, redirection,
environment variables, directories, and external programs are not implemented yet. File paths are
root-relative when written without a leading `/`; there is no current-directory state. Each mutating
file command commits one bounded Storage transaction; transaction controls are API-only. Unknown
commands print `command not found`.
