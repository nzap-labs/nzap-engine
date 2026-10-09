//! Runtime lifecycle: assign → connect → execute → keep alive → release.
//!
//! Port of colab-studio's `SessionManager` (which mirrors colab-cli's
//! `SessionState` store and colab-vscode's assignment registry). Runtimes
//! are persisted so the app reconnects to still-running VMs after a restart,
//! while `/tun/m/assignments` stays the source of truth for what the account
//! actually holds.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::state::{
    scalar_text, unix_now, validate_name, AssignmentView, SessionState, SessionView,
};
use crate::colab::client::validate_endpoint;
use crate::colab::resources::{self, Resources};
use crate::colab::{AuthType, ColabClient};
use crate::config::{
    hardware_label, shape_display_label, RuntimeRequest, KEEP_ALIVE_INTERVAL, KEEP_ALIVE_MAX,
};
use crate::error::{Error, Result};
use crate::history::{collect_outputs, HistoryLog};
use crate::runtime::{ColabRequest, KernelChannel, RuntimeProxy, Terminal, TerminalSink};
use crate::secrets::write_private;

/// Where streamed events go (a Tauri channel in the app, a Vec in tests).
pub type Emit = Arc<dyn Fn(Value) + Send + Sync>;

pub const DEFAULT_EXECUTE_TIMEOUT: Duration = Duration::from_secs(3600);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A cell paused on the VM's credential request until the user consents.
struct PendingColab {
    channel: Weak<KernelChannel>,
    request: ColabRequest,
    auth_type: AuthType,
}

struct Inner {
    client: Arc<ColabClient>,
    history: Arc<HistoryLog>,
    state_path: PathBuf,
    sessions: Mutex<BTreeMap<String, SessionState>>,
    channels: Mutex<HashMap<String, Arc<KernelChannel>>>,
    connect_lock: tokio::sync::Mutex<()>,
    notifiers: Mutex<HashMap<String, Vec<mpsc::UnboundedSender<Value>>>>,
    keepalive_tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    keepalive_enabled: AtomicBool,
    keepalive_interval: Mutex<Duration>,
    pending_colab: Mutex<HashMap<String, PendingColab>>,
    /// Serializes writes of the sessions file.
    save_lock: Mutex<()>,
}

/// One entry of a directory listing.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    /// `directory` | `file` | `notebook`
    #[serde(rename = "type")]
    pub kind: String,
    pub size: Option<u64>,
    pub last_modified: Option<String>,
    pub mimetype: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileListing {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub entries: Vec<FileEntry>,
}

/// Outcome of releasing a runtime.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopOutcome {
    /// Colab confirmed the VM was released.
    pub released: bool,
    /// Set when the local entry is gone but Colab did not confirm.
    pub warning: Option<String>,
}

/// Logs an execution when it ends — including when the stream is dropped
/// mid-way (cancelled), which is recorded as `interrupted`.
struct Recorder {
    history: Arc<HistoryLog>,
    name: String,
    code: String,
    seen: Vec<Value>,
    record: bool,
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if !self.record {
            return;
        }
        let reply = self
            .seen
            .iter()
            .rev()
            .find(|event| event.get("type").and_then(Value::as_str) == Some("execute_reply"));
        let status = reply
            .and_then(|reply| reply.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("interrupted");
        self.history.log(
            &self.name,
            "execution",
            json!({
                "code": self.code,
                "outputs": collect_outputs(&self.seen),
                "status": status,
                "execution_count": reply.and_then(|reply| reply.get("execution_count")).cloned(),
            }),
        );
    }
}

fn error_name(error: &Error) -> &'static str {
    match error {
        Error::Runtime { .. } => "RuntimeError",
        Error::Network(_) => "ConnectionError",
        Error::Cancelled => "Cancelled",
        Error::AuthExpired(_) | Error::NotConnected => "AuthError",
        _ => "Error",
    }
}

#[derive(Clone)]
pub struct SessionManager {
    inner: Arc<Inner>,
}

impl SessionManager {
    pub fn new(client: Arc<ColabClient>, history: Arc<HistoryLog>, state_path: PathBuf) -> Self {
        let sessions = std::fs::read_to_string(&state_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|blob| blob.get("sessions").cloned())
            .and_then(|list| serde_json::from_value::<Vec<SessionState>>(list).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|state| (state.name.clone(), state))
            .collect();
        Self {
            inner: Arc::new(Inner {
                client,
                history,
                state_path,
                sessions: Mutex::new(sessions),
                channels: Mutex::new(HashMap::new()),
                connect_lock: tokio::sync::Mutex::new(()),
                notifiers: Mutex::new(HashMap::new()),
                keepalive_tasks: Mutex::new(HashMap::new()),
                keepalive_enabled: AtomicBool::new(true),
                keepalive_interval: Mutex::new(KEEP_ALIVE_INTERVAL),
                pending_colab: Mutex::new(HashMap::new()),
                save_lock: Mutex::new(()),
            }),
        }
    }

    pub fn client(&self) -> &Arc<ColabClient> {
        &self.inner.client
    }

    pub fn history(&self) -> &Arc<HistoryLog> {
        &self.inner.history
    }

    /// Configure the background keep-alive (Settings).
    pub fn set_keepalive(&self, enabled: bool, interval: Duration) {
        self.inner.keepalive_enabled.store(enabled, Ordering::SeqCst);
        *lock(&self.inner.keepalive_interval) = interval.max(Duration::from_millis(10));
        if !enabled {
            for (_, task) in lock(&self.inner.keepalive_tasks).drain() {
                task.abort();
            }
        }
    }

    fn colab_host(&self) -> String {
        self.inner.client.auth().endpoints().colab.clone()
    }

    fn http(&self) -> reqwest::Client {
        self.inner.client.auth().http().clone()
    }

    // ---------------------------------------------------------- persistence

    fn save(&self) {
        // Snapshot and write under one lock, so the newest state always lands last.
        let _saving = lock(&self.inner.save_lock);
        let payload = {
            let sessions = lock(&self.inner.sessions);
            json!({ "sessions": sessions.values().collect::<Vec<_>>() })
        };
        let result = serde_json::to_vec_pretty(&payload)
            .map_err(Error::from)
            .and_then(|bytes| write_private(&self.inner.state_path, &bytes));
        if let Err(error) = result {
            tracing::warn!("Could not save runtimes: {error}");
        }
    }

    fn update<R>(&self, name: &str, change: impl FnOnce(&mut SessionState) -> R) -> Result<R> {
        let result = {
            let mut sessions = lock(&self.inner.sessions);
            let state = sessions.get_mut(name).ok_or_else(|| missing(name))?;
            change(state)
        };
        self.save();
        Ok(result)
    }

    /// Record activity without a disk write (it happens on every cell).
    fn touch(&self, name: &str) {
        if let Some(state) = lock(&self.inner.sessions).get_mut(name) {
            state.last_activity = unix_now();
        }
    }

    // --------------------------------------------------------------- lookups

    pub fn names(&self) -> Vec<String> {
        lock(&self.inner.sessions).keys().cloned().collect()
    }

    pub fn get(&self, name: &str) -> Result<SessionState> {
        lock(&self.inner.sessions).get(name).cloned().ok_or_else(|| missing(name))
    }

    pub fn view(&self, name: &str) -> Result<SessionView> {
        let state = self.get(name)?;
        let channel = lock(&self.inner.channels).get(name).cloned();
        let connected = channel.as_ref().is_some_and(|channel| channel.is_connected());
        let kernel_state = channel.and_then(|channel| channel.execution_state());
        Ok(state.view(&self.colab_host(), connected, kernel_state))
    }

    pub fn views(&self) -> Vec<SessionView> {
        self.names().iter().filter_map(|name| self.view(name).ok()).collect()
    }

    pub fn proxy(&self, name: &str) -> Result<RuntimeProxy> {
        let state = self.get(name)?;
        if state.url.is_empty() {
            return Err(Error::runtime(None, "This runtime has no proxy address."));
        }
        Ok(RuntimeProxy::new(self.http(), &state.url, &state.token))
    }

    // ---------------------------------------------------------------- create

    fn register(&self, state: SessionState, how: &str) {
        let name = state.name.clone();
        let event = json!({
            "endpoint": state.endpoint,
            "accelerator": state.hardware(),
            "shape": shape_display_label(&state.machine_shape),
            "how": how,
        });
        lock(&self.inner.sessions).insert(name.clone(), state);
        self.save();
        self.inner.history.log(&name, "session_created", event);
        self.start_keepalive(&name);
    }

    /// Allocate a VM and track it as a runtime.
    pub async fn create(&self, mut request: RuntimeRequest) -> Result<SessionView> {
        if request.name.trim().is_empty() {
            request.name = format!("session-{}", unix_now() as u64);
        }
        let name = validate_name(request.name.trim())?.to_owned();
        if lock(&self.inner.sessions).contains_key(&name) {
            return Err(Error::invalid(format!("A runtime named '{name}' already exists.")));
        }
        tracing::info!("Requesting a {} runtime", request.label());
        let (_, accelerator, _) = request.resolved();
        let assignment = self.inner.client.assign(&request, None).await?;
        let state = SessionState::from_assignment(&name, &assignment, accelerator.as_str());
        if state.endpoint.is_empty() {
            return Err(Error::Colab {
                status: None,
                message: "Colab did not return a runtime endpoint.".to_owned(),
                body: String::new(),
            });
        }
        self.register(state, "assigned");
        self.view(&name)
    }

    /// Create, then open the kernel socket (best-effort: the VM exists even
    /// if its kernel lags). Returns the view and whether it connected.
    pub async fn create_and_connect(&self, request: RuntimeRequest) -> Result<(SessionView, bool)> {
        let view = self.create(request).await?;
        let connected = match self.channel(&view.name).await {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!("Kernel connect failed for {}: {error}", view.name);
                false
            }
        };
        Ok((self.view(&view.name)?, connected))
    }

    /// Everything the account holds, including VMs created elsewhere.
    pub async fn server_assignments(&self) -> Result<Vec<AssignmentView>> {
        let local: HashSet<String> =
            lock(&self.inner.sessions).values().map(|state| state.endpoint.clone()).collect();
        let assignments = self.inner.client.list_assignments().await?;
        Ok(assignments
            .iter()
            .map(|assignment| {
                let endpoint = scalar_text(assignment.get("endpoint"), "");
                AssignmentView {
                    managed: local.contains(&endpoint),
                    endpoint,
                    accelerator: hardware_label(&scalar_text(
                        assignment.get("accelerator"),
                        "NONE",
                    )),
                    variant: scalar_text(assignment.get("variant"), "0"),
                    shape: shape_display_label(&scalar_text(assignment.get("machineShape"), "0"))
                        .to_owned(),
                }
            })
            .collect())
    }

    /// Track a VM the app did not create. One endpoint may only be managed
    /// under a single name — otherwise releasing one entry would orphan the
    /// other.
    pub async fn adopt(&self, endpoint: &str, name: Option<&str>) -> Result<SessionView> {
        let endpoint = validate_endpoint(endpoint)?;
        let name = match name.map(str::trim).filter(|name| !name.is_empty()) {
            Some(name) => name.to_owned(),
            None => format!("imported-{}", &endpoint[..endpoint.len().min(8)]),
        };
        let name = validate_name(&name)?.to_owned();
        {
            let sessions = lock(&self.inner.sessions);
            if sessions.contains_key(&name) {
                return Err(Error::invalid(format!("A runtime named '{name}' already exists.")));
            }
            if let Some(existing) = sessions.values().find(|state| state.endpoint == endpoint) {
                return Err(Error::invalid(format!(
                    "That runtime is already managed as '{}'.",
                    existing.name
                )));
            }
        }
        let assignment = self
            .inner
            .client
            .list_assignments()
            .await?
            .into_iter()
            .find(|assignment| assignment.get("endpoint").and_then(Value::as_str) == Some(endpoint))
            .ok_or_else(|| Error::not_found("Assignment not found."))?;
        self.register(SessionState::from_assignment(&name, &assignment, "NONE"), "adopted");
        self.view(&name)
    }

    /// After a restart: forget runtimes whose VM is gone and resume the
    /// keep-alive for the rest.
    pub async fn resume(&self) -> Result<()> {
        let names = self.names();
        if names.is_empty() {
            return Ok(());
        }
        let live: HashSet<String> = self
            .inner
            .client
            .list_assignments()
            .await?
            .iter()
            .filter_map(|assignment| assignment.get("endpoint").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        for name in names {
            let Ok(state) = self.get(&name) else { continue };
            if live.contains(&state.endpoint) {
                self.start_keepalive(&name);
            } else {
                lock(&self.inner.sessions).remove(&name);
                self.inner.history.log(&name, "session_terminated", json!({"reason": "expired"}));
            }
        }
        self.save();
        Ok(())
    }

    // --------------------------------------------------------------- kernel

    /// The live kernel socket for a runtime, starting a kernel if needed.
    pub async fn channel(&self, name: &str) -> Result<Arc<KernelChannel>> {
        if let Some(channel) = self.connected_channel(name) {
            return Ok(channel);
        }
        let _guard = self.inner.connect_lock.lock().await;
        if let Some(channel) = self.connected_channel(name) {
            return Ok(channel);
        }
        if self.get(name)?.kernel_id.is_none() {
            self.start_kernel(name, "python3").await?;
        }
        let channel = match self.open_channel(name).await {
            // The kernel died with its VM process: start a fresh one once.
            Err(Error::Runtime { status: Some(404), .. }) => {
                self.start_kernel(name, "python3").await?;
                self.open_channel(name).await?
            }
            other => other?,
        };
        if let Some(old) = lock(&self.inner.channels).insert(name.to_owned(), channel.clone()) {
            old.stop();
        }
        let session_id = channel.session_id().to_owned();
        self.update(name, |state| state.session_id = Some(session_id))?;
        Ok(channel)
    }

    fn connected_channel(&self, name: &str) -> Option<Arc<KernelChannel>> {
        lock(&self.inner.channels).get(name).filter(|channel| channel.is_connected()).cloned()
    }

    async fn open_channel(&self, name: &str) -> Result<Arc<KernelChannel>> {
        let state = self.get(name)?;
        let kernel_id =
            state.kernel_id.clone().ok_or_else(|| Error::runtime(None, "No kernel."))?;
        let proxy = self.proxy(name)?;
        let (requests_tx, mut requests_rx) = mpsc::unbounded_channel::<ColabRequest>();
        let channel =
            KernelChannel::connect(&proxy, &kernel_id, state.session_id.clone(), Some(requests_tx))
                .await?;

        let manager = self.clone();
        let owner = name.to_owned();
        let weak = Arc::downgrade(&channel);
        tokio::spawn(async move {
            while let Some(request) = requests_rx.recv().await {
                let Some(channel) = weak.upgrade() else { break };
                manager.handle_colab_request(&owner, channel, request).await;
            }
        });
        tracing::info!("Kernel socket connected for {name}");
        Ok(channel)
    }

    /// Open the socket and ask for `kernel_info` (the Connect button).
    pub async fn connect(&self, name: &str) -> Result<Value> {
        let channel = self.channel(name).await?;
        let info = match channel.kernel_info(Duration::from_secs(30)).await {
            Ok(info) => info,
            Err(error) => {
                tracing::info!("kernel_info failed: {error}");
                json!({})
            }
        };
        Ok(json!({
            "connected": channel.is_connected(),
            "kernelId": channel.kernel_id(),
            "kernelInfo": info,
        }))
    }

    pub fn disconnect(&self, name: &str) -> Result<()> {
        self.get(name)?;
        if let Some(channel) = lock(&self.inner.channels).remove(name) {
            channel.stop();
        }
        Ok(())
    }

    /// Start a kernel and bind a named notebook session to it, so the runtime
    /// shows up with its name on Colab's "Manage sessions" page.
    pub async fn start_kernel(&self, name: &str, kernel_name: &str) -> Result<Value> {
        let proxy = self.proxy(name)?;
        let info = proxy.start_kernel(kernel_name).await?;
        let kernel_id = info.get("id").and_then(Value::as_str).map(str::to_owned);
        let mut session_id = None;
        if let Some(kernel_id) = &kernel_id {
            match proxy.create_session(name, kernel_id, kernel_name).await {
                Ok(session) => {
                    session_id = session.get("id").and_then(Value::as_str).map(str::to_owned);
                }
                Err(error) => tracing::info!("Could not name the session on the VM: {error}"),
            }
        }
        self.update(name, |state| {
            state.kernel_id = kernel_id;
            state.session_id = session_id;
            state.last_activity = unix_now();
        })?;
        Ok(info)
    }

    pub async fn restart_kernel(&self, name: &str) -> Result<Value> {
        let state = self.get(name)?;
        if let Some(channel) = lock(&self.inner.channels).remove(name) {
            channel.stop();
        }
        if let Some(kernel_id) = &state.kernel_id {
            match self.proxy(name)?.restart_kernel(kernel_id).await {
                Ok(_) => return Ok(json!({ "id": kernel_id, "restarted": true })),
                Err(error) => tracing::info!("Restart failed, starting a new kernel: {error}"),
            }
        }
        let info = self.start_kernel(name, "python3").await?;
        Ok(json!({ "id": info.get("id"), "restarted": false }))
    }

    pub async fn interrupt(&self, name: &str) -> Result<()> {
        let state = self.get(name)?;
        self.release_pending_reply(name);
        if let Some(kernel_id) = &state.kernel_id {
            self.proxy(name)?.interrupt_kernel(kernel_id).await?;
        }
        Ok(())
    }

    pub async fn shutdown_kernel(&self, name: &str) -> Result<()> {
        let state = self.get(name)?;
        if let Some(channel) = lock(&self.inner.channels).remove(name) {
            channel.stop();
        }
        if let Some(kernel_id) = &state.kernel_id {
            if let Err(error) = self.proxy(name)?.shutdown_kernel(kernel_id).await {
                tracing::info!("Kernel shutdown failed: {error}");
            }
        }
        self.update(name, |state| {
            state.kernel_id = None;
            state.session_id = None;
        })
    }

    /// Answer an `input()` prompt.
    pub async fn send_stdin(&self, name: &str, value: &str) -> Result<()> {
        self.channel(name).await?.send_input_reply(json!(value), json!({}))?;
        self.inner.history.log(name, "input_reply", json!({ "value": value }));
        Ok(())
    }

    // ------------------------------------------------- execute + streaming

    fn notify(&self, name: &str, event: Value) {
        if let Some(listeners) = lock(&self.inner.notifiers).get_mut(name) {
            listeners.retain(|listener| listener.send(event.clone()).is_ok());
        }
    }

    fn subscribe(&self, name: &str) -> mpsc::UnboundedReceiver<Value> {
        let (tx, rx) = mpsc::unbounded_channel();
        lock(&self.inner.notifiers).entry(name.to_owned()).or_default().push(tx);
        rx
    }

    /// Run a cell and stream every event to `emit` until it finishes. The
    /// final event is always an `execute_reply`, which is also returned.
    /// `record = false` keeps the cell out of the history log (automations
    /// log themselves).
    pub async fn execute(
        &self,
        name: &str,
        code: &str,
        timeout: Duration,
        record: bool,
        emit: &Emit,
    ) -> Result<Value> {
        if code.trim().is_empty() {
            return Err(Error::invalid("There is no code to run."));
        }
        let channel = self.channel(name).await?;
        self.touch(name);
        let mut notifications = self.subscribe(name);
        let (outputs_tx, mut outputs) = mpsc::unbounded_channel::<Value>();
        let mut recorder = Recorder {
            history: self.inner.history.clone(),
            name: name.to_owned(),
            code: code.to_owned(),
            seen: Vec::new(),
            record,
        };

        let execution = channel.execute(code, outputs_tx, timeout, true);
        tokio::pin!(execution);
        let result = loop {
            tokio::select! {
                Some(event) = notifications.recv() => emit(event),
                Some(event) = outputs.recv() => {
                    recorder.seen.push(event.clone());
                    emit(event);
                }
                result = &mut execution => break result,
            }
        };
        while let Ok(event) = outputs.try_recv() {
            recorder.seen.push(event.clone());
            emit(event);
        }
        while let Ok(event) = notifications.try_recv() {
            emit(event);
        }

        let reply = match result {
            Ok(reply) => json!({
                "type": "execute_reply",
                "status": reply.get("status").cloned().unwrap_or(Value::Null),
                "execution_count": reply.get("execution_count").cloned().unwrap_or(Value::Null),
            }),
            Err(error) => {
                let event = json!({
                    "type": "error",
                    "ename": error_name(&error),
                    "evalue": error.to_string(),
                    "traceback": [],
                });
                recorder.seen.push(event.clone());
                emit(event);
                json!({ "type": "execute_reply", "status": "error" })
            }
        };
        recorder.seen.push(reply.clone());
        emit(reply.clone());
        self.touch(name);
        Ok(reply)
    }

    // ------------------------------------------------ credential requests

    async fn handle_colab_request(
        &self,
        name: &str,
        channel: Arc<KernelChannel>,
        request: ColabRequest,
    ) {
        let reply_now = |channel: &KernelChannel, request: &ColabRequest| {
            if let Err(error) = channel.send_input_reply(request.reply(), request.header.clone()) {
                tracing::info!("Could not answer colab_request: {error}");
            }
        };
        let Some(auth_type) = request.auth_type().and_then(AuthType::parse) else {
            reply_now(&channel, &request);
            return;
        };
        let Ok(state) = self.get(name) else {
            reply_now(&channel, &request);
            return;
        };
        let what = auth_type.label();
        self.notify(
            name,
            json!({"type": "colab_request", "auth_type": auth_type, "message": format!("{what} authorization requested by the VM…")}),
        );
        self.inner.history.log(
            name,
            "colab_request",
            json!({"type": auth_type, "colab_msg_id": request.colab_msg_id}),
        );

        match self.inner.client.propagate_credentials(&state.endpoint, auth_type).await {
            Err(error) => {
                tracing::warn!("Credential propagation failed: {error}");
                self.notify(
                    name,
                    json!({"type": "error", "ename": "PropagationError", "evalue": format!("{what} propagation failed: {error}"), "traceback": []}),
                );
                // Never wedge the kernel on a hard failure.
                reply_now(&channel, &request);
            }
            Ok(outcome) if outcome.success => {
                self.mark_propagated(name, auth_type);
                self.notify(
                    name,
                    json!({"type": "colab_request", "auth_type": auth_type, "message": format!("{what} credentials propagated. Resuming…")}),
                );
                reply_now(&channel, &request);
            }
            Ok(outcome) => {
                let uri = outcome.unauthorized_redirect_uri;
                let _ = self.update(name, |state| state.drive_pending_uri = uri.clone());
                lock(&self.inner.pending_colab).insert(
                    name.to_owned(),
                    PendingColab { channel: Arc::downgrade(&channel), request, auth_type },
                );
                self.inner.history.log(name, "drive_auth_needed", json!({ "uri": uri }));
                self.notify(
                    name,
                    json!({
                        "type": "drive_auth_required",
                        "auth_type": auth_type,
                        "message": format!(
                            "{what} access has not been granted yet. Open the link, approve \
                             access, then click Continue — the cell resumes where it paused."
                        ),
                        "uri": uri,
                    }),
                );
            }
        }
    }

    fn mark_propagated(&self, name: &str, auth_type: AuthType) {
        let _ = self.update(name, |state| {
            if auth_type == AuthType::DfsEphemeral {
                state.drive_authorized = true;
            }
            state.drive_pending_uri = None;
        });
        self.inner.history.log(name, "drive_auth_success", json!({ "type": auth_type }));
    }

    /// Unblock a kernel still waiting on a credentials reply.
    fn release_pending_reply(&self, name: &str) -> bool {
        let Some(pending) = lock(&self.inner.pending_colab).remove(name) else {
            return false;
        };
        if let Some(channel) = pending.channel.upgrade() {
            if let Err(error) =
                channel.send_input_reply(pending.request.reply(), pending.request.header.clone())
            {
                tracing::info!("Could not release the paused cell: {error}");
            }
        }
        true
    }

    /// Retry credential propagation after the user consented; a paused cell
    /// resumes on success (the CLI's "press Enter after granting access").
    pub async fn authorize_drive(&self, name: &str) -> Result<Value> {
        let state = self.get(name)?;
        let auth_type = lock(&self.inner.pending_colab)
            .get(name)
            .map(|pending| pending.auth_type)
            .unwrap_or(AuthType::DfsEphemeral);
        let outcome = self.inner.client.propagate_credentials(&state.endpoint, auth_type).await?;
        if outcome.success {
            self.mark_propagated(name, auth_type);
            let resumed = self.release_pending_reply(name);
            Ok(json!({ "success": true, "resumed": resumed }))
        } else {
            let uri = outcome.unauthorized_redirect_uri;
            self.update(name, |state| state.drive_pending_uri = uri.clone())?;
            Ok(json!({ "success": false, "unauthorizedRedirectUri": uri }))
        }
    }

    // ------------------------------------------------------------ keep-alive

    /// One keep-alive ping (also used by the background loop).
    pub async fn keepalive(&self, name: &str) -> Result<Value> {
        let state = self.get(name)?;
        match self.inner.client.keep_alive(&state.endpoint).await {
            Ok(()) => {
                let now = unix_now();
                self.update(name, |state| {
                    state.last_keepalive = Some(now);
                    state.keepalive_error = None;
                })?;
                Ok(json!({ "ok": true, "at": now }))
            }
            Err(error) => {
                let message = error.to_string();
                self.update(name, |state| state.keepalive_error = Some(message.clone()))?;
                Ok(json!({ "ok": false, "error": message }))
            }
        }
    }

    /// The CLI's `keep_alive` loop: every interval, for at most 24 hours,
    /// while the runtime is tracked. Idempotent.
    pub fn start_keepalive(&self, name: &str) {
        if !self.inner.keepalive_enabled.load(Ordering::SeqCst) {
            return;
        }
        let mut tasks = lock(&self.inner.keepalive_tasks);
        if tasks.get(name).is_some_and(|task| !task.is_finished()) {
            return;
        }
        let manager = self.clone();
        let owner = name.to_owned();
        let task = tokio::spawn(async move {
            let started = Instant::now();
            while started.elapsed() < KEEP_ALIVE_MAX {
                if manager.get(&owner).is_err() {
                    return;
                }
                if let Err(error) = manager.keepalive(&owner).await {
                    tracing::debug!("Keep-alive for {owner} stopped: {error}");
                    return;
                }
                let interval = *lock(&manager.inner.keepalive_interval);
                tokio::time::sleep(interval).await;
            }
        });
        tasks.insert(name.to_owned(), task);
    }

    // --------------------------------------------------------------- release

    /// Disconnect, release the VM and forget the runtime. With
    /// `unassign = false` the VM stays alive on Google's side (used by
    /// [`Self::release_endpoint`], which unassigns once for all entries).
    pub async fn stop(&self, name: &str, unassign: bool) -> Result<StopOutcome> {
        let state = lock(&self.inner.sessions).remove(name).ok_or_else(|| missing(name))?;
        if let Some(channel) = lock(&self.inner.channels).remove(name) {
            channel.stop();
        }
        if let Some(task) = lock(&self.inner.keepalive_tasks).remove(name) {
            task.abort();
        }
        lock(&self.inner.pending_colab).remove(name);
        self.save();
        self.inner.history.log(
            name,
            "session_terminated",
            json!({ "reason": if unassign { "user_requested" } else { "released" } }),
        );
        if !unassign || state.endpoint.is_empty() {
            return Ok(StopOutcome { released: false, warning: None });
        }
        match self.inner.client.unassign(&state.endpoint).await {
            Ok(()) => Ok(StopOutcome { released: true, warning: None }),
            Err(error) => {
                tracing::warn!("Unassign of {} failed: {error}", state.endpoint);
                Ok(StopOutcome {
                    released: false,
                    warning: Some(format!(
                        "Colab did not confirm the release ({error}). If the VM still shows \
                         under Assignments, release it there."
                    )),
                })
            }
        }
    }

    /// Release an assignment, forgetting every local runtime bound to it,
    /// then unassign exactly once.
    pub async fn release_endpoint(&self, endpoint: &str) -> Result<()> {
        let endpoint = validate_endpoint(endpoint)?;
        let names: Vec<String> = lock(&self.inner.sessions)
            .values()
            .filter(|state| state.endpoint == endpoint)
            .map(|state| state.name.clone())
            .collect();
        for name in names {
            self.stop(&name, false).await?;
        }
        self.inner.client.unassign(endpoint).await
    }

    /// Release every runtime (disconnecting Google).
    pub async fn stop_all(&self) {
        for name in self.names() {
            if let Err(error) = self.stop(&name, true).await {
                tracing::warn!("Could not release {name}: {error}");
            }
        }
    }

    /// Stop background work without releasing anything (app shutdown).
    pub fn shutdown(&self) {
        for (_, task) in lock(&self.inner.keepalive_tasks).drain() {
            task.abort();
        }
        for (_, channel) in lock(&self.inner.channels).drain() {
            channel.stop();
        }
    }

    // ----------------------------------------------------------------- files

    fn log_file_op(&self, name: &str, op: &str, path: &str, extra: Value) {
        let mut event = json!({ "op": op, "path": path });
        if let (Some(event), Value::Object(extra)) = (event.as_object_mut(), extra) {
            event.extend(extra);
        }
        self.inner.history.log(name, "file_operation", event);
    }

    pub async fn list_files(&self, name: &str, path: &str) -> Result<FileListing> {
        let listing = self.proxy(name)?.list_contents(path).await?;
        let text =
            |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let kind = text(&listing, "type").unwrap_or_default();
        let mut entries: Vec<FileEntry> = if kind == "directory" {
            listing
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|item| FileEntry {
                    name: text(item, "name").unwrap_or_default(),
                    path: text(item, "path").unwrap_or_default(),
                    kind: text(item, "type").unwrap_or_default(),
                    size: item.get("size").and_then(Value::as_u64),
                    last_modified: text(item, "last_modified"),
                    mimetype: text(item, "mimetype"),
                })
                .collect()
        } else {
            Vec::new()
        };
        entries.sort_by(|a, b| {
            (a.kind != "directory", a.name.to_lowercase())
                .cmp(&(b.kind != "directory", b.name.to_lowercase()))
        });
        Ok(FileListing {
            path: text(&listing, "path").unwrap_or_else(|| path.trim_matches('/').to_owned()),
            kind,
            entries,
        })
    }

    pub async fn read_file(&self, name: &str, path: &str) -> Result<Value> {
        self.proxy(name)?.read_file(require_path(path)?).await
    }

    pub async fn download_file(&self, name: &str, path: &str) -> Result<Vec<u8>> {
        self.proxy(name)?.download(require_path(path)?).await
    }

    pub async fn write_file(&self, name: &str, path: &str, content: &str) -> Result<Value> {
        let path = require_path(path)?;
        self.touch(name);
        let info = self.proxy(name)?.write_file(path, content, "text").await?;
        self.log_file_op(name, "write", path, json!({}));
        Ok(info)
    }

    pub async fn upload_file(&self, name: &str, path: &str, bytes: &[u8]) -> Result<Value> {
        let path = require_path(path)?;
        self.touch(name);
        let info = self.proxy(name)?.upload_file(path, bytes).await?;
        self.log_file_op(name, "upload", path, json!({ "size": bytes.len() }));
        Ok(info)
    }

    /// Create the folders above `path`. Folders that exist are left alone,
    /// and a real failure surfaces when the file itself is written.
    pub async fn make_parents(&self, name: &str, path: &str) {
        let Ok(proxy) = self.proxy(name) else { return };
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        for depth in 1..parts.len() {
            if let Err(error) = proxy.make_directory(&parts[..depth].join("/")).await {
                tracing::debug!("mkdir {} failed: {error}", parts[..depth].join("/"));
            }
        }
    }

    pub async fn make_directory(&self, name: &str, path: &str) -> Result<Value> {
        let path = require_path(path)?;
        let info = self.proxy(name)?.make_directory(path).await?;
        self.log_file_op(name, "mkdir", path, json!({}));
        Ok(info)
    }

    pub async fn delete_file(&self, name: &str, path: &str) -> Result<()> {
        let path = require_path(path)?;
        self.proxy(name)?.delete(path).await?;
        self.log_file_op(name, "rm", path, json!({}));
        Ok(())
    }

    /// `PATCH api/contents` — VS Code's `colab.renameFile`.
    pub async fn rename_file(&self, name: &str, path: &str, new_path: &str) -> Result<Value> {
        let path = require_path(path)?;
        let new_path = require_path(new_path)?;
        let info = self.proxy(name)?.rename(path, new_path).await?;
        self.log_file_op(name, "rename", path, json!({ "new_path": new_path }));
        Ok(info)
    }

    // ---------------------------------------------------- telemetry, terminal

    pub async fn resources(&self, name: &str) -> Result<Resources> {
        let state = self.get(name)?;
        let raw = self.inner.client.get_resources(&state.url, &state.token).await?;
        Ok(resources::normalize(&raw))
    }

    /// Open a shell on the VM; keystrokes count as activity.
    pub async fn open_terminal(&self, name: &str, sink: TerminalSink) -> Result<Terminal> {
        let proxy = self.proxy(name)?;
        let manager = self.clone();
        let owner = name.to_owned();
        let terminal =
            Terminal::open(&proxy, sink, Arc::new(move || manager.touch(&owner))).await?;
        self.inner.history.log(name, "console_started", json!({}));
        Ok(terminal)
    }
}

fn missing(name: &str) -> Error {
    Error::not_found(format!("No such runtime: {name}"))
}

fn require_path(path: &str) -> Result<&str> {
    let trimmed = path.trim();
    if trimmed.trim_matches('/').is_empty() {
        Err(Error::invalid("A file path is required."))
    } else {
        Ok(trimmed)
    }
}
