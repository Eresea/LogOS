#![no_std]

#[cfg(test)]
extern crate std;

pub const MAX_COMMAND_BYTES: usize = 256;
pub const MAX_OUTPUT_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandKind {
    Help,
    Echo,
    Clear,
    True,
    False,
    Version,
    Uname,
    Shutdown,
    Reboot,
    List,
    Touch,
    Cat,
    Write,
    Remove,
    Move,
    Service,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum CommandAction {
    None = 0,
    Shutdown = logos_abi::POWER_SHUTDOWN,
    Reboot = logos_abi::POWER_REBOOT,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub name: &'static [u8],
    pub kind: CommandKind,
    pub usage: &'static [u8],
    pub summary: &'static [u8],
    pub manual: &'static [u8],
}

pub const COMMAND_SPECS: [CommandSpec; 16] = [
    CommandSpec {
        name: b"help",
        kind: CommandKind::Help,
        usage: b"help [command]",
        summary: b"show command help",
        manual: b"Lists commands or shows one command manual.",
    },
    CommandSpec {
        name: b"echo",
        kind: CommandKind::Echo,
        usage: b"echo <text>",
        summary: b"print text",
        manual: b"Prints the supplied text.",
    },
    CommandSpec {
        name: b"clear",
        kind: CommandKind::Clear,
        usage: b"clear",
        summary: b"clear the screen",
        manual: b"Clears the terminal display.",
    },
    CommandSpec {
        name: b"true",
        kind: CommandKind::True,
        usage: b"true",
        summary: b"succeed",
        manual: b"Completes successfully without output.",
    },
    CommandSpec {
        name: b"false",
        kind: CommandKind::False,
        usage: b"false",
        summary: b"fail",
        manual: b"Completes with a failure status.",
    },
    CommandSpec {
        name: b"version",
        kind: CommandKind::Version,
        usage: b"version",
        summary: b"show the version",
        manual: b"Prints the LogOS version.",
    },
    CommandSpec {
        name: b"uname",
        kind: CommandKind::Uname,
        usage: b"uname",
        summary: b"show the system name",
        manual: b"Prints the operating-system name.",
    },
    CommandSpec {
        name: b"shutdown",
        kind: CommandKind::Shutdown,
        usage: b"shutdown",
        summary: b"power off",
        manual: b"Requests a system shutdown.",
    },
    CommandSpec {
        name: b"reboot",
        kind: CommandKind::Reboot,
        usage: b"reboot",
        summary: b"restart",
        manual: b"Requests a system reboot.",
    },
    CommandSpec {
        name: b"ls",
        kind: CommandKind::List,
        usage: b"ls [path]",
        summary: b"list files",
        manual: b"Lists files in the root or specified directory.",
    },
    CommandSpec {
        name: b"touch",
        kind: CommandKind::Touch,
        usage: b"touch <path>",
        summary: b"create an empty file",
        manual: b"Creates an empty file at path.",
    },
    CommandSpec {
        name: b"cat",
        kind: CommandKind::Cat,
        usage: b"cat <path>",
        summary: b"print a file",
        manual: b"Prints a file's contents.",
    },
    CommandSpec {
        name: b"write",
        kind: CommandKind::Write,
        usage: b"write <path> <data>",
        summary: b"replace file contents",
        manual: b"Atomically replaces an existing file's contents. Data must be non-empty.",
    },
    CommandSpec {
        name: b"rm",
        kind: CommandKind::Remove,
        usage: b"rm <path>",
        summary: b"remove a file",
        manual: b"Removes a file.",
    },
    CommandSpec {
        name: b"mv",
        kind: CommandKind::Move,
        usage: b"mv <from> <to>",
        summary: b"rename a file",
        manual: b"Renames a file from one path to another.",
    },
    CommandSpec {
        name: b"service",
        kind: CommandKind::Service,
        usage: b"service <list|status|start|stop|restart> [name]",
        summary: b"manage services",
        manual: b"Lists, inspects, starts, stops, or restarts a service.",
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageCommand<'a> {
    List { path: &'a [u8] },
    Touch { path: &'a [u8] },
    Cat { path: &'a [u8] },
    Write { path: &'a [u8], data: &'a [u8] },
    Remove { path: &'a [u8] },
    Move { from: &'a [u8], to: &'a [u8] },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageCommandError {
    Usage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceCommand<'a> {
    List,
    Status { name: &'a [u8] },
    Start { name: &'a [u8] },
    Stop { name: &'a [u8] },
    Restart { name: &'a [u8] },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceCommandError {
    Usage,
}

/// Resolve shell paths against the fixed root because the shell has no cwd.
pub fn root_relative_path<'a>(path: &[u8], output: &'a mut [u8]) -> Option<&'a [u8]> {
    if path.is_empty() {
        return Some(&output[..0]);
    }
    let prefix = usize::from(path.first().copied() != Some(b'/'));
    let length = prefix.checked_add(path.len())?;
    if length > output.len() {
        return None;
    }
    if prefix != 0 {
        output[0] = b'/';
    }
    output[prefix..length].copy_from_slice(path);
    Some(&output[..length])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub bytes: [u8; MAX_OUTPUT_BYTES],
    pub len: usize,
    pub status: u8,
    pub clear_screen: bool,
    pub action: CommandAction,
}

impl CommandOutput {
    pub const fn new() -> Self {
        Self {
            bytes: [0; MAX_OUTPUT_BYTES],
            len: 0,
            status: 0,
            clear_screen: false,
            action: CommandAction::None,
        }
    }

    fn push(&mut self, byte: u8) {
        if self.len < self.bytes.len() {
            self.bytes[self.len] = byte;
            self.len += 1;
        }
    }

    fn extend(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.push(byte);
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl Default for CommandOutput {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CommandService;

impl CommandService {
    pub const fn new() -> Self {
        Self
    }

    fn command_spec(line: &[u8]) -> Option<CommandSpec> {
        for spec in COMMAND_SPECS {
            let matches = match spec.kind {
                CommandKind::Echo => {
                    line == spec.name
                        || (line.len() > spec.name.len()
                            && line.starts_with(spec.name)
                            && line[spec.name.len()] == b' ')
                }
                CommandKind::Service => {
                    line == spec.name
                        || (line.len() > spec.name.len()
                            && line.starts_with(spec.name)
                            && line[spec.name.len()] == b' ')
                }
                _ => line == spec.name,
            };
            if matches {
                return Some(spec);
            }
        }
        None
    }

    fn help(line: &[u8]) -> CommandOutput {
        let mut output = CommandOutput::new();
        let line = trim_command(line);
        let target = match line.iter().position(|byte| *byte == b' ') {
            Some(index) => trim_command(&line[index + 1..]),
            None => &[][..],
        };
        if target.is_empty() {
            output.extend(b"Available commands:\r\n");
            for spec in COMMAND_SPECS {
                output.extend(spec.usage);
                output.extend(b" - ");
                output.extend(spec.summary);
                output.extend(b"\r\n");
            }
            output.extend(b"Use help <command> for details.\r\n");
            return output;
        }
        if target.contains(&b' ') {
            output.status = 2;
            output.extend(b"usage: help [command]\r\n");
            return output;
        }
        let Some(spec) = COMMAND_SPECS.iter().find(|spec| spec.name == target) else {
            output.status = 1;
            output.extend(b"help: no manual entry for ");
            output.extend(target);
            output.extend(b"\r\n");
            return output;
        };
        output.extend(spec.name);
        output.extend(b" - ");
        output.extend(spec.summary);
        output.extend(b"\r\nUsage: ");
        output.extend(spec.usage);
        output.extend(b"\r\n");
        output.extend(spec.manual);
        output.extend(b"\r\n");
        output
    }

    pub fn execute(&mut self, line: &[u8]) -> CommandOutput {
        let mut output = CommandOutput::new();
        if line.len() > MAX_COMMAND_BYTES {
            output.status = 2;
            output.extend(b"command too long\r\n");
            return output;
        }
        let line = trim_command(line);
        if line == b"help" || line.starts_with(b"help ") {
            return Self::help(line);
        }
        match Self::command_spec(line) {
            Some(spec) => match spec.kind {
                CommandKind::Help => {
                    for (index, command) in COMMAND_SPECS.iter().enumerate() {
                        if index > 0 {
                            output.push(b' ');
                        }
                        output.extend(command.name);
                    }
                    output.extend(b"\r\n");
                }
                CommandKind::Echo => {
                    if line.len() > spec.name.len() {
                        output.extend(&line[spec.name.len() + 1..]);
                    }
                    output.extend(b"\r\n");
                }
                CommandKind::Clear => output.clear_screen = true,
                CommandKind::True => {}
                CommandKind::False => output.status = 1,
                CommandKind::Version => output.extend(b"LogOS vNext 0.1.0\r\n"),
                CommandKind::Uname => output.extend(b"LogOS\r\n"),
                CommandKind::Shutdown => output.action = CommandAction::Shutdown,
                CommandKind::Reboot => output.action = CommandAction::Reboot,
                CommandKind::List
                | CommandKind::Touch
                | CommandKind::Cat
                | CommandKind::Write
                | CommandKind::Remove
                | CommandKind::Move => output.extend(b"storage command unavailable\r\n"),
                CommandKind::Service => {
                    output.status = 2;
                    output.extend(b"usage: service <list|status|start|stop|restart> [name]\r\n");
                }
            },
            None => {
                output.status = 127;
                output.extend(b"command not found\r\n");
            }
        }
        output
    }
}

pub fn parse_service_command(
    line: &[u8],
) -> Result<Option<ServiceCommand<'_>>, ServiceCommandError> {
    let line = trim_command(line);
    let Some(separator) = line.iter().position(|byte| *byte == b' ') else {
        return if line == b"service" { Err(ServiceCommandError::Usage) } else { Ok(None) };
    };
    if &line[..separator] != b"service" {
        return Ok(None);
    }
    let args = trim_command(&line[separator + 1..]);
    let Some(command_end) = args.iter().position(|byte| *byte == b' ') else {
        return match args {
            b"list" => Ok(Some(ServiceCommand::List)),
            _ => Err(ServiceCommandError::Usage),
        };
    };
    let command = &args[..command_end];
    let name = trim_command(&args[command_end + 1..]);
    if name.is_empty() || name.contains(&b' ') {
        return Err(ServiceCommandError::Usage);
    }
    match command {
        b"status" => Ok(Some(ServiceCommand::Status { name })),
        b"start" => Ok(Some(ServiceCommand::Start { name })),
        b"stop" => Ok(Some(ServiceCommand::Stop { name })),
        b"restart" => Ok(Some(ServiceCommand::Restart { name })),
        _ => Err(ServiceCommandError::Usage),
    }
}

pub fn parse_storage_command(
    line: &[u8],
) -> Result<Option<StorageCommand<'_>>, StorageCommandError> {
    let line = trim_command(line);
    let (name, args) = match line.iter().position(|byte| *byte == b' ') {
        Some(index) => (&line[..index], trim_command(&line[index + 1..])),
        None => (line, &[][..]),
    };
    match name {
        b"ls" => Ok(Some(StorageCommand::List { path: if args.is_empty() { b"/" } else { args } })),
        b"touch" => Ok(Some(StorageCommand::Touch { path: path_arg(args)? })),
        b"cat" => Ok(Some(StorageCommand::Cat { path: path_arg(args)? })),
        b"rm" => Ok(Some(StorageCommand::Remove { path: path_arg(args)? })),
        b"write" => {
            let Some(separator) = args.iter().position(|byte| *byte == b' ') else {
                return Err(StorageCommandError::Usage);
            };
            let path = &args[..separator];
            let data = &args[separator + 1..];
            if path.is_empty() || data.is_empty() {
                return Err(StorageCommandError::Usage);
            }
            Ok(Some(StorageCommand::Write { path, data }))
        }
        b"mv" => {
            let Some(separator) = args.iter().position(|byte| *byte == b' ') else {
                return Err(StorageCommandError::Usage);
            };
            let from = &args[..separator];
            let to = trim_command(&args[separator + 1..]);
            if from.is_empty() || to.is_empty() || to.contains(&b' ') {
                return Err(StorageCommandError::Usage);
            }
            Ok(Some(StorageCommand::Move { from, to }))
        }
        _ => Ok(None),
    }
}

fn path_arg(args: &[u8]) -> Result<&[u8], StorageCommandError> {
    if args.is_empty() || args.contains(&b' ') {
        return Err(StorageCommandError::Usage);
    }
    Ok(args)
}

fn trim_command(line: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = line.len();
    while start < end && line[start] == b' ' {
        start += 1;
    }
    while end > start && line[end - 1] == b' ' {
        end -= 1;
    }
    &line[start..end]
}

impl Default for CommandService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_bounded_and_deterministic() {
        let mut commands = CommandService::new();
        let help = commands.execute(b"help");
        assert!(help.as_bytes().len() <= MAX_OUTPUT_BYTES);
        assert!(help.as_bytes().windows(b"service".len()).any(|window| window == b"service"));
        assert_eq!(
            commands.execute(b"help write").as_bytes(),
            b"write - replace file contents\r\nUsage: write <path> <data>\r\nAtomically replaces an existing file's contents. Data must be non-empty.\r\n"
        );
        assert_eq!(commands.execute(b"help missing").status, 1);
        assert!(commands.execute(b"help missing").as_bytes().len() <= MAX_OUTPUT_BYTES);
        assert_eq!(commands.execute(b"echo").as_bytes(), b"\r\n");
        assert_eq!(commands.execute(b"echo hi").as_bytes(), b"hi\r\n");
        assert!(commands.execute(b"clear").clear_screen);
        assert_eq!(commands.execute(b"true").status, 0);
        assert_eq!(commands.execute(b"false").status, 1);
        assert_eq!(commands.execute(b"version").as_bytes(), b"LogOS vNext 0.1.0\r\n");
        assert_eq!(commands.execute(b" version ").as_bytes(), b"LogOS vNext 0.1.0\r\n");
        assert_eq!(commands.execute(b"uname").as_bytes(), b"LogOS\r\n");
        assert_eq!(commands.execute(b"shutdown").action, CommandAction::Shutdown);
        assert_eq!(commands.execute(b"reboot").action, CommandAction::Reboot);
        assert_eq!(commands.execute(b"missing").status, 127);
    }

    #[test]
    fn every_catalog_entry_has_dispatch_behavior() {
        let mut commands = CommandService::new();
        for spec in COMMAND_SPECS {
            assert_ne!(commands.execute(spec.name).status, 127);
        }
    }

    #[test]
    fn command_input_is_bounded() {
        let mut commands = CommandService::new();
        let output = commands.execute(&[b'x'; MAX_COMMAND_BYTES + 1]);
        assert_eq!(output.status, 2);
        assert!(output.as_bytes().len() <= MAX_OUTPUT_BYTES);
    }

    #[test]
    fn storage_commands_parse_with_bounded_arguments() {
        assert_eq!(
            parse_storage_command(b"ls").unwrap(),
            Some(StorageCommand::List { path: b"/" })
        );
        assert_eq!(
            parse_storage_command(b"write /file durable data").unwrap(),
            Some(StorageCommand::Write { path: b"/file", data: b"durable data" })
        );
        assert_eq!(
            parse_storage_command(b"mv /old /new").unwrap(),
            Some(StorageCommand::Move { from: b"/old", to: b"/new" })
        );
        assert_eq!(parse_storage_command(b"write /file").unwrap_err(), StorageCommandError::Usage);
        assert!(parse_storage_command(b"not-storage").unwrap().is_none());
    }

    #[test]
    fn service_commands_parse_with_bounded_arguments() {
        assert_eq!(parse_service_command(b"service list").unwrap(), Some(ServiceCommand::List));
        assert_eq!(
            parse_service_command(b"service restart storage").unwrap(),
            Some(ServiceCommand::Restart { name: b"storage" })
        );
        assert_eq!(
            parse_service_command(b"service start").unwrap_err(),
            ServiceCommandError::Usage
        );
        assert!(parse_service_command(b"echo hi").unwrap().is_none());
    }

    #[test]
    fn shell_paths_are_root_relative_without_a_current_directory() {
        let mut output = [0; 8];
        assert_eq!(root_relative_path(b"marker", &mut output), Some(&b"/marker"[..]));
        assert_eq!(root_relative_path(b"/marker", &mut output), Some(&b"/marker"[..]));
        assert_eq!(root_relative_path(b"", &mut output), Some(&b""[..]));
        assert_eq!(root_relative_path(b"toolong", &mut [0; 8]), Some(&b"/toolong"[..]));
        assert_eq!(root_relative_path(b"toolong", &mut [0; 7]), None);
    }
}
