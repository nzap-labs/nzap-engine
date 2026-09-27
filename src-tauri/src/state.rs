//! Shared application state and the plumbing every command uses.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use nzap_core::runtime::Terminal;
use nzap_core::session::Emit;
use nzap_core::{Engine, Error, ErrorPayload};
use serde_json::Value;
use tauri::ipc::Channel;
use tokio::task::AbortHandle;

/// What a failed command returns to the webview: `{ code, message, status? }`.
pub type CmdResult<T> = Result<T, ErrorPayload>;

pub struct AppState {
    pub engine: Arc<Engine>,
    /// Running streams by the id the UI chose, for `stream_cancel`.
    streams: Mutex<HashMap<String, AbortHandle>>,
    terminals: Mutex<HashMap<u32, Terminal>>,
    next_terminal: AtomicU32,
    /// The Google sign-in in progress, if any.
    login: Mutex<Option<AbortHandle>>,
    /// Test builds only: write URLs here instead of opening a browser.
    pub open_log: Option<std::path::PathBuf>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl AppState {
    pub fn new(engine: Engine, open_log: Option<std::path::PathBuf>) -> Self {
        Self {
            engine: Arc::new(engine),
            streams: Mutex::new(HashMap::new()),
            terminals: Mutex::new(HashMap::new()),
            next_terminal: AtomicU32::new(1),
            login: Mutex::new(None),
            open_log,
        }
    }

    /// Run `work` on its own task so the UI can cancel it by `stream_id`.
    pub async fn run_stream<T, F>(&self, stream_id: Option<String>, work: F) -> CmdResult<T>
    where
        T: Send + 'static,
        F: Future<Output = nzap_core::Result<T>> + Send + 'static,
    {
        let task = tokio::spawn(work);
        if let Some(id) = &stream_id {
            lock(&self.streams).insert(id.clone(), task.abort_handle());
        }
        let outcome = task.await;
        if let Some(id) = &stream_id {
            lock(&self.streams).remove(id);
        }
        match outcome {
            Ok(result) => result.map_err(ErrorPayload::from),
            Err(error) if error.is_cancelled() => Err(Error::Cancelled.into()),
            Err(error) => Err(Error::internal(format!("The operation crashed: {error}")).into()),
        }
    }

    pub fn cancel_stream(&self, stream_id: &str) -> bool {
        match lock(&self.streams).remove(stream_id) {
            Some(handle) => {
                handle.abort();
                true
            }
            None => false,
        }
    }

    /// Track the sign-in task (a new sign-in replaces the previous one).
    pub fn set_login(&self, handle: Option<AbortHandle>) {
        if let Some(previous) = std::mem::replace(&mut *lock(&self.login), handle) {
            previous.abort();
        }
    }

    pub fn add_terminal(&self, terminal: Terminal) -> u32 {
        let id = self.next_terminal.fetch_add(1, Ordering::Relaxed);
        lock(&self.terminals).insert(id, terminal);
        id
    }

    pub fn with_terminal<R>(&self, id: u32, action: impl FnOnce(&Terminal) -> R) -> Option<R> {
        lock(&self.terminals).get(&id).map(action)
    }

    pub fn remove_terminal(&self, id: u32) -> Option<Terminal> {
        lock(&self.terminals).remove(&id)
    }

    /// Close everything the app holds open (window close / exit).
    pub fn shutdown(&self) {
        for (_, handle) in lock(&self.streams).drain() {
            handle.abort();
        }
        lock(&self.terminals).clear();
        self.set_login(None);
        self.engine.shutdown();
    }
}

/// Forward engine events to a webview channel.
pub fn emit_to(channel: Channel<Value>) -> Emit {
    Arc::new(move |event: Value| {
        // A closed channel means the view went away; the work continues.
        let _ = channel.send(event);
    })
}
