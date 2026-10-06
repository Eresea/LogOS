use logos_abi::{DeviceOperation, DeviceRequest, DeviceResponse, DeviceStatus, IpcStatus};

use crate::{IdSpace, MAX_OUTPUT_BYTES, PendingOutput, Port, Transport};

pub struct DeviceClient {
    active: bool,
    done: bool,
    sent: bool,
    request_id: u32,
    result: [u8; MAX_OUTPUT_BYTES],
    result_len: usize,
}

impl Default for DeviceClient {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceClient {
    pub const fn new() -> Self {
        Self {
            active: false,
            done: false,
            sent: false,
            request_id: 1,
            result: [0; MAX_OUTPUT_BYTES],
            result_len: 0,
        }
    }

    pub fn start<T: Transport>(
        &mut self,
        transport: &mut T,
        command: crate::DeviceCommand,
    ) -> bool {
        if self.active || self.done || command != crate::DeviceCommand::List {
            return false;
        }
        self.active = true;
        self.sent = false;
        self.result_len = 0;
        self.request_id = transport.next_request_id(IdSpace::Device);
        true
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn done(&self) -> bool {
        self.done
    }

    pub fn drive<T: Transport>(&mut self, transport: &mut T) -> bool {
        if !self.active {
            return false;
        }
        let request = DeviceRequest::new(DeviceOperation::List, self.request_id);
        if !self.sent {
            match transport.send(Port::Device, &request) {
                IpcStatus::Ok => {
                    self.sent = true;
                }
                IpcStatus::Full => return false,
                _ => self.fail(b"device manager unavailable\r\n"),
            }
            return true;
        }
        let mut response = DeviceResponse::new(request, DeviceStatus::Invalid, 1, 1);
        match transport.receive(Port::Device, &mut response) {
            IpcStatus::Ok => {}
            IpcStatus::Empty => return false,
            _ => {
                self.fail(b"device manager unavailable\r\n");
                return true;
            }
        }
        if !response.is_valid_for(request) {
            self.fail(b"device inventory malformed\r\n");
            return true;
        }
        if response.status != DeviceStatus::Ok {
            self.fail(b"device inventory unavailable\r\n");
            return true;
        }
        let mut manager = logos_device::DeviceManager::new();
        if manager.publish(response).is_err() {
            self.fail(b"device inventory malformed\r\n");
            return true;
        }
        self.result_len = manager.format_list(&mut self.result);
        self.active = false;
        self.done = true;
        true
    }

    fn fail(&mut self, message: &[u8]) {
        self.result_len = message.len().min(self.result.len());
        self.result[..self.result_len].copy_from_slice(&message[..self.result_len]);
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
    use crate::{DeviceCommand, transport::fake::FakeTransport};
    use logos_abi::DeviceRecord;

    fn start_and_send(transport: &mut FakeTransport, client: &mut DeviceClient) -> DeviceRequest {
        assert!(client.start(transport, DeviceCommand::List));
        assert!(client.drive(transport), "request sent");
        transport.last_sent(Port::Device)
    }

    fn reply(transport: &mut FakeTransport, status: DeviceStatus, request: DeviceRequest) {
        let record = DeviceRecord::disk(0, 32, b"disk0").unwrap();
        transport
            .reply(Port::Device, &DeviceResponse::new(request, status, 1, 1).with_record(record));
    }

    fn result(client: &mut DeviceClient) -> std::vec::Vec<u8> {
        let mut pending = PendingOutput::new();
        client.take_result(&mut pending);
        pending.staged().to_vec()
    }

    #[test]
    fn lists_devices_through_the_transport() {
        let mut transport = FakeTransport::new();
        let mut client = DeviceClient::new();
        let request = start_and_send(&mut transport, &mut client);
        assert_eq!(request.operation as u8, DeviceOperation::List as u8);
        reply(&mut transport, DeviceStatus::Ok, request);
        assert!(client.drive(&mut transport));
        assert!(client.done());
        assert_eq!(result(&mut client), b"disk0: disk, 4096-byte blocks, 32 blocks, ready\r\n");
    }

    #[test]
    fn mismatched_request_id_is_not_accepted() {
        let mut transport = FakeTransport::new();
        let mut client = DeviceClient::new();
        let mut request = start_and_send(&mut transport, &mut client);
        request.request_id = request.request_id.wrapping_add(1);
        reply(&mut transport, DeviceStatus::Ok, request);
        assert!(client.drive(&mut transport));
        assert_eq!(result(&mut client), b"device inventory malformed\r\n");
    }

    #[test]
    fn error_status_maps_to_unavailable_text() {
        let mut transport = FakeTransport::new();
        let mut client = DeviceClient::new();
        let request = start_and_send(&mut transport, &mut client);
        reply(&mut transport, DeviceStatus::Invalid, request);
        assert!(client.drive(&mut transport));
        assert_eq!(result(&mut client), b"device inventory unavailable\r\n");
    }

    #[test]
    fn transport_failures_map_to_manager_unavailable_text() {
        let mut transport = FakeTransport::new();
        let mut client = DeviceClient::new();
        start_and_send(&mut transport, &mut client);
        transport.reply_status(Port::Device, IpcStatus::Disconnected);
        assert!(client.drive(&mut transport));
        assert_eq!(result(&mut client), b"device manager unavailable\r\n");

        let mut transport = FakeTransport::new();
        transport.fail_sends(Port::Device, IpcStatus::Unauthorized);
        let mut client = DeviceClient::new();
        assert!(client.start(&mut transport, DeviceCommand::List));
        assert!(client.drive(&mut transport));
        assert_eq!(result(&mut client), b"device manager unavailable\r\n");
    }

    #[test]
    fn full_queue_retries_and_second_start_is_refused() {
        let mut transport = FakeTransport::new();
        transport.fail_sends(Port::Device, IpcStatus::Full);
        let mut client = DeviceClient::new();
        assert!(client.start(&mut transport, DeviceCommand::List));
        assert!(!client.drive(&mut transport));
        assert!(client.active());
        assert!(!client.start(&mut transport, DeviceCommand::List));
    }
}
