//! Bounded, fail-closed Network boot profile parsing.

use logos_abi::{
    NETWORK_DHCP_DEADLINE_TICKS, NETWORK_GATEWAY_ARP_DEADLINE_TICKS, NetworkConfig, NetworkProfile,
};

pub const MAX_CONFIG_BYTES: usize = 512;

pub fn parse(bytes: &[u8]) -> Option<NetworkConfig> {
    if bytes.is_empty() || bytes.len() > MAX_CONFIG_BYTES {
        return None;
    }
    let mut config = NetworkConfig::disabled();
    let mut saw_profile = false;
    let mut saw_address = false;
    let mut saw_netmask = false;
    let mut saw_gateway = false;
    let mut saw_gateway_deadline = false;
    let mut saw_dhcp_deadline = false;
    for line in bytes.split(|byte| *byte == b'\n' || *byte == b'\r') {
        let line = trim(line);
        if line.is_empty() {
            continue;
        }
        let separator = line.iter().position(|byte| *byte == b'=')?;
        let key = trim(&line[..separator]);
        let value = trim(&line[separator + 1..]);
        if key.is_empty() || value.is_empty() {
            return None;
        }
        match key {
            b"profile" if !saw_profile => {
                config.profile = match value {
                    b"disabled" => NetworkProfile::Disabled,
                    b"static_then_dhcp" | b"static-then-dhcp" => NetworkProfile::StaticThenDhcp,
                    _ => return None,
                };
                saw_profile = true;
            }
            b"address" if !saw_address => {
                config.address = parse_ipv4(value)?;
                saw_address = true;
            }
            b"netmask" if !saw_netmask => {
                config.netmask = parse_ipv4(value)?;
                saw_netmask = true;
            }
            b"gateway" if !saw_gateway => {
                config.gateway = parse_ipv4(value)?;
                saw_gateway = true;
            }
            b"gateway_deadline_ticks" if !saw_gateway_deadline => {
                config.gateway_deadline_ticks = parse_u32(value)?;
                saw_gateway_deadline = true;
            }
            b"dhcp_deadline_ticks" if !saw_dhcp_deadline => {
                config.dhcp_deadline_ticks = parse_u32(value)?;
                saw_dhcp_deadline = true;
            }
            _ => return None,
        }
    }
    if !saw_profile {
        return None;
    }
    if config.profile == NetworkProfile::Disabled {
        if saw_address || saw_netmask || saw_gateway {
            return None;
        }
        config.address = [0; 4];
        config.netmask = [0; 4];
        config.gateway = [0; 4];
        config.gateway_deadline_ticks = NETWORK_GATEWAY_ARP_DEADLINE_TICKS;
        config.dhcp_deadline_ticks = NETWORK_DHCP_DEADLINE_TICKS;
    }
    config.is_valid().then_some(config)
}

fn trim(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = bytes.len();
    while start < end && (bytes[start] == b' ' || bytes[start] == b'\t') {
        start += 1;
    }
    while end > start && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t') {
        end -= 1;
    }
    &bytes[start..end]
}

fn parse_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    let mut value = 0u32;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
    }
    (value != 0).then_some(value)
}

fn parse_ipv4(bytes: &[u8]) -> Option<[u8; 4]> {
    let mut address = [0; 4];
    let mut part = 0;
    let mut value = 0u16;
    let mut digits = 0;
    for byte in bytes.iter().copied().chain(core::iter::once(b'.')) {
        if byte.is_ascii_digit() {
            value = value.checked_mul(10)?.checked_add(u16::from(byte - b'0'))?;
            if value > 255 {
                return None;
            }
            digits += 1;
        } else if byte == b'.' && digits != 0 && part < 4 {
            address[part] = value as u8;
            part += 1;
            value = 0;
            digits = 0;
        } else {
            return None;
        }
    }
    (part == 4).then_some(address)
}

#[cfg(target_os = "uefi")]
pub fn load_from_esp() -> NetworkConfig {
    use uefi::proto::media::file::{File, FileAttribute, FileMode};
    use uefi::{CStr16, boot};

    let Ok(mut filesystem) = boot::get_image_file_system(boot::image_handle()) else {
        return NetworkConfig::disabled();
    };
    let Ok(mut root) = filesystem.open_volume() else {
        return NetworkConfig::disabled();
    };
    let mut path = [0u16; 32];
    let name = b"\\EFI\\LOGOS\\NETWORK.CFG";
    for (index, byte) in name.iter().enumerate() {
        path[index] = *byte as u16;
    }
    let Ok(path) = CStr16::from_u16_with_nul(&path[..=name.len()]) else {
        return NetworkConfig::disabled();
    };
    let Ok(file) = root.open(path, FileMode::Read, FileAttribute::empty()) else {
        return NetworkConfig::disabled();
    };
    let Some(mut file) = file.into_regular_file() else {
        return NetworkConfig::disabled();
    };
    let mut bytes = [0; MAX_CONFIG_BYTES + 1];
    let Ok(length) = file.read(&mut bytes) else {
        return NetworkConfig::disabled();
    };
    parse(&bytes[..length]).unwrap_or_else(NetworkConfig::disabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_malformed_profiles_fail_closed() {
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(b"profile=wat"), None);
        assert_eq!(parse(b"profile=static_then_dhcp\naddress=10.0.2.15"), None);
        assert_eq!(parse(b"profile=disabled\naddress=10.0.2.15"), None);
    }

    #[test]
    fn static_profile_is_bounded_and_validated() {
        let config = parse(
            b"profile=static_then_dhcp\naddress=10.0.2.15\nnetmask=255.255.255.0\ngateway=10.0.2.2\n",
        )
        .unwrap();
        assert_eq!(config.profile, NetworkProfile::StaticThenDhcp);
        assert_eq!(config.address, [10, 0, 2, 15]);
        assert!(config.is_enabled());
    }

    #[test]
    fn static_profile_rejects_noncontiguous_masks_and_duplicate_deadlines() {
        assert_eq!(
            parse(
                b"profile=static_then_dhcp\naddress=10.0.2.15\nnetmask=255.0.255.0\ngateway=10.0.2.2\n"
            ),
            None
        );
        assert_eq!(
            parse(b"profile=disabled\ngateway_deadline_ticks=1\ngateway_deadline_ticks=2\n"),
            None
        );
    }

    #[test]
    fn oversized_profiles_fail_closed() {
        let mut bytes = [b' '; MAX_CONFIG_BYTES + 1];
        bytes[..b"profile=disabled".len()].copy_from_slice(b"profile=disabled");
        assert_eq!(parse(&bytes), None);
    }

    #[test]
    fn disabled_profile_is_explicit_and_deterministic() {
        let config = parse(b"profile=disabled\n").unwrap();
        assert_eq!(config, NetworkConfig::disabled());
        assert!(!config.is_enabled());
    }
}
