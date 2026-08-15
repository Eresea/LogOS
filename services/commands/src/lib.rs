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
}

pub const COMMAND_SPECS: [CommandSpec; 9] = [
    CommandSpec { name: b"help", kind: CommandKind::Help },
    CommandSpec { name: b"echo", kind: CommandKind::Echo },
    CommandSpec { name: b"clear", kind: CommandKind::Clear },
    CommandSpec { name: b"true", kind: CommandKind::True },
    CommandSpec { name: b"false", kind: CommandKind::False },
    CommandSpec { name: b"version", kind: CommandKind::Version },
    CommandSpec { name: b"uname", kind: CommandKind::Uname },
    CommandSpec { name: b"shutdown", kind: CommandKind::Shutdown },
    CommandSpec { name: b"reboot", kind: CommandKind::Reboot },
];

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
                _ => line == spec.name,
            };
            if matches {
                return Some(spec);
            }
        }
        None
    }

    pub fn execute(&mut self, line: &[u8]) -> CommandOutput {
        let mut output = CommandOutput::new();
        if line.len() > MAX_COMMAND_BYTES {
            output.status = 2;
            output.extend(b"command too long\r\n");
            return output;
        }
        match Self::command_spec(trim_command(line)) {
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
            },
            None => {
                output.status = 127;
                output.extend(b"command not found\r\n");
            }
        }
        output
    }
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
        assert_eq!(
            commands.execute(b"help").as_bytes(),
            b"help echo clear true false version uname shutdown reboot\r\n"
        );
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
}
