pub const NAME: &[u8] = b"storage";
pub const SERVICE: crate::platform::services::Service = crate::platform::services::Service::Storage;

pub const TERMINAL_NAMESPACE: logos_abi::NamespaceId = logos_abi::TERMINAL_NAMESPACE;
pub const TEXT_NAMESPACE: logos_abi::NamespaceId = logos_abi::NamespaceId(2);
pub const AUDIT_NAMESPACE: logos_abi::NamespaceId = logos_abi::NamespaceId(3);
pub const SECRETS_NAMESPACE: logos_abi::NamespaceId = logos_abi::NamespaceId(4);
