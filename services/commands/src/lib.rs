#![no_std]

#[cfg(test)]
extern crate std;

pub const MAX_COMMAND_BYTES: usize = 256;
pub const MAX_OUTPUT_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub bytes: [u8; MAX_OUTPUT_BYTES],
    pub len: usize,
    pub status: u8,
    pub clear_screen: bool,
}

impl CommandOutput {
    pub const fn new() -> Self {
        Self { bytes: [0; MAX_OUTPUT_BYTES], len: 0, status: 0, clear_screen: false }
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

    pub fn execute(&mut self, line: &[u8]) -> CommandOutput {
        let mut output = CommandOutput::new();
        if line.len() > MAX_COMMAND_BYTES {
            output.status = 2;
            output.extend(b"command too long\r\n");
            return output;
        }
        match line {
            b"help" => {
                for (index, command) in logos_abi::BUILTIN_COMMANDS.iter().enumerate() {
                    if index > 0 {
                        output.push(b' ');
                    }
                    output.extend(command);
                }
                output.extend(b"\r\n");
            }
            b"clear" => output.clear_screen = true,
            b"true" => {}
            b"false" => output.status = 1,
            b"version" => output.extend(b"LogOS vNext 0.1.0\r\n"),
            b"uname" => output.extend(b"LogOS\r\n"),
            _ if line.starts_with(b"echo ") => {
                output.extend(&line[5..]);
                output.extend(b"\r\n");
            }
            _ => {
                output.status = 127;
                output.extend(b"command not found\r\n");
            }
        }
        output
    }
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
            b"help echo clear true false version uname\r\n"
        );
        assert_eq!(commands.execute(b"echo hi").as_bytes(), b"hi\r\n");
        assert!(commands.execute(b"clear").clear_screen);
        assert_eq!(commands.execute(b"true").status, 0);
        assert_eq!(commands.execute(b"false").status, 1);
        assert_eq!(commands.execute(b"version").as_bytes(), b"LogOS vNext 0.1.0\r\n");
        assert_eq!(commands.execute(b"uname").as_bytes(), b"LogOS\r\n");
        assert_eq!(commands.execute(b"missing").status, 127);
    }

    #[test]
    fn command_input_is_bounded() {
        let mut commands = CommandService::new();
        let output = commands.execute(&[b'x'; MAX_COMMAND_BYTES + 1]);
        assert_eq!(output.status, 2);
        assert!(output.as_bytes().len() <= MAX_OUTPUT_BYTES);
    }
}
