pub mod adapter;
pub mod connection;
pub mod permission;
pub mod translator;

#[cfg(any(test, feature = "fake-acp"))]
pub mod fake_server;

pub use adapter::{AcpAdapter, PermissionHint, default_client_capabilities, sympheo_client_info};
pub use connection::{AcpConnection, MIN_PROTOCOL_VERSION, check_protocol_version};
pub use permission::{
    MatrixOutcome, PermissionDecision, decide_permission, handle_request_permission, matrix,
};
pub use translator::translate;
