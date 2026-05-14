pub mod adapter;
pub mod connection;
pub mod permission;
pub mod translator;

#[cfg(any(test, feature = "fake-acp"))]
pub mod fake_server;

pub use adapter::{
    default_client_capabilities, sympheo_client_info, AcpAdapter, PermissionHint,
};
pub use connection::{check_protocol_version, AcpConnection, MIN_PROTOCOL_VERSION};
pub use permission::{
    decide_permission, handle_request_permission, matrix, MatrixOutcome, PermissionDecision,
};
pub use translator::translate;
