use logos_abi::{
    IpcBytes, IpcStatus, MessageKind, StorageApiOperation, StorageApiRequest, StorageApiResponse,
    StorageApiStatus,
};

use super::storage::{status_text, storage_ipc_error};
use crate::{
    MAX_FLOW_BYTES, MAX_OUTPUT_BYTES, PackageCommand, PendingOutput, Port, Transport,
    root_relative_path,
};

#[derive(Clone, Copy)]
enum PackageWork {
    List,
    Info,
    Install,
}

pub struct PackageClient {
    work: Option<PackageWork>,
    name: [u8; MAX_FLOW_BYTES],
    name_len: usize,
    active: bool,
    done: bool,
    sent: bool,
    request_id: u32,
    cursor: u32,
    result: [u8; MAX_OUTPUT_BYTES],
    result_len: usize,
    cancelled: bool,
}

impl Default for PackageClient {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageClient {
    pub fn done(&self) -> bool {
        self.done
    }

    pub const fn new() -> Self {
        Self {
            work: None,
            name: [0; MAX_FLOW_BYTES],
            name_len: 0,
            active: false,
            done: false,
            sent: false,
            request_id: 1,
            cursor: 0,
            result: [0; MAX_OUTPUT_BYTES],
            result_len: 0,
            cancelled: false,
        }
    }

    pub fn start(&mut self, command: PackageCommand<'_>) -> bool {
        if self.active || self.done {
            return false;
        }
        self.name_len = 0;
        self.result_len = 0;
        self.cursor = 0;
        self.sent = false;
        self.cancelled = false;
        self.work = match command {
            PackageCommand::List => Some(PackageWork::List),
            PackageCommand::Info { name } => {
                if name.is_empty() || name.len() > self.name.len() {
                    return false;
                }
                self.name[..name.len()].copy_from_slice(name);
                self.name_len = name.len();
                Some(PackageWork::Info)
            }
            PackageCommand::Install { path } => {
                let Some(path) = root_relative_path(path, &mut self.name) else {
                    return false;
                };
                self.name_len = path.len();
                Some(PackageWork::Install)
            }
        };
        self.active = true;
        true
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn cancel(&mut self) {
        if self.active {
            self.cancelled = true;
        }
    }

    fn request(&self) -> Option<IpcBytes> {
        let (operation, offset, path) = match self.work {
            Some(PackageWork::List) => (StorageApiOperation::PackageList, self.cursor, &[][..]),
            Some(PackageWork::Info) => {
                (StorageApiOperation::PackageInfo, 0, &self.name[..self.name_len])
            }
            Some(PackageWork::Install) => {
                (StorageApiOperation::PackageInstall, 0, &self.name[..self.name_len])
            }
            None => return None,
        };
        StorageApiRequest::encode(operation, 0, self.request_id, 0, offset, path, &[], &[])
    }

    pub fn drive<T: Transport>(&mut self, transport: &mut T) -> bool {
        if !self.active {
            return false;
        }
        if self.cancelled && !self.sent {
            self.fail(StorageApiStatus::Unsupported);
            return true;
        }
        if !self.sent {
            let Some(request) = self.request() else {
                self.fail(StorageApiStatus::Invalid);
                return true;
            };
            match transport.send(Port::Storage, &request) {
                IpcStatus::Ok => self.sent = true,
                IpcStatus::Full => return false,
                status => {
                    self.fail(storage_ipc_error(status));
                    return true;
                }
            }
            return true;
        }
        let mut message = IpcBytes::empty(MessageKind::StorageResponse);
        match transport.receive(Port::Storage, &mut message) {
            IpcStatus::Ok => self.sent = false,
            IpcStatus::Empty => return false,
            status => {
                self.fail(storage_ipc_error(status));
                return true;
            }
        }
        let Ok(response) = StorageApiResponse::decode(&message) else {
            self.fail(StorageApiStatus::Invalid);
            return true;
        };
        self.handle_response(response);
        true
    }

    fn handle_response(&mut self, response: StorageApiResponse<'_>) {
        if response.request_id != self.request_id {
            if self.cancelled {
                self.sent = true;
            } else {
                self.fail(StorageApiStatus::Stale);
            }
            return;
        }
        if self.cancelled {
            self.fail(StorageApiStatus::Unsupported);
            return;
        }
        if response.status != StorageApiStatus::Ok {
            self.fail(response.status);
            return;
        }
        self.append(response.data);
        if response.more {
            if response.data.is_empty() || self.result_len == self.result.len() {
                self.fail(StorageApiStatus::Invalid);
            } else {
                self.cursor = self.cursor.saturating_add(1);
                self.request_id = self.request_id.wrapping_add(1).max(1);
            }
        } else {
            self.succeed();
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        let amount = bytes.len().min(self.result.len().saturating_sub(self.result_len));
        self.result[self.result_len..self.result_len + amount].copy_from_slice(&bytes[..amount]);
        self.result_len += amount;
    }

    fn fail(&mut self, status: StorageApiStatus) {
        self.result_len = 0;
        if self.cancelled {
            self.append(b"command cancelled\r\n");
            self.cancelled = false;
        } else {
            self.append(status_text(status));
        }
        self.active = false;
        self.done = true;
    }

    fn succeed(&mut self) {
        self.active = false;
        self.done = true;
    }

    pub fn take_result(&mut self, pending: &mut PendingOutput) {
        if self.done {
            pending.stage(&self.result[..self.result_len]);
            self.done = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::fake::FakeTransport;
    use std::vec::Vec;

    fn request(transport: &FakeTransport) -> (StorageApiOperation, u32, u32, Vec<u8>) {
        let message: IpcBytes = transport.last_sent(Port::Storage);
        let request = StorageApiRequest::decode(&message).unwrap();
        (request.operation, request.request_id, request.offset, request.path.to_vec())
    }

    /// Send the next request and answer it with `request_id` (`None` echoes the
    /// request's own id).
    fn exchange(
        client: &mut PackageClient,
        transport: &mut FakeTransport,
        status: StorageApiStatus,
        data: &[u8],
        more: bool,
        request_id: Option<u32>,
    ) -> (StorageApiOperation, u32, u32, Vec<u8>) {
        assert!(client.drive(transport), "request is sent");
        let seen = request(transport);
        let reply = StorageApiResponse::encode(status, request_id.unwrap_or(seen.1), 0, data, more)
            .unwrap();
        transport.reply(Port::Storage, &reply);
        assert!(client.drive(transport), "reply is consumed");
        seen
    }

    fn output(client: &mut PackageClient) -> Vec<u8> {
        assert!(client.done() && !client.active());
        let mut pending = PendingOutput::new();
        client.take_result(&mut pending);
        pending.staged().to_vec()
    }

    #[test]
    fn list_pages_until_the_last_chunk() {
        let mut transport = FakeTransport::new();
        let mut client = PackageClient::new();
        assert!(client.start(PackageCommand::List));
        let first =
            exchange(&mut client, &mut transport, StorageApiStatus::Ok, b"a 1\r\n", true, None);
        assert_eq!((first.0, first.2), (StorageApiOperation::PackageList, 0));
        let second =
            exchange(&mut client, &mut transport, StorageApiStatus::Ok, b"b 2\r\n", false, None);
        assert_eq!(second.2, 1, "the second page asks for the next cursor");
        assert_ne!(first.1, second.1, "each page uses a fresh request id");
        assert_eq!(output(&mut client), b"a 1\r\nb 2\r\n");
    }

    #[test]
    fn info_and_install_send_the_package_name_or_root_relative_path() {
        let mut transport = FakeTransport::new();
        let mut client = PackageClient::new();
        assert!(client.start(PackageCommand::Info { name: b"storage" }));
        let info = exchange(
            &mut client,
            &mut transport,
            StorageApiStatus::Ok,
            b"storage 1\r\n",
            false,
            None,
        );
        assert_eq!(
            (info.0, info.3.as_slice()),
            (StorageApiOperation::PackageInfo, &b"storage"[..])
        );
        assert_eq!(output(&mut client), b"storage 1\r\n");

        assert!(client.start(PackageCommand::Install { path: b"app.pkg" }));
        let install = exchange(
            &mut client,
            &mut transport,
            StorageApiStatus::Ok,
            b"installed\r\n",
            false,
            None,
        );
        assert_eq!(
            (install.0, install.3.as_slice()),
            (StorageApiOperation::PackageInstall, &b"/app.pkg"[..])
        );
        assert_eq!(output(&mut client), b"installed\r\n");
    }

    #[test]
    fn mismatched_request_id_fails_as_stale() {
        let mut transport = FakeTransport::new();
        let mut client = PackageClient::new();
        assert!(client.start(PackageCommand::List));
        assert!(client.drive(&mut transport));
        let wrong = request(&transport).1.wrapping_add(1);
        let reply =
            StorageApiResponse::encode(StorageApiStatus::Ok, wrong, 0, b"x", false).unwrap();
        transport.reply(Port::Storage, &reply);
        assert!(client.drive(&mut transport));
        assert_eq!(output(&mut client), b"stale transaction\r\n");
    }

    #[test]
    fn error_status_maps_to_the_same_text() {
        let mut transport = FakeTransport::new();
        let mut client = PackageClient::new();
        assert!(client.start(PackageCommand::Info { name: b"nope" }));
        exchange(&mut client, &mut transport, StorageApiStatus::NotFound, b"", false, None);
        assert_eq!(output(&mut client), b"not found\r\n");
    }

    #[test]
    fn package_import_reports_storage_failures() {
        let mut transport = FakeTransport::new();
        let mut client = PackageClient::new();
        assert!(client.start(PackageCommand::Install { path: b"/bad.pkg" }));
        exchange(&mut client, &mut transport, StorageApiStatus::Corrupt, b"", false, None);
        assert_eq!(output(&mut client), b"storage format is invalid\r\n");
    }

    #[test]
    fn cancelled_request_drains_its_response() {
        let mut transport = FakeTransport::new();
        let mut client = PackageClient::new();
        assert!(client.start(PackageCommand::List));
        assert!(client.drive(&mut transport));
        client.cancel();
        let id = request(&transport).1;
        let stray =
            StorageApiResponse::encode(StorageApiStatus::Ok, id.wrapping_add(1), 0, b"", false)
                .unwrap();
        transport.reply(Port::Storage, &stray);
        assert!(client.drive(&mut transport));
        assert!(client.active(), "an unrelated response keeps waiting for ours");
        let reply =
            StorageApiResponse::encode(StorageApiStatus::Ok, id, 0, b"storage 1.0.0\r\n", false)
                .unwrap();
        transport.reply(Port::Storage, &reply);
        assert!(client.drive(&mut transport));
        assert_eq!(output(&mut client), b"command cancelled\r\n");
    }

    #[test]
    fn cancelling_before_send_never_reaches_storage() {
        let mut transport = FakeTransport::new();
        let mut client = PackageClient::new();
        assert!(client.start(PackageCommand::List));
        client.cancel();
        assert!(client.drive(&mut transport));
        assert_eq!(transport.sent_count(Port::Storage), 0);
        assert_eq!(output(&mut client), b"command cancelled\r\n");
    }

    #[test]
    fn busy_client_and_bad_names_are_refused() {
        let mut client = PackageClient::new();
        assert!(!client.start(PackageCommand::Info { name: b"" }));
        assert!(!client.start(PackageCommand::Info { name: &[b'a'; MAX_FLOW_BYTES + 1] }));
        assert!(client.start(PackageCommand::List));
        assert!(!client.start(PackageCommand::List));
    }

    #[test]
    fn dead_peer_reports_the_cause() {
        let mut transport = FakeTransport::new();
        let mut client = PackageClient::new();
        assert!(client.start(PackageCommand::List));
        assert!(client.drive(&mut transport));
        transport.reply_status(Port::Storage, IpcStatus::Disconnected);
        assert!(client.drive(&mut transport));
        assert_eq!(output(&mut client), b"storage service unavailable\r\n");
    }
}
