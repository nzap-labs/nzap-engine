//! The Colab control plane: VM assignment, keep-alive, account and quota,
//! telemetry and credential propagation.

pub mod client;
pub mod quota;
pub mod resources;

pub use client::{AuthType, ColabClient, Propagation};
pub use quota::Quota;
pub use resources::Resources;
