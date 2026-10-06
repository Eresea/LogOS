use core::{mem, ptr};

use logos_abi::{
    COMPLETION_FLAG_TRUNCATED, CompletionRequest, CompletionResponse, CompletionStatus, IpcBytes,
    MAX_COMPLETION_ITEM_BYTES, MessageKind,
};

use crate::{
    CompletionTarget, DEVICE_COMPLETION_MEMBERS, FILE_OPEN_COMPLETION_MEMBERS,
    FILE_OPEN_MEMBER_COMPLETION, FILE_TOUCH_COMPLETION_MEMBERS, FILE_TOUCH_MEMBER_COMPLETION,
    FILESYSTEM_COMPLETION_MEMBERS, FLOW_SPECS, FlowKind, NETWORK_COMPLETION_MEMBERS,
    PACKAGE_COMPLETION_MEMBERS, PROGRAM_COMPLETION_MEMBERS, SERVICE_COMPLETION_MEMBERS,
    SYSTEM_COMPLETION_MEMBERS, completion_context, completion_cursor_offset,
};

/// Static Flow completion provider. Service names are the only dynamic
/// candidates; the image supplies them through `service_names`.
pub struct CompletionService {
    enabled: bool,
}

impl Default for CompletionService {
    fn default() -> Self {
        Self::new()
    }
}

impl CompletionService {
    pub const fn new() -> Self {
        Self { enabled: true }
    }

    /// `service_names(prefix, response)` appends the live service names that
    /// start with `prefix`; `Err` marks the provider unavailable.
    pub fn complete(
        &mut self,
        request: CompletionRequest,
        mut service_names: impl FnMut(&[u8], &mut CompletionResponse) -> Result<(), ()>,
    ) -> CompletionResponse {
        if !self.enabled || !request.is_valid() {
            let mut response =
                CompletionResponse::empty(request.request_id, CompletionStatus::Unavailable);
            response.line_revision = request.line_revision;
            return response;
        }
        let Some(line) = request.line() else {
            let mut response =
                CompletionResponse::empty(request.request_id, CompletionStatus::Malformed);
            response.line_revision = request.line_revision;
            return response;
        };
        let Ok(Some(context)) = completion_context(line, usize::from(request.cursor)) else {
            let mut response =
                CompletionResponse::empty(request.request_id, CompletionStatus::NoMatch);
            response.line_revision = request.line_revision;
            return response;
        };
        let mut response = CompletionResponse::empty(request.request_id, CompletionStatus::Ok);
        response.line_revision = request.line_revision;
        response.replace_start = context.replace_start as u8;
        response.replace_end = context.replace_end as u8;
        match context.target {
            CompletionTarget::Root => {
                if b"help".starts_with(context.prefix)
                    && !response
                        .push_candidate_with_cursor(b"help()", completion_cursor_offset(b"help()"))
                {
                    response.flags |= COMPLETION_FLAG_TRUNCATED;
                }
                if b"clear".starts_with(context.prefix)
                    && !response.push_candidate_with_cursor(
                        b"clear()",
                        completion_cursor_offset(b"clear()"),
                    )
                {
                    response.flags |= COMPLETION_FLAG_TRUNCATED;
                }
                if b"echo".starts_with(context.prefix)
                    && !response.push_candidate_with_cursor(
                        b"echo(\"\")",
                        completion_cursor_offset(b"echo(\"\")"),
                    )
                {
                    response.flags |= COMPLETION_FLAG_TRUNCATED;
                }
                for spec in FLOW_SPECS {
                    if !spec.name.starts_with(context.prefix) {
                        continue;
                    }
                    let punctuation = match spec.kind {
                        FlowKind::Filesystem => b".".as_slice(),
                        FlowKind::Service => b"[\"".as_slice(),
                        FlowKind::Network => b".".as_slice(),
                        FlowKind::System => b".".as_slice(),
                        FlowKind::Package => b".".as_slice(),
                        FlowKind::Program => b".".as_slice(),
                        FlowKind::Device => b".".as_slice(),
                    };
                    let mut candidate = [0; MAX_COMPLETION_ITEM_BYTES];
                    let Some(length) = copy_candidate(&mut candidate, spec.name, punctuation)
                    else {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        continue;
                    };
                    if !response.push_candidate_with_cursor(
                        &candidate[..length],
                        completion_cursor_offset(&candidate[..length]),
                    ) {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            CompletionTarget::ServiceName => {
                if service_names(context.prefix, &mut response).is_err() {
                    response.status = CompletionStatus::Unavailable;
                }
            }
            CompletionTarget::ServiceMember => {
                for candidate in SERVICE_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            CompletionTarget::NetworkMember => {
                for candidate in NETWORK_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            CompletionTarget::SystemMember => {
                for candidate in SYSTEM_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            CompletionTarget::FilesystemMember => {
                for candidate in FILESYSTEM_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            CompletionTarget::PackageMember => {
                for candidate in PACKAGE_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            CompletionTarget::ProgramMember => {
                for candidate in PROGRAM_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            CompletionTarget::DeviceMember => {
                for candidate in DEVICE_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            CompletionTarget::FileHandleOpen
            | CompletionTarget::FileHandleOpenMember
            | CompletionTarget::FileHandleTouch
            | CompletionTarget::FileHandleTouchMember => {
                let candidates = match context.target {
                    CompletionTarget::FileHandleOpen => &FILE_OPEN_COMPLETION_MEMBERS,
                    CompletionTarget::FileHandleOpenMember => &FILE_OPEN_MEMBER_COMPLETION,
                    CompletionTarget::FileHandleTouch => &FILE_TOUCH_COMPLETION_MEMBERS,
                    CompletionTarget::FileHandleTouchMember => &FILE_TOUCH_MEMBER_COMPLETION,
                    _ => unreachable!(),
                };
                for candidate in candidates {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            CompletionTarget::InterfaceName => {
                if b"eth0".starts_with(context.prefix)
                    && !response.push_candidate_with_cursor(
                        b"eth0\"]",
                        completion_cursor_offset(b"eth0\"]"),
                    )
                {
                    response.flags |= COMPLETION_FLAG_TRUNCATED;
                }
            }
        }
        if response.candidate_count == 0 && response.status == CompletionStatus::Ok {
            response.status = CompletionStatus::NoMatch;
        }
        response
    }
}

pub fn copy_candidate(
    output: &mut [u8; MAX_COMPLETION_ITEM_BYTES],
    first: &[u8],
    second: &[u8],
) -> Option<usize> {
    let length = first.len().checked_add(second.len())?;
    if length > output.len() {
        return None;
    }
    output[..first.len()].copy_from_slice(first);
    output[first.len()..length].copy_from_slice(second);
    Some(length)
}

pub fn completion_request(message: &IpcBytes) -> Option<CompletionRequest> {
    (message.kind == MessageKind::CompletionRequest
        && message.len as usize == core::mem::size_of::<CompletionRequest>())
    .then(|| unsafe { ptr::read_unaligned(message.bytes.as_ptr().cast()) })
    .filter(|request: &CompletionRequest| request.is_valid())
}

pub fn completion_message(response: CompletionResponse) -> IpcBytes {
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&response as *const CompletionResponse).cast::<u8>(),
            mem::size_of::<CompletionResponse>(),
        )
    };
    IpcBytes::from_bytes(MessageKind::CompletionResponse, bytes)
        .unwrap_or_else(|| IpcBytes::empty(MessageKind::CompletionResponse))
}

pub fn trim_flow_input(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    &bytes[start..]
}

pub fn flow_is_foreground(bytes: &[u8]) -> bool {
    trim_flow_input(bytes).starts_with(b"await ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(
        provider: &mut CompletionService,
        request: CompletionRequest,
    ) -> CompletionResponse {
        provider.complete(request, |_, _| Ok(()))
    }

    #[test]
    fn completion_provider_returns_targeted_static_candidates() {
        let mut provider = CompletionService::new();
        let root = complete(&mut provider, CompletionRequest::new(1, b"f", 1).unwrap());
        assert_eq!(root.status, CompletionStatus::Ok);
        assert_eq!(root.candidate(0), Some(&b"fs."[..]));

        let help = complete(&mut provider, CompletionRequest::new(4, b"hel", 3).unwrap());
        assert_eq!(help.candidate(0), Some(&b"help()"[..]));

        let repeated_help =
            complete(&mut provider, CompletionRequest::new(10, b"help()", 4).unwrap());
        assert_eq!(repeated_help.status, CompletionStatus::NoMatch);

        let clear = complete(&mut provider, CompletionRequest::new(8, b"cle", 3).unwrap());
        assert_eq!(clear.candidate(0), Some(&b"clear()"[..]));

        let echo = complete(&mut provider, CompletionRequest::new(9, b"ech", 3).unwrap());
        assert_eq!(echo.candidate(0), Some(&b"echo(\"\")"[..]));
        assert_eq!(echo.cursor_offsets[0], 6);

        let fs = complete(&mut provider, CompletionRequest::new(5, b"fs.l", 4).unwrap());
        assert_eq!(fs.candidate(0), Some(&b"list()"[..]));
        assert_eq!(fs.cursor_offsets[0], 6);

        let fs_touch = complete(&mut provider, CompletionRequest::new(7, b"fs.t", 4).unwrap());
        assert_eq!(fs_touch.candidate(0), Some(&b"touch(\"\").create()"[..]));
        assert_eq!(fs_touch.cursor_offsets[0], 7);

        let fs_move = complete(&mut provider, CompletionRequest::new(13, b"fs.mo", 5).unwrap());
        assert_eq!(fs_move.candidate(0), Some(&b"move(\"\", \"\")"[..]));
        assert_eq!(fs_move.cursor_offsets[0], 6);

        let network = complete(&mut provider, CompletionRequest::new(14, b"net.", 4).unwrap());
        assert_eq!(network.candidate(1), Some(&b"ping(\"\")"[..]));
        assert_eq!(network.cursor_offsets[1], 6);
        assert_eq!(network.candidate(2), Some(&b"tcp-probe(\"\", 0)"[..]));
        assert_eq!(network.cursor_offsets[2], 11);
        assert_eq!(network.candidate(3), Some(&b"fetch(\"\")"[..]));
        assert_eq!(network.cursor_offsets[3], 7);

        let sys = complete(&mut provider, CompletionRequest::new(6, b"sys.v", 5).unwrap());
        assert_eq!(sys.candidate(0), Some(&b"version()"[..]));

        let member = complete(
            &mut provider,
            CompletionRequest::new(2, b"service[\"storage\"].re", 21).unwrap(),
        );
        assert_eq!(member.candidate(0), Some(&b"restart()"[..]));

        let file_handle =
            complete(&mut provider, CompletionRequest::new(11, b"fs.open(\"test\").", 16).unwrap());
        assert_eq!(file_handle.candidate_count, 2);
        assert_eq!(file_handle.candidate(0), Some(&b"read()"[..]));
        assert_eq!(file_handle.candidate(1), Some(&b"write(\"\")"[..]));
        assert_eq!(file_handle.cursor_offsets[0], 6);
        assert_eq!(file_handle.cursor_offsets[1], 7);

        let packages = complete(&mut provider, CompletionRequest::new(15, b"pkg.", 4).unwrap());
        assert_eq!(packages.cursor_offsets[0], 6);
        assert_eq!(packages.cursor_offsets[1], 6);
        assert_eq!(packages.cursor_offsets[2], 9);

        let filtered_file_handle = complete(
            &mut provider,
            CompletionRequest::new(12, b"fs.open(\"test\").re", 18).unwrap(),
        );
        assert_eq!(filtered_file_handle.candidate_count, 1);
        assert_eq!(filtered_file_handle.candidate(0), Some(&b"read()"[..]));

        let interface =
            complete(&mut provider, CompletionRequest::new(3, b"net.interface[\"e", 16).unwrap());
        assert_eq!(interface.candidate(0), Some(&b"eth0\"]"[..]));
    }

    #[test]
    fn service_names_come_from_the_supplied_lookup() {
        let mut provider = CompletionService::new();
        let request = CompletionRequest::new(1, b"service[\"st", 11).unwrap();
        let response = provider.complete(request, |prefix, response| {
            assert_eq!(prefix, b"st");
            assert!(response.push_candidate(b"storage\"]"));
            Ok(())
        });
        assert_eq!(response.status, CompletionStatus::Ok);
        assert_eq!(response.candidate(0), Some(&b"storage\"]"[..]));
    }

    #[test]
    fn failing_service_lookup_makes_completion_unavailable() {
        let mut provider = CompletionService::new();
        let request = CompletionRequest::new(1, b"service[\"", 9).unwrap();
        let response = provider.complete(request, |_, _| Err(()));
        assert_eq!(response.status, CompletionStatus::Unavailable);
    }

    #[test]
    fn foreground_flow_input_starts_with_await() {
        assert!(flow_is_foreground(b"  await fetch"));
        assert!(!flow_is_foreground(b"fetch"));
        assert_eq!(trim_flow_input(b" \t x "), b"x ");
    }

    #[test]
    fn completion_messages_round_trip_through_ipc_bytes() {
        let request = CompletionRequest::new(5, b"fs.l", 4).unwrap();
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&request as *const CompletionRequest).cast::<u8>(),
                mem::size_of::<CompletionRequest>(),
            )
        };
        let message = IpcBytes::from_bytes(MessageKind::CompletionRequest, bytes).unwrap();
        assert_eq!(completion_request(&message).map(|r| r.request_id), Some(5));
        let wrong_kind = IpcBytes::from_bytes(MessageKind::SessionInput, bytes).unwrap();
        assert!(completion_request(&wrong_kind).is_none());

        let response = CompletionResponse::empty(5, CompletionStatus::NoMatch);
        let reply = completion_message(response);
        assert_eq!(reply.kind, MessageKind::CompletionResponse);
        assert_eq!(usize::from(reply.len), mem::size_of::<CompletionResponse>());
    }
}
