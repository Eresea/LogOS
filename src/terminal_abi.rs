//! Compatibility re-export for the shared terminal ABI.
//!
//! New kernel and service code should depend on `logos-abi` directly. This
//! module remains temporarily so the existing host-tested models keep their
//! stable import path while the service split is delivered incrementally.

pub use logos_abi::*;
