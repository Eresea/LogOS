pub const VIRTIO_GPU_CMD_GET_DISPLAY_INFO: u32 = 0x0100;
pub const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
pub const VIRTIO_GPU_CMD_RESOURCE_UNREF: u32 = 0x0102;
pub const VIRTIO_GPU_CMD_SET_SCANOUT: u32 = 0x0103;
pub const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32 = 0x0104;
pub const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
pub const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;

pub const VIRTIO_GPU_RESP_OK_NODATA: u32 = 0x1100;
pub const VIRTIO_GPU_RESP_ERR_UNSPEC: u32 = 0x1200;
pub const VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM: u32 = 2;
pub const VIRTIO_GPU_FORMAT_X8B8G8R8_UNORM: u32 = 68;
pub const VIRTIO_GPU_FLAG_FENCE: u32 = 1;
pub const VIRTIO_GPU_CTRL_HEADER_BYTES: usize = 24;
pub const VIRTIO_GPU_MAX_BACKING_BYTES: u64 = 4 * 1024 * 1024;
pub const VIRTIO_GPU_MAX_COMMAND_BYTES: usize = VIRTIO_GPU_CTRL_HEADER_BYTES + 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtioGpuRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl VirtioGpuRect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        if x.checked_add(width).is_none() || y.checked_add(height).is_none() {
            return None;
        }
        Some(Self { x, y, width, height })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtioGpuCommand {
    ResourceCreate2d { resource_id: u32, format: u32, width: u32, height: u32 },
    ResourceAttachBacking { resource_id: u32, address: u64, length: u32 },
    SetScanout { scanout_id: u32, resource_id: u32, rect: VirtioGpuRect },
    TransferToHost2d { resource_id: u32, rect: VirtioGpuRect },
    ResourceFlush { resource_id: u32, rect: VirtioGpuRect },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtioGpuEncodeError {
    InvalidResource,
    InvalidDimensions,
    InvalidBacking,
    BufferTooSmall,
}

impl VirtioGpuCommand {
    pub const fn encoded_len(self) -> usize {
        match self {
            Self::ResourceCreate2d { .. } => 40,
            Self::ResourceAttachBacking { .. } => 56,
            Self::SetScanout { .. } => 56,
            Self::TransferToHost2d { .. } => 56,
            Self::ResourceFlush { .. } => 48,
        }
    }

    pub fn encode(
        self,
        fence_id: u64,
        buffer: &mut [u8; VIRTIO_GPU_MAX_COMMAND_BYTES],
    ) -> Result<usize, VirtioGpuEncodeError> {
        let length = self.encoded_len();
        if buffer.len() < length {
            return Err(VirtioGpuEncodeError::BufferTooSmall);
        }
        buffer[..length].fill(0);
        let kind = match self {
            Self::ResourceCreate2d { resource_id, format, width, height } => {
                if resource_id == 0 || width == 0 || height == 0 {
                    return Err(if resource_id == 0 {
                        VirtioGpuEncodeError::InvalidResource
                    } else {
                        VirtioGpuEncodeError::InvalidDimensions
                    });
                }
                put_u32(buffer, 24, resource_id);
                put_u32(buffer, 28, format);
                put_u32(buffer, 32, width);
                put_u32(buffer, 36, height);
                VIRTIO_GPU_CMD_RESOURCE_CREATE_2D
            }
            Self::ResourceAttachBacking { resource_id, address, length } => {
                if resource_id == 0 {
                    return Err(VirtioGpuEncodeError::InvalidResource);
                }
                if address == 0 || address % 4096 != 0 || length == 0 {
                    return Err(VirtioGpuEncodeError::InvalidBacking);
                }
                if u64::from(length) > VIRTIO_GPU_MAX_BACKING_BYTES {
                    return Err(VirtioGpuEncodeError::InvalidBacking);
                }
                put_u32(buffer, 24, resource_id);
                put_u32(buffer, 28, 1);
                put_u64(buffer, 40, address);
                put_u32(buffer, 48, length);
                VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING
            }
            Self::SetScanout { scanout_id, resource_id, rect } => {
                if resource_id == 0 {
                    return Err(VirtioGpuEncodeError::InvalidResource);
                }
                put_rect(buffer, 24, rect)?;
                put_u32(buffer, 40, scanout_id);
                put_u32(buffer, 44, resource_id);
                VIRTIO_GPU_CMD_SET_SCANOUT
            }
            Self::TransferToHost2d { resource_id, rect } => {
                if resource_id == 0 {
                    return Err(VirtioGpuEncodeError::InvalidResource);
                }
                put_rect(buffer, 24, rect)?;
                put_u64(buffer, 40, 0);
                put_u32(buffer, 48, resource_id);
                VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D
            }
            Self::ResourceFlush { resource_id, rect } => {
                if resource_id == 0 {
                    return Err(VirtioGpuEncodeError::InvalidResource);
                }
                put_rect(buffer, 24, rect)?;
                put_u32(buffer, 40, resource_id);
                VIRTIO_GPU_CMD_RESOURCE_FLUSH
            }
        };
        put_u32(buffer, 0, kind);
        put_u32(buffer, 4, VIRTIO_GPU_FLAG_FENCE);
        put_u64(buffer, 8, fence_id);
        Ok(length)
    }
}

pub const fn response_is_ok(response_type: u32) -> bool {
    response_type == VIRTIO_GPU_RESP_OK_NODATA
}

fn put_rect(
    buffer: &mut [u8; VIRTIO_GPU_MAX_COMMAND_BYTES],
    offset: usize,
    rect: VirtioGpuRect,
) -> Result<(), VirtioGpuEncodeError> {
    if rect.width == 0 || rect.height == 0 {
        return Err(VirtioGpuEncodeError::InvalidDimensions);
    }
    put_u32(buffer, offset, rect.x);
    put_u32(buffer, offset + 4, rect.y);
    put_u32(buffer, offset + 8, rect.width);
    put_u32(buffer, offset + 12, rect.height);
    Ok(())
}

fn put_u32(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(buffer: &mut [u8], offset: usize, value: u64) {
    buffer[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_transfer_and_flush_with_fences() {
        let rect = VirtioGpuRect::new(8, 16, 32, 64).unwrap();
        let mut buffer = [0; VIRTIO_GPU_MAX_COMMAND_BYTES];
        let length = VirtioGpuCommand::TransferToHost2d { resource_id: 7, rect }
            .encode(9, &mut buffer)
            .unwrap();
        assert_eq!(length, 56);
        assert_eq!(u32::from_le_bytes(buffer[0..4].try_into().unwrap()), 0x0105);
        assert_eq!(u32::from_le_bytes(buffer[4..8].try_into().unwrap()), VIRTIO_GPU_FLAG_FENCE);
        assert_eq!(u64::from_le_bytes(buffer[8..16].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(buffer[24..28].try_into().unwrap()), 8);
        assert_eq!(u32::from_le_bytes(buffer[28..32].try_into().unwrap()), 16);
        assert_eq!(u32::from_le_bytes(buffer[48..52].try_into().unwrap()), 7);

        let length = VirtioGpuCommand::ResourceFlush { resource_id: 7, rect }
            .encode(10, &mut buffer)
            .unwrap();
        assert_eq!(length, 48);
        assert_eq!(u32::from_le_bytes(buffer[0..4].try_into().unwrap()), 0x0104);
        assert_eq!(u32::from_le_bytes(buffer[40..44].try_into().unwrap()), 7);
    }

    #[test]
    fn attach_backing_requires_page_aligned_bounded_memory() {
        let mut buffer = [0; VIRTIO_GPU_MAX_COMMAND_BYTES];
        let command = VirtioGpuCommand::ResourceAttachBacking {
            resource_id: 1,
            address: 0x1001,
            length: 4096,
        };
        assert_eq!(command.encode(1, &mut buffer), Err(VirtioGpuEncodeError::InvalidBacking));
        let command = VirtioGpuCommand::ResourceAttachBacking {
            resource_id: 1,
            address: 0x1000,
            length: VIRTIO_GPU_MAX_BACKING_BYTES as u32 + 1,
        };
        assert_eq!(command.encode(1, &mut buffer), Err(VirtioGpuEncodeError::InvalidBacking));
    }

    #[test]
    fn rejects_invalid_resources_and_rectangles() {
        let mut buffer = [0; VIRTIO_GPU_MAX_COMMAND_BYTES];
        assert_eq!(
            VirtioGpuCommand::ResourceCreate2d {
                resource_id: 0,
                format: VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM,
                width: 1,
                height: 1,
            }
            .encode(1, &mut buffer),
            Err(VirtioGpuEncodeError::InvalidResource)
        );
        assert_eq!(VirtioGpuRect::new(0, 0, 0, 1), None);
        assert!(!response_is_ok(VIRTIO_GPU_RESP_ERR_UNSPEC));
    }
}
