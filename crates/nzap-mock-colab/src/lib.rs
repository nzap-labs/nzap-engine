//! Mock Google services for NZAP Engine tests.
//!
//! Stands in for everything the engine talks to — Google OAuth, the Colab
//! control plane (`/tun/m/*`, `v1/*`), a runtime's Jupyter server, the kernel
//! WebSocket and `/colab/tty` — so integration and desktop E2E tests exercise
//! the real client code without a Google account. Every service is served
//! from one base URL (`Endpoints::single_host` in `nzap-core`).
//!
//! The mock is deliberately strict where Google is: PKCE is verified, codes
//! are single-use, and bearer tokens are checked on every protected route.

#![allow(clippy::result_large_err)] // test infrastructure: axum responses as errors

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use tokio::sync::oneshot;

mod control;
mod kernel;
mod oauth;
mod runtime;
pub mod state;
mod tty;

pub use state::{MockAssignment, MockFile, MockRuntime, MockState, RecordedRequest};

/// Shared mock state, handed to every route.
pub type Shared = Arc<Mutex<MockState>>;

/// A running mock server. Dropping it shuts the server down.
pub struct MockGoogle {
    pub base_url: String,
    pub addr: SocketAddr,
    state: Shared,
    shutdown: Option<oneshot::Sender<()>>,
}

impl MockGoogle {
    /// Start on an ephemeral loopback port.
    pub async fn start() -> Self {
        Self::start_on("127.0.0.1:0").await
    }

    /// Start on a specific address (the `mock-colab` binary for desktop E2E).
    pub async fn start_on(bind: &str) -> Self {
        let listener = tokio::net::TcpListener::bind(bind).await.expect("bind mock Google server");
        let addr = listener.local_addr().expect("mock server address");
        let base_url = format!("http://{addr}");
        let state: Shared = Arc::new(Mutex::new(MockState::new(&base_url)));
        let app = router(state.clone());
        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = rx.await;
            });
            if let Err(error) = server.await {
                eprintln!("mock Google server stopped: {error}");
            }
        });
        Self { base_url, addr, state, shutdown: Some(tx) }
    }

    /// Lock the mock state to inspect or tweak it between requests.
    pub fn state(&self) -> MutexGuard<'_, MockState> {
        self.state.lock().expect("mock state lock")
    }

    pub fn shared(&self) -> Shared {
        self.state.clone()
    }
}

impl Drop for MockGoogle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

fn router(state: Shared) -> Router {
    Router::new()
        .merge(oauth::routes())
        .merge(control::routes())
        .merge(runtime::routes())
        .merge(kernel::routes())
        .merge(tty::routes())
        .layer(middleware::from_fn_with_state(state.clone(), record))
        .with_state(state)
}

/// Remember every request so tests can assert headers and parameters.
async fn record(State(state): State<Shared>, request: Request, next: Next) -> Response {
    let query = request
        .uri()
        .query()
        .map(|query| {
            query
                .split('&')
                .filter_map(|pair| {
                    let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                    Some((decode(key)?, decode(value)?))
                })
                .collect()
        })
        .unwrap_or_default();
    let headers = request
        .headers()
        .iter()
        .map(|(name, value)| {
            (name.as_str().to_owned(), value.to_str().unwrap_or_default().to_owned())
        })
        .collect();
    let recorded = RecordedRequest {
        method: request.method().to_string(),
        path: request.uri().path().to_owned(),
        query,
        headers,
    };
    state.lock().expect("mock state").requests.push(recorded);
    next.run(request).await
}

/// `application/x-www-form-urlencoded` component decoding.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                index += 2;
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8(out).ok()
}
