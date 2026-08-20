#![no_std]

#[cfg(test)]
extern crate std;

use logos_abi::{DeviceRecord, DeviceResponse, DeviceStatus, MAX_DEVICES};

pub const MAX_DEVICE_LIST_BYTES: usize = 512;

pub struct DeviceManager {
    records: [DeviceRecord; MAX_DEVICES],
    count: usize,
}

impl DeviceManager {
    pub const fn new() -> Self {
        Self { records: [DeviceRecord::EMPTY; MAX_DEVICES], count: 0 }
    }

    pub fn publish(&mut self, response: DeviceResponse) -> Result<(), DeviceStatus> {
        if response.status != DeviceStatus::Ok {
            return Err(response.status);
        }
        if usize::from(response.count) > MAX_DEVICES
            || response.records[..usize::from(response.count)]
                .iter()
                .any(|record| !record.is_valid())
        {
            return Err(DeviceStatus::Invalid);
        }
        self.records = [DeviceRecord::EMPTY; MAX_DEVICES];
        self.records[..usize::from(response.count)]
            .copy_from_slice(&response.records[..usize::from(response.count)]);
        self.count = usize::from(response.count);
        Ok(())
    }

    pub const fn count(&self) -> usize {
        self.count
    }

    pub const fn record(&self, index: usize) -> Option<DeviceRecord> {
        if index < self.count { Some(self.records[index]) } else { None }
    }

    pub fn format_list(&self, output: &mut [u8]) -> usize {
        if self.count == 0 {
            return copy_bounded(b"no devices\r\n", output);
        }
        let mut length = 0;
        for record in &self.records[..self.count] {
            let name_len =
                record.name.iter().position(|byte| *byte == 0).unwrap_or(record.name.len());
            append(&mut length, output, &record.name[..name_len]);
            append(&mut length, output, b": ");
            append(&mut length, output, kind(record.kind));
            append(&mut length, output, b", ");
            append_number(&mut length, output, u64::from(record.block_size));
            append(&mut length, output, b"-byte blocks, ");
            append_number(&mut length, output, record.block_count);
            append(&mut length, output, b" blocks, ");
            append(&mut length, output, state(record.state));
            append(&mut length, output, b"\r\n");
        }
        length
    }
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

fn kind(kind: logos_abi::DeviceKind) -> &'static [u8] {
    match kind {
        logos_abi::DeviceKind::Disk => b"disk",
        logos_abi::DeviceKind::Unknown => b"unknown",
    }
}

fn state(state: logos_abi::DeviceState) -> &'static [u8] {
    match state {
        logos_abi::DeviceState::Ready => b"ready",
        logos_abi::DeviceState::Faulted => b"faulted",
        logos_abi::DeviceState::Absent => b"absent",
    }
}

fn append(length: &mut usize, output: &mut [u8], bytes: &[u8]) {
    let count = bytes.len().min(output.len().saturating_sub(*length));
    output[*length..*length + count].copy_from_slice(&bytes[..count]);
    *length += count;
}

fn append_number(length: &mut usize, output: &mut [u8], mut value: u64) {
    let mut digits = [0; 20];
    let mut count = 0;
    loop {
        digits[digits.len() - 1 - count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    append(length, output, &digits[digits.len() - count..]);
}

fn copy_bounded(bytes: &[u8], output: &mut [u8]) -> usize {
    let count = bytes.len().min(output.len());
    output[..count].copy_from_slice(&bytes[..count]);
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::{DeviceOperation, DeviceRequest, DeviceStatus};

    #[test]
    fn formats_a_bounded_disk_inventory() {
        let request = DeviceRequest::new(DeviceOperation::List, 1);
        let record = DeviceRecord::disk(0, 32, b"disk0").unwrap();
        let response = DeviceResponse::new(request, DeviceStatus::Ok, 1, 1).with_record(record);
        let mut manager = DeviceManager::new();
        manager.publish(response).unwrap();
        let mut output = [0; MAX_DEVICE_LIST_BYTES];
        let length = manager.format_list(&mut output);
        assert_eq!(&output[..length], b"disk0: disk, 4096-byte blocks, 32 blocks, ready\r\n");
    }

    #[test]
    fn empty_inventory_is_explicit() {
        let manager = DeviceManager::new();
        let mut output = [0; 32];
        assert_eq!(manager.format_list(&mut output), 12);
        assert_eq!(&output[..12], b"no devices\r\n");
    }
}
