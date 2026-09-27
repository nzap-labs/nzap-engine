//! A runtime's Jupyter server: contents, kernels and sessions over HTTP,
//! plus the kernel and terminal WebSockets.

pub mod proxy;

pub use proxy::RuntimeProxy;
