pub const SERVICE: crate::platform::services::Service = crate::platform::services::Service::Network;

use logos_abi::{NetworkOperation, NetworkRequest};
use logos_core::capabilities::CapabilityKind;

pub fn capability(request: NetworkRequest) -> Option<(CapabilityKind, u64)> {
    match request.operation {
        NetworkOperation::Bind => Some((CapabilityKind::NetworkBind, request.peer.0)),
        NetworkOperation::SendTo | NetworkOperation::Echo => {
            Some((CapabilityKind::NetworkSend, request.peer.0))
        }
        NetworkOperation::ReceiveFrom => Some((CapabilityKind::NetworkReceive, request.peer.0)),
        NetworkOperation::Status | NetworkOperation::Cancel | NetworkOperation::Close => None,
    }
}

pub fn status_for(request: NetworkRequest, allowed: bool) -> logos_abi::NetworkStatus {
    if !request.valid_shape() {
        logos_abi::NetworkStatus::Invalid
    } else if !allowed {
        logos_abi::NetworkStatus::Denied
    } else {
        logos_abi::NetworkStatus::Complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::{NetworkEndpoint, NetworkProtocol, NetworkScope, PageHandle};

    #[test]
    fn capability_mapping_is_exact() {
        let request = NetworkRequest {
            id: 1,
            operation: NetworkOperation::SendTo,
            endpoint: NetworkEndpoint::new(1, 1).unwrap(),
            peer: NetworkScope::new(NetworkProtocol::Udp, 0xc000_0201, 4001),
            page: PageHandle(1),
            length: 1,
            generation: 1,
            deadline: 1,
        };
        assert_eq!(capability(request), Some((CapabilityKind::NetworkSend, request.peer.0)));
        assert_eq!(status_for(request, false), logos_abi::NetworkStatus::Denied);
    }
}
