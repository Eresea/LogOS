#![no_std]

pub mod interpreter;

pub use interpreter::{
    FlowEvalError, FlowParseError, FlowRuntime, FlowType, FlowTypeError, NamespaceKind,
    OperationRegistry, OperationSignature, Program as FlowProgram, PromiseState, PromiseType,
    Variables as FlowVariables,
};

#[cfg(test)]
extern crate std;

pub const MAX_FLOW_BYTES: usize = 256;
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
    PackageMember,
    FileHandleOpen,
    FileHandleOpenMember,
    FileHandleTouch,
    FileHandleTouchMember,
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
    [b"status", b"ping(\"\")", b"tcp-probe(\"\", 0)", b"fetch(\"\")", b"interface[\""];
pub const SYSTEM_COMPLETION_MEMBERS: [&[u8]; 4] =
    [b"version()", b"uname()", b"shutdown()", b"reboot()"];
pub const FILESYSTEM_COMPLETION_MEMBERS: [&[u8]; 6] = [
    b"list()",
    b"open(\"\")",
    b"touch(\"\").create()",
    b"touch(\"\").write(\"\")",
    b"remove(\"\")",
    b"move(\"\", \"\")",
];
pub const FILESYSTEM_COMPLETION_CURSOR_OFFSETS: [u8; 6] = [6, 6, 8, 8, 8, 6];
pub const FILE_OPEN_COMPLETION_MEMBERS: [&[u8]; 2] = [b".read()", b".write(\"\")"];
pub const FILE_OPEN_COMPLETION_OFFSETS: [u8; 2] = [7, 7];
pub const FILE_OPEN_MEMBER_COMPLETION: [&[u8]; 2] = [b"read()", b"write(\"\")"];
pub const FILE_OPEN_MEMBER_OFFSETS: [u8; 2] = [6, 6];
pub const FILE_TOUCH_COMPLETION_MEMBERS: [&[u8]; 2] = [b".create()", b".write(\"\")"];
pub const FILE_TOUCH_COMPLETION_OFFSETS: [u8; 2] = [9, 7];
pub const FILE_TOUCH_MEMBER_COMPLETION: [&[u8]; 2] = [b"create()", b"write(\"\")"];
pub const FILE_TOUCH_MEMBER_OFFSETS: [u8; 2] = [8, 6];
pub const PACKAGE_COMPLETION_MEMBERS: [&[u8]; 3] = [b"list()", b"info(\"\")", b"install(\"\")"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowKind {
    System,
    Filesystem,
    Service,
    Network,
    Package,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum FlowAction {
    None = 0,
    Shutdown = logos_abi::POWER_SHUTDOWN,
    Reboot = logos_abi::POWER_REBOOT,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlowSpec {
    pub name: &'static [u8],
    pub kind: FlowKind,
    pub usage: &'static [u8],
    pub summary: &'static [u8],
    pub manual: &'static [u8],
}

pub const FLOW_SPECS: [FlowSpec; 5] = [
    FlowSpec {
        name: b"sys",
        kind: FlowKind::System,
        usage: b"sys.version() / sys.uname() / sys.shutdown() / sys.reboot()",
        summary: b"inspect and control the system",
        manual: b"Shows system information or requests shutdown/reboot.",
    },
    FlowSpec {
        name: b"fs",
        kind: FlowKind::Filesystem,
        usage: b"fs.list() / fs.open(\"path\").read() / fs.touch(\"path\").write(\"data\")",
        summary: b"manage files",
        manual: b"Lists, creates, reads, writes, removes, or moves files.",
    },
    FlowSpec {
        name: b"service",
        kind: FlowKind::Service,
        usage: b"service[\"name\"].status / service[\"name\"].restart()",
        summary: b"manage services",
        manual: b"Lists, inspects, starts, stops, or restarts a service.",
    },
    FlowSpec {
        name: b"net",
        kind: FlowKind::Network,
        usage: b"net.status / net.fetch(\"url\") / net.fetch(\"url\", \"path\")",
        summary: b"inspect and probe networking",
        manual: b"Shows network status, probes networking, or fetches an HTTP file.",
    },
    FlowSpec {
        name: b"pkg",
        kind: FlowKind::Package,
        usage: b"pkg.list() / pkg.info(\"name\")",
        summary: b"inspect installed packages",
        manual: b"Lists installed packages or shows one package manifest summary.",
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

    if let Some(context) = file_handle_completion_context(
        before,
        b"fs.open(\"",
        CompletionTarget::FileHandleOpen,
        CompletionTarget::FileHandleOpenMember,
    ) {
        return Ok(Some(context));
    }
    if let Some(context) = file_handle_completion_context(
        before,
        b"fs.touch(\"",
        CompletionTarget::FileHandleTouch,
        CompletionTarget::FileHandleTouchMember,
    ) {
        return Ok(Some(context));
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

    if before.starts_with(b"pkg.") && before[4..].iter().all(|byte| is_member_byte(*byte)) {
        return Ok(Some(CompletionContext {
            target: CompletionTarget::PackageMember,
            replace_start: 4,
            replace_end: cursor,
            prefix: &before[4..],
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
        || line[cursor..].iter().any(|byte| !byte.is_ascii_whitespace())
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

fn file_handle_completion_context<'a>(
    before: &'a [u8],
    call_prefix: &[u8],
    call_target: CompletionTarget,
    member_target: CompletionTarget,
) -> Option<CompletionContext<'a>> {
    if !before.starts_with(call_prefix) {
        return None;
    }
    let close = before[call_prefix.len()..].windows(2).position(|window| window == b"\")")?;
    let call_end = call_prefix.len() + close + 2;
    let suffix = &before[call_end..];
    if suffix.is_empty() {
        return Some(CompletionContext {
            target: call_target,
            replace_start: before.len(),
            replace_end: before.len(),
            prefix: &[],
        });
    }
    if suffix[0] != b'.' || !suffix[1..].iter().all(|byte| is_member_byte(*byte)) {
        return None;
    }
    Some(CompletionContext {
        target: member_target,
        replace_start: call_end + 1,
        replace_end: before.len(),
        prefix: &before[call_end + 1..],
    })
}

fn find_bytes(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes.windows(needle.len()).position(|window| window == needle)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageCommand<'a> {
    List {
        path: &'a [u8],
    },
    Touch {
        path: &'a [u8],
    },
    Cat {
        path: &'a [u8],
    },
    Write {
        path: &'a [u8],
        data: &'a [u8],
    },
    TouchWrite {
        path: &'a [u8],
        data: &'a [u8],
    },
    WriteVariables {
        path: &'a [u8],
        data: &'a [u8],
        path_is_variable: bool,
        data_is_variable: bool,
        create: bool,
    },
    Remove {
        path: &'a [u8],
    },
    Move {
        from: &'a [u8],
        to: &'a [u8],
    },
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageCommand<'a> {
    List,
    Info { name: &'a [u8] },
    Install { path: &'a [u8] },
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
pub enum FlowDiagnostic {
    Parse(FlowParseError),
    Type(FlowTypeError),
    Eval(FlowEvalError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowOperation<'a> {
    Help {
        topic: Option<&'a [u8]>,
    },
    Clear,
    Echo {
        text: &'a [u8],
    },
    EchoVariable {
        name: &'a [u8],
    },
    Storage(StorageCommand<'a>),
    Service(ServiceCommand<'a>),
    Network(NetworkCommand<'a>),
    Package(PackageCommand<'a>),
    System(SystemOperation),
    CancelPromise {
        name: &'a [u8],
    },
    AwaitPromise {
        name: &'a [u8],
    },
    FetchResponse {
        url: &'a [u8],
    },
    FetchResponseVariable {
        name: &'a [u8],
        url: &'a [u8],
        url_is_variable: bool,
    },
    FetchResponseToFile {
        url: &'a [u8],
        destination: &'a [u8],
    },
    FetchResponseToFileVariables {
        url: &'a [u8],
        destination: &'a [u8],
    },
    WriteResponse {
        url: &'a [u8],
        destination: &'a [u8],
    },
    WriteResponseVariables {
        url: &'a [u8],
        destination: &'a [u8],
        url_is_variable: bool,
        destination_is_variable: bool,
    },
    WriteResponsePromise {
        name: &'a [u8],
        destination: &'a [u8],
        destination_is_variable: bool,
    },
}

#[derive(Clone, Copy)]
struct FlowStringVariable {
    name: [u8; interpreter::MAX_VARIABLE_NAME_BYTES],
    name_len: usize,
    value: [u8; MAX_FLOW_BYTES],
    value_len: usize,
}

impl FlowStringVariable {
    const EMPTY: Self = Self {
        name: [0; interpreter::MAX_VARIABLE_NAME_BYTES],
        name_len: 0,
        value: [0; MAX_FLOW_BYTES],
        value_len: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemOperation {
    Version,
    Uname,
    Shutdown,
    Reboot,
}

pub fn check_flow(
    source: &[u8],
    variables: &mut FlowVariables,
) -> Result<FlowType, FlowDiagnostic> {
    let program = FlowProgram::parse(source).map_err(FlowDiagnostic::Parse)?;
    program.type_check(variables).map_err(FlowDiagnostic::Type)
}

/// Converts the typed Flow surface into the fixed service operations.  The
/// registry/type-check pass runs before this adapter, so unsupported aliases
/// cannot reach Storage or Network.
pub fn parse_flow_operation<'a>(
    source: &'a [u8],
) -> Result<Option<FlowOperation<'a>>, FlowDiagnostic> {
    let mut variables = FlowVariables::new();
    check_flow(source, &mut variables)?;
    parse_flow_operation_unchecked(source)
}

fn parse_flow_operation_unchecked<'a>(
    source: &'a [u8],
) -> Result<Option<FlowOperation<'a>>, FlowDiagnostic> {
    let trimmed = trim_command(source);
    let expression = trim_flow_prefix(trimmed);
    if expression == b"help()" {
        return Ok(Some(FlowOperation::Help { topic: None }));
    }
    if expression.starts_with(b"help(") {
        let topic = quoted_after(expression, b"help(")
            .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?;
        return Ok(Some(FlowOperation::Help { topic: Some(topic) }));
    }
    if expression == b"clear()" {
        return Ok(Some(FlowOperation::Clear));
    }
    if expression.starts_with(b"echo(") {
        if let Some(text) = quoted_after(expression, b"echo(") {
            return Ok(Some(FlowOperation::Echo { text }));
        }
        let name = identifier_after(expression, b"echo(")
            .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?;
        return Ok(Some(FlowOperation::EchoVariable { name }));
    }
    if expression.ends_with(b".cancel()") {
        let name = expression
            .strip_suffix(b".cancel()")
            .and_then(|value| value.rsplit(|byte| *byte == b'.').next())
            .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?;
        return Ok(Some(FlowOperation::CancelPromise { name }));
    }
    if trimmed.starts_with(b"await ") && is_identifier(expression) {
        return Ok(Some(FlowOperation::AwaitPromise { name: expression }));
    }
    if contains(expression, b".then(") {
        if !contains(expression, b"net.fetch(") {
            let name = expression
                .split(|byte| *byte == b'.')
                .next()
                .filter(|name| is_identifier(name))
                .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(
                    SpanForDiagnostic::span(),
                )))?;
            let destination = quoted_after(expression, b"fs.touch(")
                .or_else(|| identifier_after(expression, b"fs.touch("))
                .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(
                    SpanForDiagnostic::span(),
                )))?;
            return Ok(Some(FlowOperation::WriteResponsePromise {
                name,
                destination,
                destination_is_variable: quoted_after(expression, b"fs.touch(").is_none(),
            }));
        }
        let url = quoted_after(expression, b"net.fetch(")
            .or_else(|| identifier_after(expression, b"net.fetch("))
            .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?;
        let destination = quoted_after(expression, b"fs.touch(")
            .or_else(|| identifier_after(expression, b"fs.touch("))
            .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?;
        let url_is_variable = quoted_after(expression, b"net.fetch(").is_none();
        let destination_is_variable = quoted_after(expression, b"fs.touch(").is_none();
        if !url_is_variable && !destination_is_variable {
            return Ok(Some(FlowOperation::WriteResponse { url, destination }));
        }
        return Ok(Some(FlowOperation::WriteResponseVariables {
            url,
            destination,
            url_is_variable,
            destination_is_variable,
        }));
    }
    if contains(expression, b"net.fetch(") {
        let url = quoted_after(expression, b"net.fetch(");
        let url_name = url.or_else(|| identifier_after(expression, b"net.fetch("));
        let Some(url_name) = url_name else {
            return Err(FlowDiagnostic::Type(
                FlowTypeError::InvalidCall(SpanForDiagnostic::span()),
            ));
        };
        let destination = quoted_after(expression, b"net.fetch(").and_then(|_first| {
            let start = expression
                .windows(b"net.fetch(".len())
                .position(|window| window == b"net.fetch(")?;
            quoted_after(&expression[start + b"net.fetch(".len()..], b",")
        });
        let destination_name = destination.or_else(|| {
            let start = expression
                .windows(b"net.fetch(".len())
                .position(|window| window == b"net.fetch(")?;
            identifier_after(&expression[start + b"net.fetch(".len()..], b",")
        });
        if let Some(destination) = destination {
            if let Some(url) = url {
                return Ok(Some(FlowOperation::FetchResponseToFile { url, destination }));
            }
        }
        if let Some(destination) = destination_name {
            if url.is_none() {
                return Ok(Some(FlowOperation::FetchResponseToFileVariables {
                    url: url_name,
                    destination,
                }));
            }
        }
        let name = flow_assignment_name(trimmed);
        return Ok(Some(if name.is_some() || url.is_none() {
            FlowOperation::FetchResponseVariable {
                name: name.unwrap_or_default(),
                url: url_name,
                url_is_variable: url.is_none(),
            }
        } else {
            FlowOperation::FetchResponse { url: url.unwrap_or_default() }
        }));
    }
    if (contains(expression, b"fs.touch(") || contains(expression, b"fs.open("))
        && contains(expression, b").write(")
    {
        let path = quoted_after(expression, b"fs.touch(")
            .or_else(|| identifier_after(expression, b"fs.touch("))
            .or_else(|| quoted_after(expression, b"fs.open("))
            .or_else(|| identifier_after(expression, b"fs.open("))
            .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?;
        let data = quoted_after(expression, b").write(")
            .or_else(|| identifier_after(expression, b").write("))
            .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?;
        let path_is_variable = quoted_after(expression, b"fs.touch(").is_none()
            && quoted_after(expression, b"fs.open(").is_none();
        let data_is_variable = quoted_after(expression, b").write(").is_none();
        return Ok(Some(FlowOperation::Storage(if path_is_variable || data_is_variable {
            StorageCommand::WriteVariables {
                path,
                data,
                path_is_variable,
                data_is_variable,
                create: expression.starts_with(b"fs.touch("),
            }
        } else if expression.starts_with(b"fs.touch(") {
            StorageCommand::TouchWrite { path, data }
        } else {
            StorageCommand::Write { path, data }
        })));
    }
    if contains(expression, b"fs.open(") && contains(expression, b").read()") {
        let path = quoted_after(expression, b"fs.open(")
            .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?;
        return Ok(Some(FlowOperation::Storage(StorageCommand::Cat { path })));
    }
    if contains(expression, b"fs.touch(") && contains(expression, b").create()") {
        let path = quoted_after(expression, b"fs.touch(")
            .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?;
        return Ok(Some(FlowOperation::Storage(StorageCommand::Touch { path })));
    }
    if let Some(command) = parse_service_command(expression)
        .map_err(|_| FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?
    {
        return Ok(Some(FlowOperation::Service(command)));
    }
    if let Some(command) = parse_network_command(expression)
        .map_err(|_| FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?
    {
        return Ok(Some(FlowOperation::Network(command)));
    }
    if expression == b"pkg.list()" {
        return Ok(Some(FlowOperation::Package(PackageCommand::List)));
    }
    if expression.starts_with(b"pkg.info(") {
        let name = quoted_after(expression, b"pkg.info(")
            .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?;
        return Ok(Some(FlowOperation::Package(PackageCommand::Info { name })));
    }
    if expression.starts_with(b"pkg.install(") {
        let path = quoted_after(expression, b"pkg.install(")
            .ok_or(FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?;
        return Ok(Some(FlowOperation::Package(PackageCommand::Install { path })));
    }
    if let Some(command) = parse_storage_command(expression)
        .map_err(|_| FlowDiagnostic::Type(FlowTypeError::InvalidCall(SpanForDiagnostic::span())))?
    {
        return Ok(Some(FlowOperation::Storage(command)));
    }
    let system = match expression {
        b"sys.version()" => Some(SystemOperation::Version),
        b"sys.uname()" => Some(SystemOperation::Uname),
        b"sys.shutdown()" => Some(SystemOperation::Shutdown),
        b"sys.reboot()" => Some(SystemOperation::Reboot),
        _ => None,
    };
    if let Some(operation) = system {
        return Ok(Some(FlowOperation::System(operation)));
    }
    Ok(None)
}

fn trim_flow_prefix(source: &[u8]) -> &[u8] {
    let mut source = source;
    loop {
        if source.starts_with(b"var ") {
            if let Some(index) = source.iter().position(|byte| *byte == b'=') {
                source = trim_command(&source[index + 1..]);
                continue;
            }
        }
        if source.starts_with(b"await ") {
            source = trim_command(&source[6..]);
        }
        break;
    }
    source
}

fn contains(source: &[u8], needle: &[u8]) -> bool {
    source.windows(needle.len()).any(|window| window == needle)
}

fn is_identifier(source: &[u8]) -> bool {
    !source.is_empty()
        && source[0].is_ascii_alphabetic()
        && source.iter().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn flow_assignment_name(source: &[u8]) -> Option<&[u8]> {
    if !source.starts_with(b"var ") {
        return None;
    }
    let equal = source.iter().position(|byte| *byte == b'=')?;
    let name = trim_command(&source[4..equal]);
    is_identifier(name).then_some(name)
}

fn quoted_after<'a>(source: &'a [u8], marker: &[u8]) -> Option<&'a [u8]> {
    let start = source.windows(marker.len()).position(|window| window == marker)? + marker.len();
    let rest = &source[start..];
    let quote = rest.iter().position(|byte| *byte == b'"')? + 1;
    let end = rest[quote..].iter().position(|byte| *byte == b'"')? + quote;
    Some(&rest[quote..end])
}

fn identifier_after<'a>(source: &'a [u8], marker: &[u8]) -> Option<&'a [u8]> {
    let start = source.windows(marker.len()).position(|window| window == marker)? + marker.len();
    let rest = &source[start..];
    let start = rest.iter().position(|byte| byte.is_ascii_alphabetic() || *byte == b'_')?;
    let end = rest[start..]
        .iter()
        .position(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
        .map_or(rest.len(), |offset| start + offset);
    Some(&rest[start..end])
}

// Keeps the adapter's diagnostics source-aware without adding an owned error
// object to the fixed service ABI.
struct SpanForDiagnostic;
impl SpanForDiagnostic {
    const fn span() -> interpreter::Span {
        interpreter::Span { start: 0, end: 0 }
    }
}

pub struct FlowService {
    runtime: FlowRuntime,
    strings: [FlowStringVariable; interpreter::MAX_VARIABLES],
}

impl FlowService {
    pub const fn new() -> Self {
        Self {
            runtime: FlowRuntime::new(),
            strings: [FlowStringVariable::EMPTY; interpreter::MAX_VARIABLES],
        }
    }

    pub fn validate(&mut self, source: &[u8]) -> Result<FlowType, FlowDiagnostic> {
        let mut candidate = self.runtime.variables;
        let result = check_flow(source, &mut candidate);
        if result.is_ok() {
            self.runtime.variables = candidate;
            self.remember_string_assignment(source);
        }
        result
    }

    pub fn copy_string_variable(&self, name: &[u8], output: &mut [u8]) -> Option<usize> {
        self.strings[..].iter().find(|entry| &entry.name[..entry.name_len] == name).and_then(
            |entry| {
                (output.len() >= entry.value_len).then(|| {
                    output[..entry.value_len].copy_from_slice(&entry.value[..entry.value_len]);
                    entry.value_len
                })
            },
        )
    }

    pub fn copy_value(&self, name: &[u8], output: &mut [u8]) -> Option<usize> {
        let value = self.runtime.variable(name)?;
        let (bytes, length) = match value {
            interpreter::FlowValue::String { bytes, len }
            | interpreter::FlowValue::Bytes { bytes, len } => (bytes, len),
            _ => return None,
        };
        (output.len() >= length).then(|| {
            output[..length].copy_from_slice(&bytes[..length]);
            length
        })
    }

    fn remember_string_assignment(&mut self, source: &[u8]) {
        let source = trim_command(source);
        let Some(equal) = source.iter().position(|byte| *byte == b'=') else { return };
        let mut name = &source[..equal];
        if name.starts_with(b"var ") {
            name = trim_command(&name[4..]);
        }
        if name.is_empty() || name.len() > interpreter::MAX_VARIABLE_NAME_BYTES {
            return;
        }
        let value = trim_command(&source[equal + 1..]);
        if value.len() < 2 || value[0] != b'"' || value[value.len() - 1] != b'"' {
            return;
        }
        let value = &value[1..value.len() - 1];
        if value.len() > MAX_FLOW_BYTES {
            return;
        }
        let index = self
            .strings
            .iter()
            .position(|entry| entry.name_len == 0 || &entry.name[..entry.name_len] == name);
        let Some(index) = index else { return };
        let mut entry = FlowStringVariable::EMPTY;
        entry.name[..name.len()].copy_from_slice(name);
        entry.name_len = name.len();
        entry.value[..value.len()].copy_from_slice(value);
        entry.value_len = value.len();
        self.strings[index] = entry;
        if let Some(value) = interpreter::FlowValue::string(value) {
            let _ = self.runtime.set_variable(name, value);
        }
    }

    pub fn operation<'a>(
        &mut self,
        source: &'a [u8],
    ) -> Result<Option<FlowOperation<'a>>, FlowDiagnostic> {
        let program = FlowProgram::parse(source).map_err(FlowDiagnostic::Parse)?;
        program.type_check(&mut self.runtime.variables).map_err(FlowDiagnostic::Type)?;
        let operation = parse_flow_operation_unchecked(source)?;
        let materializes_promise = matches!(
            operation,
            Some(FlowOperation::FetchResponseVariable { name, .. }) if !name.is_empty()
        );
        if materializes_promise {
            program.evaluate(&mut self.runtime).map_err(FlowDiagnostic::Eval)?;
            self.runtime.reclaim_temporary_promises();
        }
        self.remember_string_assignment(source);
        Ok(operation)
    }

    pub fn promise_state(&self, name: &[u8]) -> Option<PromiseState> {
        self.runtime.promise_state(name)
    }

    pub fn resolve_response_promise(&mut self, name: &[u8], status: u16, body: &[u8]) -> bool {
        let Some(value) = interpreter::FlowValue::response(status, body) else { return false };
        self.runtime.resolve_variable(name, value).is_ok()
    }

    pub fn take_promise(&mut self, name: &[u8]) -> bool {
        self.runtime.take_variable(name).is_ok()
    }

    pub fn copy_response_promise(&self, name: &[u8], output: &mut [u8]) -> Option<(u16, usize)> {
        let value = self.runtime.variable(name)?;
        let interpreter::FlowValue::Response { status, body, body_len } = value else {
            return None;
        };
        if output.len() < body_len {
            return None;
        }
        output[..body_len].copy_from_slice(&body[..body_len]);
        Some((status, body_len))
    }

    pub fn cancel_promise(&mut self, name: &[u8]) -> bool {
        self.runtime.cancel_variable(name).is_ok()
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

pub fn format_help(topic: Option<&[u8]>, output: &mut [u8]) -> usize {
    let mut length = 0;
    match topic {
        None => {
            append_help(&mut length, output, b"Flow operations:\r\n");
            for spec in FLOW_SPECS {
                append_help(&mut length, output, b"  ");
                append_help(&mut length, output, spec.name);
                append_help(&mut length, output, b" - ");
                append_help(&mut length, output, spec.summary);
                append_help(&mut length, output, b"\r\n");
            }
            append_help(&mut length, output, b"Terminal context:\r\n");
            append_help(&mut length, output, b"  help() - show Flow help\r\n");
            append_help(&mut length, output, b"  clear() - clear the terminal\r\n");
            append_help(&mut length, output, b"  echo(\"text\") - print text\r\n");
            append_help(&mut length, output, b"Use help(\"name\") for details.\r\n");
        }
        Some(topic) => {
            if topic == b"clear" {
                append_help(&mut length, output, b"clear\r\nUsage: clear()\r\n");
                append_help(&mut length, output, b"Clears the terminal display.\r\n");
                return length;
            }
            if topic == b"echo" {
                append_help(&mut length, output, b"echo\r\nUsage: echo(\"text\")\r\n");
                append_help(&mut length, output, b"Prints text or a string variable.\r\n");
                return length;
            }
            let Some(spec) = FLOW_SPECS.iter().find(|spec| spec.name == topic) else {
                append_help(&mut length, output, b"flow: no help for ");
                append_help(&mut length, output, topic);
                append_help(&mut length, output, b"\r\n");
                return length;
            };
            append_help(&mut length, output, spec.name);
            append_help(&mut length, output, b"\r\nUsage: ");
            append_help(&mut length, output, spec.usage);
            append_help(&mut length, output, b"\r\n");
            append_help(&mut length, output, spec.manual);
            append_help(&mut length, output, b"\r\n");
        }
    }
    length
}

fn append_help(length: &mut usize, output: &mut [u8], bytes: &[u8]) {
    let count = bytes.len().min(output.len().saturating_sub(*length));
    output[*length..*length + count].copy_from_slice(&bytes[..count]);
    *length += count;
}

pub fn format_flow_diagnostic(error: FlowDiagnostic, output: &mut [u8]) -> usize {
    match error {
        FlowDiagnostic::Parse(error) => {
            let message = match error {
                FlowParseError::TooLong => b"flow: source exceeds 256 bytes\r\n" as &[u8],
                FlowParseError::TooManyExpressions => b"flow: expression limit reached\r\n",
                FlowParseError::TooManyStatements => b"flow: statement limit reached\r\n",
                FlowParseError::TooManyArguments(_) => b"flow: too many arguments\r\n",
                FlowParseError::CallbackLimit(_) => b"flow: callback nesting limit reached\r\n",
                FlowParseError::InvalidToken(_)
                | FlowParseError::UnexpectedToken(_)
                | FlowParseError::UnterminatedString(_)
                | FlowParseError::MissingExpression(_) => b"flow: syntax error\r\n",
            };
            copy_bounded(message, output)
        }
        FlowDiagnostic::Type(error) => {
            let length = interpreter::format_diagnostic(error, output);
            if length < output.len() {
                output[length] = b'\r';
                if length + 1 < output.len() {
                    output[length + 1] = b'\n';
                    return length + 2;
                }
            }
            length
        }
        FlowDiagnostic::Eval(error) => {
            let message = match error {
                interpreter::FlowEvalError::Type(error) => {
                    return format_flow_diagnostic(FlowDiagnostic::Type(error), output);
                }
                interpreter::FlowEvalError::UnknownValue(_)
                | interpreter::FlowEvalError::Unsupported(_) => {
                    b"flow: unsupported expression" as &[u8]
                }
                interpreter::FlowEvalError::Promise(_) => b"flow: promise error" as &[u8],
            };
            let length = copy_bounded(message, output);
            if length + 2 <= output.len() {
                output[length] = b'\r';
                output[length + 1] = b'\n';
                length + 2
            } else {
                length
            }
        }
    }
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
        (b"remove", 1) => Ok(Some(StorageCommand::Remove { path: path_arg(args[0])? })),
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
            (b"status", Some(call)) if call.arg_count == 0 => Ok(Some(NetworkCommand::Status)),
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
        && expression.call.is_none_or(|call| call.arg_count == 0)
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

impl Default for FlowService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            completion_context(b"pkg.", 4).unwrap().unwrap().target,
            CompletionTarget::PackageMember
        );
        assert_eq!(
            completion_context(b"fs.open(\"test\")", 15).unwrap(),
            Some(CompletionContext {
                target: CompletionTarget::FileHandleOpen,
                replace_start: 15,
                replace_end: 15,
                prefix: b"",
            })
        );
        assert_eq!(
            completion_context(b"fs.open(\"test\").wr", 18).unwrap(),
            Some(CompletionContext {
                target: CompletionTarget::FileHandleOpenMember,
                replace_start: 16,
                replace_end: 18,
                prefix: b"wr",
            })
        );
        assert_eq!(
            completion_context(b"net.interface[\"e", 16).unwrap().unwrap().target,
            CompletionTarget::InterfaceName
        );
        assert!(completion_context(b"help()", 4).unwrap().is_none());
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
            parse_storage_command(b"fs.move(\"/old\", \"/new\")").unwrap(),
            Some(StorageCommand::Move { from: b"/old", to: b"/new" })
        );
        assert_eq!(
            parse_storage_command(b"fs.create(\"/file\")").unwrap_err(),
            StorageCommandError::Usage
        );
        assert_eq!(
            parse_storage_command(b"fs.read(\"/file\")").unwrap_err(),
            StorageCommandError::Usage
        );
        assert_eq!(
            parse_storage_command(b"fs.write(\"/file\", \"data\")").unwrap_err(),
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
        assert_eq!(parse_network_command(b"net.status()").unwrap(), Some(NetworkCommand::Status));
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
    fn canonical_flow_operations_are_typed_before_dispatch() {
        assert_eq!(
            parse_flow_operation(b"help()").unwrap(),
            Some(FlowOperation::Help { topic: None })
        );
        assert_eq!(
            parse_flow_operation(br#"help("fs")"#).unwrap(),
            Some(FlowOperation::Help { topic: Some(b"fs") })
        );
        assert_eq!(parse_flow_operation(b"clear()").unwrap(), Some(FlowOperation::Clear));
        assert_eq!(
            parse_flow_operation(b"pkg.list()").unwrap(),
            Some(FlowOperation::Package(PackageCommand::List))
        );
        assert_eq!(
            parse_flow_operation(br#"pkg.info("flow")"#).unwrap(),
            Some(FlowOperation::Package(PackageCommand::Info { name: b"flow" }))
        );
        assert_eq!(
            parse_flow_operation(br#"pkg.install("/tmp/package")"#).unwrap(),
            Some(FlowOperation::Package(PackageCommand::Install { path: b"/tmp/package" }))
        );
        let mut service = FlowService::new();
        assert_eq!(
            service.operation(b"help()").unwrap(),
            Some(FlowOperation::Help { topic: None })
        );
        assert_eq!(
            parse_flow_operation(br#"echo("hello")"#).unwrap(),
            Some(FlowOperation::Echo { text: b"hello" })
        );
        assert!(service.validate(br#"var text = "hello""#).is_ok());
        assert_eq!(
            service.operation(b"echo(text)").unwrap(),
            Some(FlowOperation::EchoVariable { name: b"text" })
        );
        assert_eq!(
            parse_flow_operation(b"fs.touch(\"/file\").write(\"data\")").unwrap(),
            Some(FlowOperation::Storage(StorageCommand::TouchWrite {
                path: b"/file",
                data: b"data",
            }))
        );
        assert_eq!(
            parse_flow_operation(b"fs.open(\"/file\").write(\"data\")").unwrap(),
            Some(FlowOperation::Storage(StorageCommand::Write { path: b"/file", data: b"data" }))
        );
        assert!(service.validate(br#"var path = "/file""#).is_ok());
        assert!(service.validate(br#"var data = "data""#).is_ok());
        assert_eq!(
            service.operation(b"fs.touch(path).write(data)").unwrap(),
            Some(FlowOperation::Storage(StorageCommand::WriteVariables {
                path: b"path",
                data: b"data",
                path_is_variable: true,
                data_is_variable: true,
                create: true,
            }))
        );
        assert_eq!(
            service.operation(b"net.status").unwrap(),
            Some(FlowOperation::Network(NetworkCommand::Status))
        );
        assert_eq!(
            service.operation(b"net.interface[\"eth0\"].status").unwrap(),
            Some(FlowOperation::Network(NetworkCommand::InterfaceStatus { name: b"eth0" }))
        );
        assert_eq!(
            parse_flow_operation(b"await net.fetch(\"http://10.0.2.2/readme\")").unwrap(),
            Some(FlowOperation::FetchResponse { url: b"http://10.0.2.2/readme" })
        );
        assert_eq!(
            parse_flow_operation(b"net.fetch(\"http://10.0.2.2/readme\", \"/readme\")").unwrap(),
            Some(FlowOperation::FetchResponseToFile {
                url: b"http://10.0.2.2/readme",
                destination: b"/readme",
            })
        );
        let mut service = FlowService::new();
        assert!(service.validate(b"var url = \"http://10.0.2.2/readme\"").is_ok());
        assert_eq!(
            service.operation(b"var response = net.fetch(url)").unwrap(),
            Some(FlowOperation::FetchResponseVariable {
                name: b"response",
                url: b"url",
                url_is_variable: true,
            })
        );
        assert!(service.validate(b"var destination = \"/download\"").is_ok());
        assert_eq!(
            service.operation(b"net.fetch(url, destination)").unwrap(),
            Some(FlowOperation::FetchResponseToFileVariables {
                url: b"url",
                destination: b"destination",
            })
        );
        assert_eq!(
            parse_flow_operation(
                br#"await net.fetch("http://10.0.2.2/readme").then((response) => { return fs.touch("/download").write(response.body); })"#,
            )
            .unwrap(),
            Some(FlowOperation::WriteResponse {
                url: b"http://10.0.2.2/readme",
                destination: b"/download",
            })
        );
        assert!(matches!(
            parse_flow_operation(b"fs.create(\"/file\")"),
            Err(FlowDiagnostic::Type(_))
        ));
    }

    #[test]
    fn help_is_bounded_and_uses_flow_specs() {
        let mut output = [0; MAX_OUTPUT_BYTES];
        let length = format_help(None, &mut output);
        assert!(length <= MAX_OUTPUT_BYTES);
        assert!(output[..length].starts_with(b"Flow operations:\r\n"));

        let length = format_help(Some(b"fs"), &mut output);
        assert!(output[..length].starts_with(b"fs\r\nUsage: fs."));

        let length = format_help(Some(b"missing"), &mut output);
        assert_eq!(&output[..length], b"flow: no help for missing\r\n");

        let length = format_help(Some(b"clear"), &mut output);
        assert_eq!(
            &output[..length],
            b"clear\r\nUsage: clear()\r\nClears the terminal display.\r\n"
        );

        let length = format_help(Some(b"echo"), &mut output);
        assert_eq!(
            &output[..length],
            b"echo\r\nUsage: echo(\"text\")\r\nPrints text or a string variable.\r\n"
        );
    }

    #[test]
    fn failed_flow_does_not_commit_partial_variables() {
        let mut service = FlowService::new();
        assert!(service.validate(b"var url = \"http://10.0.2.2/readme\"; missing()").is_err());
        assert!(matches!(
            service.operation(b"net.fetch(url)"),
            Err(FlowDiagnostic::Type(FlowTypeError::UnknownVariable(_)))
        ));
    }

    #[test]
    fn typed_registry_covers_canonical_namespaces() {
        assert_eq!(
            OperationRegistry::lookup(NamespaceKind::Network, b"fetch")
                .map(|signature| signature.argument_count),
            Some(2)
        );
        assert_eq!(
            OperationRegistry::lookup(NamespaceKind::Network, b"fetch")
                .map(|signature| signature.minimum_argument_count),
            Some(1)
        );
        assert_eq!(
            OperationRegistry::lookup(NamespaceKind::Filesystem, b"list")
                .map(|signature| signature.argument_count),
            Some(0)
        );
        assert!(OperationRegistry::lookup(NamespaceKind::Supervisor, b"restart").is_some());
        assert!(OperationRegistry::lookup(NamespaceKind::Filesystem, b"create").is_none());
    }

    #[test]
    fn flow_service_keeps_named_fetch_promises_and_callbacks_typed() {
        let mut service = std::boxed::Box::new(FlowService::new());
        assert_eq!(
            service.operation(b"var response = net.fetch(\"http://10.0.2.2/readme\")").unwrap(),
            Some(FlowOperation::FetchResponseVariable {
                name: b"response",
                url: b"http://10.0.2.2/readme",
                url_is_variable: false,
            })
        );
        assert_eq!(service.promise_state(b"response"), Some(PromiseState::Pending));
        assert!(service.resolve_response_promise(b"response", 200, b"body"));
        assert_eq!(service.promise_state(b"response"), Some(PromiseState::Ready));
        assert_eq!(
            service.operation(b"await response").unwrap(),
            Some(FlowOperation::AwaitPromise { name: b"response" })
        );
        assert!(service.validate(b"var destination = \"/download\"").is_ok());
        assert_eq!(
            service
                .operation(
                    b"await response.then((value) => { return fs.touch(destination).write(value.body); })"
                )
                .unwrap(),
            Some(FlowOperation::WriteResponsePromise {
                name: b"response",
                destination: b"destination",
                destination_is_variable: true,
            })
        );
        assert!(service.take_promise(b"response"));
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

    #[test]
    fn help_operation_fits_a_service_sized_stack() {
        let service = std::boxed::Box::new(FlowService::new());
        let worker = std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut service = service;
                service.operation(b"help()").unwrap()
            })
            .unwrap();
        assert_eq!(worker.join().unwrap(), Some(FlowOperation::Help { topic: None }));
    }
}
