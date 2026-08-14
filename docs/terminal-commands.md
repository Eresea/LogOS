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

Examples:

```text
help
echo hello, LogOS
clear
true
false
version
uname
```

The command line is bounded to 256 bytes and command output to 512 bytes. Pipelines, redirection,
environment variables, files, and external programs are not implemented yet. Unknown commands print
`command not found`.
