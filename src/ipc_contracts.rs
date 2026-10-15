//! Pure IPC contract policy shared by the service runtime.
//!
//! Contracts listed here are served by Core itself: the Core receives the
//! sender's staged message, validates and handles it, and queues any response,
//! instead of forwarding raw bytes between two services.

use logos_abi::ServiceId;

/// `(producer, consumer, contract id)` triples whose endpoints require Core
/// dispatch. `None` means "the Core side of the endpoint".
pub(crate) const CORE_DISPATCH_CONTRACTS: [(Option<ServiceId>, Option<ServiceId>, u16); 10] = [
    (Some(ServiceId::Storage), None, logos_abi::IPC_CONTRACT_STORAGE_REQUEST),
    (None, Some(ServiceId::Storage), logos_abi::IPC_CONTRACT_STORAGE_RESPONSE),
    (Some(ServiceId::Network), None, logos_abi::IPC_CONTRACT_PACKET),
    (None, Some(ServiceId::Network), logos_abi::IPC_CONTRACT_PACKET),
    (Some(ServiceId::Device), None, logos_abi::IPC_CONTRACT_DEVICE_REQUEST),
    (None, Some(ServiceId::Device), logos_abi::IPC_CONTRACT_DEVICE_RESPONSE),
    (None, Some(ServiceId::Storage), logos_abi::IPC_CONTRACT_PACKAGE_REQUEST),
    (Some(ServiceId::Storage), None, logos_abi::IPC_CONTRACT_PACKAGE_RESPONSE),
    (Some(ServiceId::Storage), None, logos_abi::IPC_CONTRACT_STORAGE_MAP_REQUEST),
    (None, Some(ServiceId::Storage), logos_abi::IPC_CONTRACT_STORAGE_MAP_RESPONSE),
];

/// Whether an endpoint with this producer, consumer and contract id is a
/// Core-dispatched contract.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn requires_core_dispatch(
    producer: Option<ServiceId>,
    consumer: Option<ServiceId>,
    contract: u16,
) -> bool {
    CORE_DISPATCH_CONTRACTS.contains(&(producer, consumer, contract))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_no_duplicates() {
        for (index, entry) in CORE_DISPATCH_CONTRACTS.iter().enumerate() {
            assert!(!CORE_DISPATCH_CONTRACTS[index + 1..].contains(entry), "{entry:?}");
        }
    }

    #[test]
    fn every_entry_has_exactly_one_core_side() {
        for (producer, consumer, _) in CORE_DISPATCH_CONTRACTS {
            assert!(producer.is_some() != consumer.is_some());
        }
    }

    #[test]
    fn known_contracts_match_and_others_do_not() {
        assert!(requires_core_dispatch(
            Some(ServiceId::Storage),
            None,
            logos_abi::IPC_CONTRACT_STORAGE_REQUEST
        ));
        assert!(requires_core_dispatch(
            None,
            Some(ServiceId::Network),
            logos_abi::IPC_CONTRACT_PACKET
        ));
        // Same contract id, wrong direction.
        assert!(!requires_core_dispatch(
            None,
            Some(ServiceId::Storage),
            logos_abi::IPC_CONTRACT_STORAGE_REQUEST
        ));
        // Plain byte pipes between services are never Core-dispatched.
        assert!(!requires_core_dispatch(
            Some(ServiceId::Flow),
            Some(ServiceId::Network),
            logos_abi::IPC_CONTRACT_BYTES
        ));
        assert!(!requires_core_dispatch(None, None, logos_abi::IPC_CONTRACT_PACKET));
    }
}
