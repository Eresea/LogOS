#![no_std]

#[cfg(test)]
extern crate std;

pub const MAX_COMMAND_BYTES: usize = 256;
pub const MAX_OUTPUT_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionTarget {
    Root,
    ServiceName,
    ServiceMember,
    NetworkMember,
    InterfaceName,
    SystemMember,
    FilesystemMember,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionContext<'a> {
    pub target: CompletionTarget,
    pub replace_start: usize,
    pub replace_end: usize,
    pub prefix: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionError {
    InvalidCursor,
    LineTooLong,
}

pub const SERVICE_COMPLETION_MEMBERS: [&[u8]; 6] =
    [b"status", b"name", b"version", b"start()", b"stop()", b"restart()"];
pub const NETWORK_COMPLETION_MEMBERS: [&[u8]; 5] =
    [b"status", b"ping()", b"tcp-probe()", b"fetch()", b"interface[\""];
pub const SYSTEM_COMPLETION_MEMBERS: [&[u8]; 4] =
    [b"version()", b"uname()", b"shutdown()", b"reboot()"];
pub const FILESYSTEM_COMPLETION_MEMBERS: [&[u8]; 6] =
    [b"list()", b"create()", b"read()", b"write()", b"remove()", b"move()"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandKind {
    Help,
    Echo,
    Clear,
    True,
    False,
    System,
    Filesystem,
    Service,
    Network,
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

pub const COMMAND_SPECS: [CommandSpec; 9] = [
    CommandSpec {
        name: b"help",
        kind: CommandKind::Help,
        usage: b"help() / help(\"command\")",
        summary: b"show command help",
        manual: b"Lists commands or shows one command manual.",
    },
    CommandSpec {
        name: b"echo",
        kind: CommandKind::Echo,
        usage: b"echo(\"text\")",
        summary: b"print text",
        manual: b"Prints the supplied text.",
    },
    CommandSpec {
        name: b"clear",
        kind: CommandKind::Clear,
        usage: b"clear()",
        summary: b"clear the screen",
        manual: b"Clears the terminal display.",
    },
    CommandSpec {
        name: b"true",
        kind: CommandKind::True,
        usage: b"true()",
        summary: b"succeed",
        manual: b"Completes successfully without output.",
    },
    CommandSpec {
        name: b"false",
        kind: CommandKind::False,
        usage: b"false()",
        summary: b"fail",
        manual: b"Completes with a failure status.",
    },
    CommandSpec {
        name: b"sys",
        kind: CommandKind::System,
        usage: b"sys.version() / sys.uname() / sys.shutdown() / sys.reboot()",
        summary: b"inspect and control the system",
        manual: b"Shows system information or requests shutdown/reboot.",
    },
    CommandSpec {
        name: b"fs",
        kind: CommandKind::Filesystem,
        usage: b"fs.list() / fs.create(\"path\") / fs.read(\"path\")",
        summary: b"manage files",
        manual: b"Lists, creates, reads, writes, removes, or moves files.",
    },
    CommandSpec {
        name: b"service",
        kind: CommandKind::Service,
        usage: b"service[\"name\"].status / service[\"name\"].restart()",
        summary: b"manage services",
        manual: b"Lists, inspects, starts, stops, or restarts a service.",
    },
    CommandSpec {
        name: b"net",
        kind: CommandKind::Network,
        usage: b"net.status / net.fetch(\"url\", \"path\")",
        summary: b"inspect and probe networking",
        manual: b"Shows network status, probes networking, or fetches an HTTP file.",
    },
];

pub fn completion_context<'a>(
    line: &'a [u8],
    cursor: usize,
) -> Result<Option<CompletionContext<'a>>, CompletionError> {
    if line.len() > logos_abi::MAX_COMPLETION_LINE_BYTES {
        return Err(CompletionError::LineTooLong);
    }
    if cursor > line.len() {
        return Err(CompletionError::InvalidCursor);
    }
    let before = &line[..cursor];
    const SERVICE_NAME_PREFIX: &[u8] = b"service[\"";
    const INTERFACE_NAME_PREFIX: &[u8] = b"net.interface[\"";

    if before.starts_with(SERVICE_NAME_PREFIX) {
        let start = SERVICE_NAME_PREFIX.len();
        if let Some(close) = find_bytes(&before[start..], b"\"]") {
            let member_start = start + close + 2;
            if before.get(member_start) == Some(&b'.')
                && before[member_start + 1..].iter().all(|byte| is_member_byte(*byte))
            {
                return Ok(Some(CompletionContext {
                    target: CompletionTarget::ServiceMember,
                    replace_start: member_start + 1,
                    replace_end: cursor,
                    prefix: &before[member_start + 1..],
                }));
            }
            return Ok(None);
        }
        if before[start..].iter().all(|byte| is_name_byte(*byte)) {
            return Ok(Some(CompletionContext {
                target: CompletionTarget::ServiceName,
                replace_start: start,
                replace_end: cursor,
                prefix: &before[start..],
            }));
        }
        return Ok(None);
    }

    if before.starts_with(INTERFACE_NAME_PREFIX) {
        let start = INTERFACE_NAME_PREFIX.len();
        if before[start..].iter().all(|byte| is_name_byte(*byte)) {
            return Ok(Some(CompletionContext {
                target: CompletionTarget::InterfaceName,
                replace_start: start,
                replace_end: cursor,
                prefix: &before[start..],
            }));
        }
        return Ok(None);
    }

    if before.starts_with(b"net.") && before[4..].iter().all(|byte| is_member_byte(*byte)) {
        return Ok(Some(CompletionContext {
            target: CompletionTarget::NetworkMember,
            replace_start: 4,
            replace_end: cursor,
            prefix: &before[4..],
        }));
    }

    if before.starts_with(b"sys.") && before[4..].iter().all(|byte| is_member_byte(*byte)) {
        return Ok(Some(CompletionContext {
            target: CompletionTarget::SystemMember,
            replace_start: 4,
            replace_end: cursor,
            prefix: &before[4..],
        }));
    }

    if before.starts_with(b"fs.") && before[3..].iter().all(|byte| is_member_byte(*byte)) {
        return Ok(Some(CompletionContext {
            target: CompletionTarget::FilesystemMember,
            replace_start: 3,
            replace_end: cursor,
            prefix: &before[3..],
        }));
    }

    let start = before.iter().position(|byte| byte.is_ascii_alphabetic() || *byte == b'_');
    let Some(start) = start else {
        return before
            .is_empty()
            .then_some(CompletionContext {
                target: CompletionTarget::Root,
                replace_start: 0,
                replace_end: 0,
                prefix: &[],
            })
            .map_or(Ok(None), |context| Ok(Some(context)));
    };
    if before[..start].iter().any(|byte| !byte.is_ascii_whitespace())
        || !before[start..].iter().all(|byte| is_root_byte(*byte))
    {
        return Ok(None);
    }
    Ok(Some(CompletionContext {
        target: CompletionTarget::Root,
        replace_start: start,
        replace_end: cursor,
        prefix: &before[start..],
    }))
}

fn is_root_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn is_member_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

fn find_bytes(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes.windows(needle.len()).position(|window| window == needle)
}

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
    Lookup { name: &'a [u8] },
    Status { name: &'a [u8] },
    Name { name: &'a [u8] },
    Version { name: &'a [u8] },
    Start { name: &'a [u8] },
    Stop { name: &'a [u8] },
    Restart { name: &'a [u8] },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceCommandError {
    Usage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceProperty {
    Record,
    Status,
    Name,
    Version,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkCommand<'a> {
    Status,
    InterfaceStatus { name: &'a [u8] },
    Ping { address: [u8; 4] },
    TcpProbe { address: [u8; 4], port: u16 },
    Fetch { url: &'a [u8], destination: &'a [u8] },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkCommandError {
    Usage,
}

const MAX_CALL_ARGS: usize = 3;
const MAX_EXPRESSION_PARTS: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParsedValue<'a> {
    bytes: &'a [u8],
    quoted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpressionPart<'a> {
    Member(&'a [u8]),
    Lookup(ParsedValue<'a>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParsedCall<'a> {
    args: [ParsedValue<'a>; MAX_CALL_ARGS],
    arg_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParsedExpression<'a> {
    root: &'a [u8],
    parts: [ExpressionPart<'a>; MAX_EXPRESSION_PARTS],
    part_count: usize,
    call: Option<ParsedCall<'a>>,
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

    fn command_spec(name: &[u8]) -> Option<CommandSpec> {
        COMMAND_SPECS.iter().find(|spec| spec.name == name).copied()
    }

    fn help(expression: ParsedExpression<'_>) -> CommandOutput {
        let mut output = CommandOutput::new();
        let Some(call) = expression.call else {
            output.status = 2;
            output.extend(b"usage: help() / help(\"command\")\r\n");
            return output;
        };
        if call.arg_count > 1 {
            output.status = 2;
            output.extend(b"usage: help() / help(\"command\")\r\n");
            return output;
        }
        if call.arg_count == 0 {
            output.extend(b"Available commands:\r\n");
            for spec in COMMAND_SPECS {
                output.extend(spec.usage);
                output.extend(b" - ");
                output.extend(spec.summary);
                output.extend(b"\r\n");
            }
            output.extend(b"Use help(\"command\") for details.\r\n");
            return output;
        }
        let target = call.args[0];
        if !target.quoted {
            output.status = 2;
            output.extend(b"usage: help() / help(\"command\")\r\n");
            return output;
        }
        let Some(spec) = COMMAND_SPECS.iter().find(|spec| spec.name == target.bytes) else {
            output.status = 1;
            output.extend(b"help: no manual entry for ");
            output.extend(target.bytes);
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
        let expression = match parse_expression(line) {
            Ok(expression) => expression,
            Err(()) => {
                output.status = 2;
                output.extend(b"command syntax error\r\n");
                return output;
            }
        };
        let Some(call) = expression.call else {
            output.status = 2;
            output.extend(b"command syntax error\r\n");
            return output;
        };
        let Some(spec) = Self::command_spec(expression.root) else {
            output.status = 127;
            output.extend(b"command not found\r\n");
            return output;
        };
        if expression.part_count != 0
            && !matches!(spec.kind, CommandKind::System | CommandKind::Filesystem)
        {
            output.status = 127;
            output.extend(b"command not found\r\n");
            return output;
        }
        match spec.kind {
            CommandKind::Help => return Self::help(expression),
            CommandKind::Echo => {
                if call.arg_count > 1 || (call.arg_count == 1 && !call.args[0].quoted) {
                    output.status = 2;
                    output.extend(b"usage: echo(\"text\")\r\n");
                    return output;
                }
                output.extend(call.args[0].bytes);
                output.extend(b"\r\n");
            }
            CommandKind::Clear => {
                if call.arg_count == 0 {
                    output.clear_screen = true;
                } else {
                    output.status = 2;
                    output.extend(spec.usage);
                    output.extend(b"\r\n");
                }
            }
            CommandKind::True => {
                if call.arg_count != 0 {
                    output.status = 2;
                    output.extend(spec.usage);
                    output.extend(b"\r\n");
                }
            }
            CommandKind::False => {
                if call.arg_count == 0 {
                    output.status = 1;
                } else {
                    output.status = 2;
                    output.extend(spec.usage);
                    output.extend(b"\r\n");
                }
            }
            CommandKind::System => {
                let member = expression.parts.first().and_then(|part| match part {
                    ExpressionPart::Member(member) => Some(*member),
                    ExpressionPart::Lookup(_) => None,
                });
                if expression.part_count != 1 || call.arg_count != 0 {
                    output.status = 2;
                    output.extend(spec.usage);
                    output.extend(b"\r\n");
                } else {
                    match member {
                        Some(b"version") => output.extend(b"LogOS vNext 0.1.0\r\n"),
                        Some(b"uname") => output.extend(b"LogOS\r\n"),
                        Some(b"shutdown") => output.action = CommandAction::Shutdown,
                        Some(b"reboot") => output.action = CommandAction::Reboot,
                        _ => {
                            output.status = 2;
                            output.extend(spec.usage);
                            output.extend(b"\r\n");
                        }
                    }
                }
            }
            CommandKind::Filesystem => {
                output.status = 2;
                output.extend(spec.usage);
                output.extend(b"\r\n");
            }
            CommandKind::Service => {
                output.status = 2;
                output.extend(spec.usage);
                output.extend(b"\r\n");
            }
            CommandKind::Network => {
                output.status = 2;
                output.extend(spec.usage);
                output.extend(b"\r\n");
            }
        }
        output
    }
}

pub fn format_service_record(record: &logos_abi::ServiceManagerRecord, output: &mut [u8]) -> usize {
    let state = match record.state {
        logos_abi::ManagerState::Vacant => b"vacant" as &[u8],
        logos_abi::ManagerState::Disabled => b"disabled",
        logos_abi::ManagerState::Stopped => b"stopped",
        logos_abi::ManagerState::Starting => b"starting",
        logos_abi::ManagerState::Running => b"running",
        logos_abi::ManagerState::Stopping => b"stopping",
        logos_abi::ManagerState::Failed => b"failed",
    };
    let name_len = usize::from(record.name_len).min(record.name.len());
    let mut length = 0;
    for part in [&record.name[..name_len], b" ", state, b"\r\n"] {
        let count = part.len().min(output.len().saturating_sub(length));
        output[length..length + count].copy_from_slice(&part[..count]);
        length += count;
    }
    length
}

pub fn format_service_property(
    record: &logos_abi::ServiceManagerRecord,
    property: ServiceProperty,
    output: &mut [u8],
) -> usize {
    match property {
        ServiceProperty::Record => format_service_record(record, output),
        ServiceProperty::Status => copy_line(service_state(record.state), output),
        ServiceProperty::Name => {
            let name_len = usize::from(record.name_len).min(record.name.len());
            copy_line(&record.name[..name_len], output)
        }
        ServiceProperty::Version => copy_line(b"0.1.0", output),
    }
}

fn service_state(state: logos_abi::ManagerState) -> &'static [u8] {
    match state {
        logos_abi::ManagerState::Vacant => b"vacant",
        logos_abi::ManagerState::Disabled => b"disabled",
        logos_abi::ManagerState::Stopped => b"stopped",
        logos_abi::ManagerState::Starting => b"starting",
        logos_abi::ManagerState::Running => b"running",
        logos_abi::ManagerState::Stopping => b"stopping",
        logos_abi::ManagerState::Failed => b"failed",
    }
}

fn copy_bounded(bytes: &[u8], output: &mut [u8]) -> usize {
    let length = bytes.len().min(output.len());
    output[..length].copy_from_slice(&bytes[..length]);
    length
}

fn copy_line(bytes: &[u8], output: &mut [u8]) -> usize {
    let mut length = copy_bounded(bytes, output);
    for byte in b"\r\n" {
        if length == output.len() {
            break;
        }
        output[length] = *byte;
        length += 1;
    }
    length
}

fn parse_expression(line: &[u8]) -> Result<ParsedExpression<'_>, ()> {
    let line = trim_command(line);
    let (root, mut index) = parse_identifier(line, 0)?;
    let mut parts = [ExpressionPart::Member(&[][..]); MAX_EXPRESSION_PARTS];
    let mut part_count = 0;
    let mut call = None;
    loop {
        skip_spaces(line, &mut index);
        if index == line.len() {
            break;
        }
        if call.is_some() {
            return Err(());
        }
        match line[index] {
            b'.' => {
                index += 1;
                skip_spaces(line, &mut index);
                let (member, next) = parse_identifier(line, index)?;
                index = next;
                push_part(&mut parts, &mut part_count, ExpressionPart::Member(member))?;
            }
            b'[' => {
                index += 1;
                let value = parse_value(line, &mut index)?;
                skip_spaces(line, &mut index);
                if line.get(index) != Some(&b']') {
                    return Err(());
                }
                index += 1;
                push_part(&mut parts, &mut part_count, ExpressionPart::Lookup(value))?;
            }
            b'(' => {
                if matches!(parts.get(part_count.wrapping_sub(1)), Some(ExpressionPart::Lookup(_)))
                {
                    return Err(());
                }
                call = Some(parse_arguments(line, &mut index)?);
            }
            _ => return Err(()),
        }
    }
    Ok(ParsedExpression { root, parts, part_count, call })
}

fn parse_identifier(bytes: &[u8], mut index: usize) -> Result<(&[u8], usize), ()> {
    let start = index;
    while let Some(byte) = bytes.get(index) {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-') {
            index += 1;
        } else {
            break;
        }
    }
    if start == index { Err(()) } else { Ok((&bytes[start..index], index)) }
}

fn parse_value<'a>(bytes: &'a [u8], index: &mut usize) -> Result<ParsedValue<'a>, ()> {
    skip_spaces(bytes, index);
    if bytes.get(*index) == Some(&b'"') {
        let start = *index + 1;
        *index = start;
        while *index < bytes.len() && bytes[*index] != b'"' {
            *index += 1;
        }
        if *index == bytes.len() {
            return Err(());
        }
        let value = ParsedValue { bytes: &bytes[start..*index], quoted: true };
        *index += 1;
        return Ok(value);
    }
    let start = *index;
    while let Some(byte) = bytes.get(*index) {
        if byte.is_ascii_digit() {
            *index += 1;
        } else {
            break;
        }
    }
    if start == *index {
        return Err(());
    }
    Ok(ParsedValue { bytes: &bytes[start..*index], quoted: false })
}

fn parse_arguments<'a>(bytes: &'a [u8], index: &mut usize) -> Result<ParsedCall<'a>, ()> {
    if bytes.get(*index) != Some(&b'(') {
        return Err(());
    }
    *index += 1;
    let mut args = [ParsedValue { bytes: &[][..], quoted: false }; MAX_CALL_ARGS];
    let mut arg_count = 0;
    loop {
        skip_spaces(bytes, index);
        if bytes.get(*index) == Some(&b')') {
            *index += 1;
            return Ok(ParsedCall { args, arg_count });
        }
        if arg_count == MAX_CALL_ARGS {
            return Err(());
        }
        args[arg_count] = parse_value(bytes, index)?;
        arg_count += 1;
        skip_spaces(bytes, index);
        match bytes.get(*index) {
            Some(b',') => {
                *index += 1;
                skip_spaces(bytes, index);
                if bytes.get(*index) == Some(&b')') {
                    return Err(());
                }
            }
            Some(b')') => {
                *index += 1;
                return Ok(ParsedCall { args, arg_count });
            }
            _ => return Err(()),
        }
    }
}

fn push_part<'a>(
    parts: &mut [ExpressionPart<'a>; MAX_EXPRESSION_PARTS],
    count: &mut usize,
    part: ExpressionPart<'a>,
) -> Result<(), ()> {
    if *count == parts.len() {
        return Err(());
    }
    parts[*count] = part;
    *count += 1;
    Ok(())
}

fn skip_spaces(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && bytes[*index].is_ascii_whitespace() {
        *index += 1;
    }
}

fn expression_root(line: &[u8]) -> Option<&[u8]> {
    let line = trim_command(line);
    parse_identifier(line, 0).ok().map(|(root, _)| root)
}

fn scoped_expression<'a>(line: &'a [u8], root: &[u8]) -> Result<Option<ParsedExpression<'a>>, ()> {
    if expression_root(line) != Some(root) {
        return Ok(None);
    }
    Ok(Some(parse_expression(line)?))
}

pub fn parse_service_command(
    line: &[u8],
) -> Result<Option<ServiceCommand<'_>>, ServiceCommandError> {
    let Some(expression) =
        scoped_expression(line, b"service").map_err(|_| ServiceCommandError::Usage)?
    else {
        return Ok(None);
    };
    if expression.part_count == 1 {
        let ExpressionPart::Lookup(name) = expression.parts[0] else {
            return Err(ServiceCommandError::Usage);
        };
        if !name.quoted || name.bytes.is_empty() || expression.call.is_some() {
            return Err(ServiceCommandError::Usage);
        }
        return Ok(Some(ServiceCommand::Lookup { name: name.bytes }));
    }
    if expression.part_count != 2 {
        return Err(ServiceCommandError::Usage);
    }
    let ExpressionPart::Lookup(name) = expression.parts[0] else {
        return Err(ServiceCommandError::Usage);
    };
    let ExpressionPart::Member(member) = expression.parts[1] else {
        return Err(ServiceCommandError::Usage);
    };
    if !name.quoted || name.bytes.is_empty() {
        return Err(ServiceCommandError::Usage);
    }
    match member {
        b"status" if expression.call.is_none() => {
            Ok(Some(ServiceCommand::Status { name: name.bytes }))
        }
        b"name" if expression.call.is_none() => Ok(Some(ServiceCommand::Name { name: name.bytes })),
        b"version" if expression.call.is_none() => {
            Ok(Some(ServiceCommand::Version { name: name.bytes }))
        }
        b"start" if expression.call.is_some_and(|call| call.arg_count == 0) => {
            Ok(Some(ServiceCommand::Start { name: name.bytes }))
        }
        b"stop" if expression.call.is_some_and(|call| call.arg_count == 0) => {
            Ok(Some(ServiceCommand::Stop { name: name.bytes }))
        }
        b"restart" if expression.call.is_some_and(|call| call.arg_count == 0) => {
            Ok(Some(ServiceCommand::Restart { name: name.bytes }))
        }
        _ => Err(ServiceCommandError::Usage),
    }
}

pub fn parse_storage_command(
    line: &[u8],
) -> Result<Option<StorageCommand<'_>>, StorageCommandError> {
    let Some(expression) =
        scoped_expression(line, b"fs").map_err(|_| StorageCommandError::Usage)?
    else {
        return Ok(None);
    };
    if expression.part_count != 1 {
        return Err(StorageCommandError::Usage);
    }
    let ExpressionPart::Member(member) = expression.parts[0] else {
        return Err(StorageCommandError::Usage);
    };
    let Some(call) = expression.call else { return Err(StorageCommandError::Usage) };
    let args = &call.args[..call.arg_count];
    match (member, call.arg_count) {
        (b"list", 0) => Ok(Some(StorageCommand::List { path: b"/" })),
        (b"list", 1) => Ok(Some(StorageCommand::List { path: path_arg(args[0])? })),
        (b"create", 1) => Ok(Some(StorageCommand::Touch { path: path_arg(args[0])? })),
        (b"read", 1) => Ok(Some(StorageCommand::Cat { path: path_arg(args[0])? })),
        (b"remove", 1) => Ok(Some(StorageCommand::Remove { path: path_arg(args[0])? })),
        (b"write", 2) => {
            let path = path_arg(args[0])?;
            let data = string_arg(args[1]).map_err(|_| StorageCommandError::Usage)?;
            if data.is_empty() {
                return Err(StorageCommandError::Usage);
            }
            Ok(Some(StorageCommand::Write { path, data }))
        }
        (b"move", 2) => {
            Ok(Some(StorageCommand::Move { from: path_arg(args[0])?, to: path_arg(args[1])? }))
        }
        _ => Err(StorageCommandError::Usage),
    }
}

pub fn parse_network_command(
    line: &[u8],
) -> Result<Option<NetworkCommand<'_>>, NetworkCommandError> {
    let Some(expression) =
        scoped_expression(line, b"net").map_err(|_| NetworkCommandError::Usage)?
    else {
        return Ok(None);
    };
    if expression.part_count == 1 {
        let ExpressionPart::Member(member) = expression.parts[0] else {
            return Err(NetworkCommandError::Usage);
        };
        return match (member, expression.call) {
            (b"status", None) => Ok(Some(NetworkCommand::Status)),
            (b"ping", Some(call)) if call.arg_count == 1 => Ok(Some(NetworkCommand::Ping {
                address: parse_ipv4(
                    string_arg(call.args[0]).map_err(|_| NetworkCommandError::Usage)?,
                )?,
            })),
            (b"tcp-probe", Some(call)) if call.arg_count == 2 => {
                Ok(Some(NetworkCommand::TcpProbe {
                    address: parse_ipv4(
                        string_arg(call.args[0]).map_err(|_| NetworkCommandError::Usage)?,
                    )?,
                    port: if call.args[1].quoted {
                        return Err(NetworkCommandError::Usage);
                    } else {
                        parse_port(call.args[1].bytes)?
                    },
                }))
            }
            (b"fetch", Some(call)) if call.arg_count == 2 => {
                let url = string_arg(call.args[0]).map_err(|_| NetworkCommandError::Usage)?;
                let destination =
                    string_arg(call.args[1]).map_err(|_| NetworkCommandError::Usage)?;
                if url.is_empty() || destination.is_empty() {
                    return Err(NetworkCommandError::Usage);
                }
                Ok(Some(NetworkCommand::Fetch { url, destination }))
            }
            _ => Err(NetworkCommandError::Usage),
        };
    }
    if expression.part_count == 3
        && expression.call.is_none()
        && expression.parts[0] == ExpressionPart::Member(b"interface")
        && expression.parts[2] == ExpressionPart::Member(b"status")
    {
        let ExpressionPart::Lookup(name) = expression.parts[1] else {
            return Err(NetworkCommandError::Usage);
        };
        if !name.quoted || name.bytes.is_empty() {
            return Err(NetworkCommandError::Usage);
        }
        return Ok(Some(NetworkCommand::InterfaceStatus { name: name.bytes }));
    }
    Err(NetworkCommandError::Usage)
}

fn parse_ipv4(bytes: &[u8]) -> Result<[u8; 4], NetworkCommandError> {
    let mut address = [0; 4];
    let mut part = 0;
    let mut value = 0u16;
    let mut digits = 0;
    for byte in bytes.iter().copied().chain(core::iter::once(b'.')) {
        if byte.is_ascii_digit() {
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(u16::from(byte - b'0')))
                .ok_or(NetworkCommandError::Usage)?;
            if value > 255 {
                return Err(NetworkCommandError::Usage);
            }
            digits += 1;
        } else if byte == b'.' && digits != 0 && part < 4 {
            address[part] = value as u8;
            part += 1;
            value = 0;
            digits = 0;
        } else {
            return Err(NetworkCommandError::Usage);
        }
    }
    if part == 4 { Ok(address) } else { Err(NetworkCommandError::Usage) }
}

fn parse_port(bytes: &[u8]) -> Result<u16, NetworkCommandError> {
    if bytes.is_empty() {
        return Err(NetworkCommandError::Usage);
    }
    let mut value = 0u32;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return Err(NetworkCommandError::Usage);
        }
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u32::from(byte - b'0')))
            .ok_or(NetworkCommandError::Usage)?;
    }
    let port = u16::try_from(value).map_err(|_| NetworkCommandError::Usage)?;
    if port == 0 { Err(NetworkCommandError::Usage) } else { Ok(port) }
}

fn string_arg(value: ParsedValue<'_>) -> Result<&[u8], ()> {
    if !value.quoted {
        return Err(());
    }
    Ok(value.bytes)
}

fn path_arg(value: ParsedValue<'_>) -> Result<&[u8], StorageCommandError> {
    let path = string_arg(value).map_err(|_| StorageCommandError::Usage)?;
    if path.is_empty() {
        return Err(StorageCommandError::Usage);
    }
    Ok(path)
}

fn trim_command(line: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = line.len();
    while start < end && line[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && line[end - 1].is_ascii_whitespace() {
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
        let help = commands.execute(b"help()");
        assert!(help.as_bytes().len() <= MAX_OUTPUT_BYTES);
        assert!(help.as_bytes().windows(b"service".len()).any(|window| window == b"service"));
        assert_eq!(
            commands.execute(b"help(\"fs\")").as_bytes(),
            b"fs - manage files\r\nUsage: fs.list() / fs.create(\"path\") / fs.read(\"path\")\r\nLists, creates, reads, writes, removes, or moves files.\r\n"
        );
        assert_eq!(commands.execute(b"help(\"missing\")").status, 1);
        assert!(commands.execute(b"help(\"missing\")").as_bytes().len() <= MAX_OUTPUT_BYTES);
        assert_eq!(commands.execute(b"echo()").as_bytes(), b"\r\n");
        assert_eq!(commands.execute(b"echo(\"hi\")").as_bytes(), b"hi\r\n");
        assert_eq!(commands.execute(b"echo(\"hello\")").as_bytes(), b"hello\r\n");
        assert_eq!(commands.execute(b"echo(hello)").status, 2);
        assert!(commands.execute(b"clear()").clear_screen);
        assert_eq!(commands.execute(b"true()").status, 0);
        assert_eq!(commands.execute(b"false()").status, 1);
        assert_eq!(commands.execute(b"sys.version()").as_bytes(), b"LogOS vNext 0.1.0\r\n");
        assert_eq!(commands.execute(b" sys.version() ").as_bytes(), b"LogOS vNext 0.1.0\r\n");
        assert_eq!(commands.execute(b"sys.uname()").as_bytes(), b"LogOS\r\n");
        assert_eq!(commands.execute(b"sys.shutdown()").action, CommandAction::Shutdown);
        assert_eq!(commands.execute(b"sys.reboot()").action, CommandAction::Reboot);
        assert_eq!(commands.execute(b"missing()").status, 127);
    }

    #[test]
    fn every_catalog_entry_has_dispatch_behavior() {
        let mut commands = CommandService::new();
        assert_ne!(commands.execute(b"help()").status, 127);
        assert_ne!(commands.execute(b"echo()").status, 127);
        assert_ne!(commands.execute(b"clear()").status, 127);
        assert_ne!(commands.execute(b"true()").status, 127);
        assert_ne!(commands.execute(b"false()").status, 127);
        assert_ne!(commands.execute(b"sys.version()").status, 127);
        assert_ne!(commands.execute(b"sys.uname()").status, 127);
        assert_ne!(commands.execute(b"sys.shutdown()").status, 127);
        assert_ne!(commands.execute(b"sys.reboot()").status, 127);
        assert_ne!(commands.execute(b"fs.list()").status, 127);
        assert_ne!(commands.execute(b"fs.create()").status, 127);
        assert_ne!(commands.execute(b"fs.read()").status, 127);
        assert_ne!(commands.execute(b"fs.write()").status, 127);
        assert_ne!(commands.execute(b"fs.remove()").status, 127);
        assert_ne!(commands.execute(b"fs.move()").status, 127);
        assert_ne!(commands.execute(b"service()").status, 127);
        assert_ne!(commands.execute(b"net()").status, 127);
    }

    #[test]
    fn command_input_is_bounded() {
        let mut commands = CommandService::new();
        let output = commands.execute(&[b'x'; MAX_COMMAND_BYTES + 1]);
        assert_eq!(output.status, 2);
        assert!(output.as_bytes().len() <= MAX_OUTPUT_BYTES);
    }

    #[test]
    fn completion_context_targets_expression_parts() {
        assert_eq!(
            completion_context(b"he", 2).unwrap(),
            Some(CompletionContext {
                target: CompletionTarget::Root,
                replace_start: 0,
                replace_end: 2,
                prefix: b"he",
            })
        );
        assert_eq!(
            completion_context(b"service[\"st", 11).unwrap(),
            Some(CompletionContext {
                target: CompletionTarget::ServiceName,
                replace_start: 9,
                replace_end: 11,
                prefix: b"st",
            })
        );
        assert_eq!(
            completion_context(b"service[\"storage\"].re", 21).unwrap(),
            Some(CompletionContext {
                target: CompletionTarget::ServiceMember,
                replace_start: 19,
                replace_end: 21,
                prefix: b"re",
            })
        );
        assert_eq!(
            completion_context(b"net.", 4).unwrap().unwrap().target,
            CompletionTarget::NetworkMember
        );
        assert_eq!(
            completion_context(b"sys.", 4).unwrap().unwrap().target,
            CompletionTarget::SystemMember
        );
        assert_eq!(
            completion_context(b"fs.", 3).unwrap().unwrap().target,
            CompletionTarget::FilesystemMember
        );
        assert_eq!(
            completion_context(b"net.interface[\"e", 16).unwrap().unwrap().target,
            CompletionTarget::InterfaceName
        );
        assert!(completion_context(b"service[storage", 15).unwrap().is_none());
        assert!(completion_context(&[b'x'; logos_abi::MAX_COMPLETION_LINE_BYTES + 1], 0).is_err());
    }

    #[test]
    fn storage_commands_parse_with_bounded_arguments() {
        assert_eq!(
            parse_storage_command(b"fs.list()").unwrap(),
            Some(StorageCommand::List { path: b"/" })
        );
        assert_eq!(
            parse_storage_command(b"fs.write(\"/file\", \"durable data\")").unwrap(),
            Some(StorageCommand::Write { path: b"/file", data: b"durable data" })
        );
        assert_eq!(
            parse_storage_command(b"fs.move(\"/old\", \"/new\")").unwrap(),
            Some(StorageCommand::Move { from: b"/old", to: b"/new" })
        );
        assert_eq!(
            parse_storage_command(b"fs.write(\"/file\")").unwrap_err(),
            StorageCommandError::Usage
        );
        assert!(parse_storage_command(b"cat(\"/file\")").unwrap().is_none());
    }

    #[test]
    fn service_commands_parse_with_bounded_arguments() {
        assert_eq!(
            parse_service_command(b"service[\"storage\"]").unwrap(),
            Some(ServiceCommand::Lookup { name: b"storage" })
        );
        assert_eq!(
            parse_service_command(b"service[\"storage\"].status").unwrap(),
            Some(ServiceCommand::Status { name: b"storage" })
        );
        assert_eq!(
            parse_service_command(b"service[\"storage\"].restart()").unwrap(),
            Some(ServiceCommand::Restart { name: b"storage" })
        );
        assert_eq!(
            parse_service_command(b"service[\"storage\"].start()").unwrap(),
            Some(ServiceCommand::Start { name: b"storage" })
        );
        assert_eq!(
            parse_service_command(b"service[\"storage\"].name").unwrap(),
            Some(ServiceCommand::Name { name: b"storage" })
        );
        assert_eq!(
            parse_service_command(b"service[\"storage\"].version").unwrap(),
            Some(ServiceCommand::Version { name: b"storage" })
        );
        assert_eq!(
            parse_service_command(b"service[\"storage\"].status()").unwrap_err(),
            ServiceCommandError::Usage
        );
        assert!(parse_service_command(b"echo(\"hi\")").unwrap().is_none());
    }

    #[test]
    fn network_commands_parse_ipv4_and_port_bounds() {
        assert_eq!(parse_network_command(b"net.status").unwrap(), Some(NetworkCommand::Status));
        assert_eq!(
            parse_network_command(b"net.ping(\"10.0.2.2\")").unwrap(),
            Some(NetworkCommand::Ping { address: [10, 0, 2, 2] })
        );
        assert_eq!(
            parse_network_command(b"net.interface[\"eth0\"].status").unwrap(),
            Some(NetworkCommand::InterfaceStatus { name: b"eth0" })
        );
        assert_eq!(
            parse_network_command(b"net.tcp-probe(\"10.0.2.2\", 8080)").unwrap(),
            Some(NetworkCommand::TcpProbe { address: [10, 0, 2, 2], port: 8080 })
        );
        assert_eq!(
            parse_network_command(b"net.fetch(\"http://10.0.2.2:8080/readme\", \"/readme\")")
                .unwrap(),
            Some(NetworkCommand::Fetch {
                url: b"http://10.0.2.2:8080/readme",
                destination: b"/readme"
            })
        );
        assert_eq!(
            parse_network_command(b"net.ping(\"10.0.2.999\")").unwrap_err(),
            NetworkCommandError::Usage
        );
        assert_eq!(
            parse_network_command(b"net.ping(10.0.2.2)").unwrap_err(),
            NetworkCommandError::Usage
        );
        assert_eq!(
            parse_network_command(b"net.tcp-probe(10.0.2.2, 0)").unwrap_err(),
            NetworkCommandError::Usage
        );
    }

    #[test]
    fn service_records_format_with_bounded_output() {
        let mut record = logos_abi::ServiceManagerRecord::EMPTY;
        record.state = logos_abi::ManagerState::Running;
        record.name[..7].copy_from_slice(b"storage");
        record.name_len = 7;
        let mut output = [0; MAX_OUTPUT_BYTES];
        let length = format_service_record(&record, &mut output);
        assert_eq!(&output[..length], b"storage running\r\n");

        let mut short = [0; 5];
        assert_eq!(format_service_record(&record, &mut short), 5);
        assert_eq!(&short, b"stora");

        let length = format_service_property(&record, ServiceProperty::Status, &mut output);
        assert_eq!(&output[..length], b"running\r\n");
        let length = format_service_property(&record, ServiceProperty::Name, &mut output);
        assert_eq!(&output[..length], b"storage\r\n");
        let length = format_service_property(&record, ServiceProperty::Version, &mut output);
        assert_eq!(&output[..length], b"0.1.0\r\n");
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
