//! Google sign-in for the engine.

pub mod loopback;
pub mod manager;
pub mod oauth;

pub use loopback::LOGIN_TIMEOUT;
pub use manager::{AuthManager, AuthSnapshot, DisconnectReason, LoopbackLogin};
pub use oauth::{GoogleUser, OAuthClient};
