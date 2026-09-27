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

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::Router;
use tokio::sync::oneshot;

mod oauth;
pub mod state;

pub use state::{MockState, RecordedRequest};

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
        let listener = tokio::net::TcpListener::bind(bind)
            .await
            .expect("bind mock Google server");
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
        Self {
            base_url,
            addr,
            state,
            shutdown: Some(tx),
        }
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
    Router::new().merge(oauth::routes()).with_state(state)
}
