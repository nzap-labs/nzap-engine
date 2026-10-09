//! What an agent can do with NZAP Engine: the tools behind `tools/call`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{json, Map, Value};

use nzap_core::colab::client::quota_message;
use nzap_core::config::{
    Accelerator, RuntimeRequest, GPU_CHOICES, HIGH_MEM_ONLY_CHOICES, TPU_CHOICES,
};
use nzap_core::ops::jobs::{self, JobInput, JobRequest};
use nzap_core::ops::runfile::EnvSpec;
use nzap_core::session::Emit;
use nzap_core::{Engine, Error, Result};

use crate::options::Options;
use crate::protocol::Peer;
use crate::transcript::{app_event, Transcript};

const DEFAULT_RUN_TIMEOUT: u64 = 600;
const MAX_RUN_TIMEOUT: u64 = 6 * 3600;

/// A tool's answer: text for the agent, plus any images a cell displayed.
#[derive(Debug, Default)]
pub struct Reply {
    pub text: String,
    pub images: Vec<(String, String)>,
    pub is_error: bool,
}

impl Reply {
    pub fn json(value: &Value) -> Self {
        Self { text: serde_json::to_string_pretty(value).unwrap_or_default(), ..Self::default() }
    }
}

/// How this server came to hold a runtime.
#[derive(Clone, Copy, Debug)]
struct Held {
    /// Started here (released when the agent leaves), not attached.
    started_here: bool,
    /// The agent asked to keep it after disconnecting.
    keep: bool,
}

pub struct Server {
    engine: Arc<Engine>,
    options: Options,
    held: Mutex<HashMap<String, Held>>,
    /// App slug → the runtime its model is warm on.
    app_runtimes: Mutex<HashMap<String, String>>,
    /// Jobs allocate their own VM; they count against the limit while running.
    running_jobs: AtomicUsize,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn short_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..6].to_owned()
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty())
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    str_arg(args, key).ok_or_else(|| Error::invalid(format!("Missing '{key}'.")))
}

fn bool_arg(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn timeout_arg(args: &Value) -> Duration {
    let seconds =
        args.get("timeout_seconds").and_then(Value::as_u64).unwrap_or(DEFAULT_RUN_TIMEOUT);
    Duration::from_secs(seconds.clamp(1, MAX_RUN_TIMEOUT))
}

fn strings(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| match item {
            Value::String(text) => Some(text.clone()),
            Value::Null => None,
            other => Some(other.to_string()),
        })
        .collect()
}

/// The hardware names agents may ask for.
pub fn hardware_choices() -> Vec<&'static str> {
    std::iter::once("cpu")
        .chain(GPU_CHOICES.iter().copied())
        .chain(TPU_CHOICES.iter().copied())
        .collect()
}

pub fn runtime_request(
    name: String,
    hardware: Option<&str>,
    high_mem: bool,
) -> Result<RuntimeRequest> {
    let hardware = hardware.unwrap_or("cpu").trim().to_lowercase();
    let mut request = RuntimeRequest { name, high_mem, ..RuntimeRequest::default() };
    match hardware.as_str() {
        "" | "cpu" | "none" => {}
        gpu if GPU_CHOICES.contains(&gpu) => request.gpu = Some(gpu.to_owned()),
        tpu if TPU_CHOICES.contains(&tpu) => request.tpu = Some(tpu.to_owned()),
        other => {
            return Err(Error::invalid(format!(
                "Unknown hardware '{other}'. Use one of: {}.",
                hardware_choices().join(", ")
            )))
        }
    }
    Ok(request)
}

/// A file name that is safe on the VM and on every desktop OS.
fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').to_owned();
    if cleaned.is_empty() {
        "file".into()
    } else {
        cleaned
    }
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).find(|part| !part.is_empty()).unwrap_or(path)
}

impl Server {
    pub fn new(engine: Arc<Engine>, options: Options) -> Arc<Self> {
        Arc::new(Self {
            engine,
            options,
            held: Mutex::default(),
            app_runtimes: Mutex::default(),
            running_jobs: AtomicUsize::new(0),
        })
    }

    pub fn engine(&self) -> &Arc<Engine> {
        &self.engine
    }

    /// The agent has gone: release what it started (unless asked to keep
    /// it), forget what it attached, and stop background work.
    pub async fn shutdown(&self) {
        let sessions = &self.engine.sessions;
        for name in sessions.names() {
            let held = lock(&self.held).get(&name).copied();
            // Not in `held`: a job's VM, whose job was cut short.
            let release = match held {
                Some(Held { started_here: false, .. }) => false,
                Some(Held { keep: true, .. }) => {
                    tracing::info!("Keeping runtime {name} as the agent asked");
                    continue;
                }
                _ => self.options.release_on_exit,
            };
            if !release && held.is_some_and(|held| held.started_here) {
                continue;
            }
            let stop = sessions.stop(&name, release);
            match tokio::time::timeout(Duration::from_secs(15), stop).await {
                Ok(Ok(_)) if release => tracing::info!("Released runtime {name}"),
                Ok(Ok(_)) => {}
                Ok(Err(error)) => tracing::warn!("Could not release {name}: {error}"),
                Err(_) => tracing::warn!("Releasing {name} timed out"),
            }
        }
        self.engine.shutdown();
    }

    fn at_capacity(&self, held: &HashMap<String, Held>) -> Result<()> {
        if held.len() + self.running_jobs.load(Ordering::SeqCst) >= self.options.max_runtimes {
            return Err(Error::invalid(format!(
                "This agent already holds {} runtime(s), its limit. Stop one with stop_runtime \
                 (or reuse it) first.",
                self.options.max_runtimes
            )));
        }
        Ok(())
    }

    /// Claim a slot for a runtime before the (slow) allocation.
    fn reserve(&self, name: &str, how: Held) -> Result<()> {
        let mut held = lock(&self.held);
        self.at_capacity(&held)?;
        if held.contains_key(name) {
            return Err(Error::invalid(format!("A runtime named '{name}' already exists.")));
        }
        held.insert(name.to_owned(), how);
        Ok(())
    }

    /// Claim a slot for a job's VM until the returned guard drops.
    fn reserve_job(&self) -> Result<JobSlot<'_>> {
        let held = lock(&self.held);
        self.at_capacity(&held)?;
        self.running_jobs.fetch_add(1, Ordering::SeqCst);
        Ok(JobSlot(&self.running_jobs))
    }

    /// A runtime this server holds.
    fn runtime(&self, args: &Value) -> Result<String> {
        let name = required(args, "runtime")?;
        if lock(&self.held).contains_key(name) {
            Ok(name.to_owned())
        } else {
            Err(Error::not_found(format!(
                "This agent holds no runtime named '{name}'. Call list_runtimes; a runtime the \
                 NZAP Engine app manages can be attached with attach_runtime."
            )))
        }
    }

    /// Allocation failures in the words the Colab CLI uses.
    fn allocation_error(error: Error, request: &RuntimeRequest) -> Error {
        match &error {
            Error::TooManyAssignments => Error::invalid(jobs::TOO_MANY_MESSAGE),
            Error::Colab { status: Some(400), .. } => {
                let (_, accelerator, _) = request.resolved();
                if accelerator == Accelerator::None {
                    error
                } else {
                    Error::Quota(quota_message(accelerator))
                }
            }
            _ => error,
        }
    }

    async fn start(&self, request: RuntimeRequest, keep: bool) -> Result<Value> {
        let name = request.name.clone();
        self.reserve(&name, Held { started_here: true, keep })?;
        match self.engine.sessions.create_and_connect(request.clone()).await {
            Ok((view, connected)) => {
                let mut view = serde_json::to_value(view)?;
                view["kernelConnected"] = json!(connected);
                Ok(view)
            }
            Err(error) => {
                lock(&self.held).remove(&name);
                Err(Self::allocation_error(error, &request))
            }
        }
    }

    // ----------------------------------------------------------------- tools

    pub async fn status(&self) -> Result<Reply> {
        let status = self.engine.status().await;
        let hint = (!status.connected).then_some(
            "Open the NZAP Engine app and click Connect Google, then try again. The agent \
             server uses the app's connection.",
        );
        Ok(Reply::json(&json!({
            "connected": status.connected,
            "email": status.email,
            "reason": status.reason,
            "warning": status.warning,
            "hint": hint,
            "runtimesHeld": lock(&self.held).len(),
            "maxRuntimes": self.options.max_runtimes,
            "hardware": hardware_choices(),
            "alwaysHighRam": HIGH_MEM_ONLY_CHOICES,
            "sharedFolders": self.options.files.roots(),
            "outputFolder": self.options.files.output_dir(),
            "maxFileMb": self.options.max_file_bytes / (1024 * 1024),
        })))
    }

    pub async fn list_runtimes(&self) -> Result<Reply> {
        let held = lock(&self.held).clone();
        let mine: Vec<Value> = self
            .engine
            .sessions
            .views()
            .into_iter()
            .filter_map(|view| {
                let how = held.get(&view.name)?;
                let mut value = serde_json::to_value(&view).ok()?;
                value["startedByThisAgent"] = json!(how.started_here);
                value["keepAfterDisconnect"] = json!(how.keep);
                Some(value)
            })
            .collect();
        let others = match self.engine.sessions.server_assignments().await {
            Ok(list) => json!(list
                .into_iter()
                .filter(|assignment| !assignment.managed)
                .map(|assignment| json!({
                    "endpoint": assignment.endpoint,
                    "accelerator": assignment.accelerator,
                    "shape": assignment.shape,
                }))
                .collect::<Vec<_>>()),
            Err(error) => json!({ "error": error.to_string() }),
        };
        Ok(Reply::json(&json!({
            "runtimes": mine,
            "otherRuntimesOnAccount": others,
            "note": "otherRuntimesOnAccount are VMs this agent does not hold (the NZAP Engine \
                     app's, or another tool's). attach_runtime uses one; they keep running \
                     when the agent leaves.",
        })))
    }

    pub async fn start_runtime(&self, args: &Value) -> Result<Reply> {
        let name =
            str_arg(args, "name").map_or_else(|| format!("agent-{}", short_id()), str::to_owned);
        let request = runtime_request(name, str_arg(args, "hardware"), bool_arg(args, "high_mem"))?;
        let view = self.start(request, bool_arg(args, "keep_after_disconnect")).await?;
        Ok(Reply::json(&json!({
            "runtime": view,
            "next": "Pass this runtime's name to run_code, upload_file, download_file or \
                     run_app. Stop it with stop_runtime when you are done: it uses the \
                     user's Colab compute units while it runs.",
        })))
    }

    pub async fn attach_runtime(&self, args: &Value) -> Result<Reply> {
        let endpoint = required(args, "endpoint")?;
        {
            let held = lock(&self.held);
            self.at_capacity(&held)?;
        }
        let view = self.engine.sessions.adopt(endpoint, str_arg(args, "name")).await?;
        lock(&self.held).insert(view.name.clone(), Held { started_here: false, keep: true });
        Ok(Reply::json(&json!({ "runtime": view })))
    }

    pub async fn stop_runtime(&self, args: &Value) -> Result<Reply> {
        let name = self.runtime(args)?;
        let held = lock(&self.held).get(&name).copied();
        let release = args
            .get("release")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| held.is_some_and(|held| held.started_here));
        let outcome = self.engine.sessions.stop(&name, release).await?;
        lock(&self.held).remove(&name);
        lock(&self.app_runtimes).retain(|_, runtime| *runtime != name);
        Ok(Reply::json(&json!({
            "runtime": name,
            "released": outcome.released,
            "warning": outcome.warning,
        })))
    }

    pub async fn run_code(&self, args: &Value) -> Result<Reply> {
        let name = self.runtime(args)?;
        let code = required(args, "code")?;
        let transcript = Arc::new(Mutex::new(Transcript::default()));
        let emit = self.collecting_emit(&name, transcript.clone());
        self.engine.sessions.execute(&name, code, timeout_arg(args), true, &emit).await?;
        let transcript = std::mem::take(&mut *lock(&transcript));
        let mut text = transcript.render();
        if text.trim().is_empty() {
            text = "(no output)".into();
        }
        let failed = transcript.failed();
        if failed && transcript.errors.is_empty() {
            text.push_str("\n[the cell did not finish successfully]");
        }
        Ok(Reply { text, images: transcript.images, is_error: failed })
    }

    /// Feed cell events into `transcript`, answering what an agent cannot:
    /// `input()` gets an empty line, browser consent interrupts the cell.
    fn collecting_emit(&self, name: &str, transcript: Arc<Mutex<Transcript>>) -> Emit {
        let sessions = self.engine.sessions.clone();
        let name = name.to_owned();
        Arc::new(move |event: Value| {
            match event.get("type").and_then(Value::as_str) {
                Some("input_request") => {
                    let (sessions, name) = (sessions.clone(), name.clone());
                    tokio::spawn(async move { sessions.send_stdin(&name, "").await });
                }
                Some("drive_auth_required") => {
                    let (sessions, name) = (sessions.clone(), name.clone());
                    tokio::spawn(async move { sessions.interrupt(&name).await });
                }
                _ => {}
            }
            lock(&transcript).push(&event);
        })
    }

    pub async fn list_files(&self, args: &Value) -> Result<Reply> {
        let name = self.runtime(args)?;
        let path = str_arg(args, "path").unwrap_or("/content");
        let listing = self.engine.sessions.list_files(&name, path).await?;
        Ok(Reply::json(&serde_json::to_value(listing)?))
    }

    fn read_local(&self, raw: &str) -> Result<(PathBuf, Vec<u8>)> {
        let path = self.options.files.readable(raw)?;
        let size = std::fs::metadata(&path)?.len();
        if size > self.options.max_file_bytes {
            return Err(self.too_large(&path.display().to_string(), size));
        }
        let bytes = std::fs::read(&path)?;
        Ok((path, bytes))
    }

    fn too_large(&self, what: &str, size: u64) -> Error {
        Error::invalid(format!(
            "{what} is {} MB; this server moves files up to {} MB (--max-file-mb).",
            size.div_ceil(1024 * 1024),
            self.options.max_file_bytes / (1024 * 1024)
        ))
    }

    pub async fn upload_file(&self, args: &Value) -> Result<Reply> {
        let name = self.runtime(args)?;
        let local = required(args, "local_path")?;
        let (path, bytes) = self.read_local(local)?;
        let file_name =
            path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let remote =
            jobs::remote_path(str_arg(args, "remote_path").unwrap_or(&safe_name(&file_name)));
        self.engine.sessions.make_parents(&name, &remote).await;
        self.engine.sessions.upload_file(&name, &remote, &bytes).await?;
        Ok(Reply::json(&json!({ "runtime": name, "remotePath": remote, "size": bytes.len() })))
    }

    pub async fn download_file(&self, args: &Value) -> Result<Reply> {
        let name = self.runtime(args)?;
        let remote = jobs::remote_path(required(args, "remote_path")?);
        let target = match str_arg(args, "local_path") {
            Some(local) => self.options.files.writable(Path::new(local))?,
            None => {
                let folder = self.options.files.folder(None, "downloads")?;
                self.options.files.writable(&folder.join(safe_name(basename(&remote))))?
            }
        };
        let bytes = self.engine.sessions.download_file(&name, &remote).await?;
        if bytes.len() as u64 > self.options.max_file_bytes {
            return Err(self.too_large(&remote, bytes.len() as u64));
        }
        tokio::fs::write(&target, &bytes).await?;
        Ok(Reply::json(&json!({ "remotePath": remote, "savedTo": target, "size": bytes.len() })))
    }

    pub async fn run_job(&self, args: &Value, peer: &Peer) -> Result<Reply> {
        let (filename, script) = match (str_arg(args, "script"), str_arg(args, "script_path")) {
            (Some(script), _) => {
                (str_arg(args, "filename").unwrap_or("script.py").to_owned(), script.to_owned())
            }
            (None, Some(path)) => {
                let (path, bytes) = self.read_local(path)?;
                let name = path.file_name().map(|name| name.to_string_lossy().into_owned());
                let script = String::from_utf8(bytes)
                    .map_err(|_| Error::invalid("The script is not UTF-8 text."))?;
                (name.unwrap_or_else(|| "script.py".into()), script)
            }
            (None, None) => {
                return Err(Error::invalid("Give 'script' (Python source) or 'script_path'."))
            }
        };
        let mut inputs = Vec::new();
        for input in args.get("inputs").and_then(Value::as_array).into_iter().flatten() {
            let local = input
                .get("local_path")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::invalid("Each input needs a 'local_path'."))?;
            let path = self.options.files.readable(local)?;
            let size = std::fs::metadata(&path)?.len();
            if size > self.options.max_file_bytes {
                return Err(self.too_large(local, size));
            }
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let remote = input
                .get("remote_path")
                .and_then(Value::as_str)
                .map_or_else(|| safe_name(&file_name), str::to_owned);
            inputs.push(JobInput { local: path, remote });
        }

        let request =
            runtime_request(String::new(), str_arg(args, "hardware"), bool_arg(args, "high_mem"))?;
        let env: EnvSpec = match args.get("env") {
            Some(env) if !env.is_null() => serde_json::from_value(env.clone())
                .map_err(|_| Error::invalid("'env' must map names to values."))?,
            _ => EnvSpec::default(),
        };
        let name = format!("agent-job-{}", short_id());
        let mut spec = jobs::plan(&JobRequest {
            filename: Some(filename),
            script,
            args: strings(args, "args"),
            env,
            artifacts: strings(args, "artifacts"),
            gpu: request.gpu,
            tpu: request.tpu,
            high_mem: request.high_mem,
            keep: false,
            timeout_seconds: Some(timeout_arg(args).as_secs()),
            name: Some(name.clone()),
        })?;
        spec.inputs = inputs;
        spec.max_artifact_bytes = self.options.max_file_bytes;
        spec.max_artifacts_total = self.options.max_file_bytes.saturating_mul(2);
        let _slot = self.reserve_job()?;
        let folder = if spec.artifacts.is_empty() {
            None
        } else {
            Some(self.options.files.folder(str_arg(args, "output_dir"), &format!("jobs/{name}"))?)
        };

        let transcript = Arc::new(Mutex::new(Transcript::default()));
        let artifacts = Arc::new(Mutex::new(Vec::new()));
        let emit: Emit = {
            let transcript = transcript.clone();
            let artifacts = artifacts.clone();
            let peer = peer.clone();
            Arc::new(move |event: Value| match event.get("type").and_then(Value::as_str) {
                Some("job") => {
                    let phase = event.get("phase").and_then(Value::as_str).unwrap_or_default();
                    peer.progress(&format!("job {phase}"));
                }
                Some("artifact") => lock(&artifacts).push(event),
                Some("input") => peer.progress("uploaded an input file"),
                Some("job_done") => {}
                _ => lock(&transcript).push(&event),
            })
        };
        let done = jobs::run(&self.engine.sessions, spec, folder.clone(), &emit).await?;
        let transcript = std::mem::take(&mut *lock(&transcript));
        let exit_code = done.get("exit_code").and_then(Value::as_i64).unwrap_or(1);
        let artifacts: Vec<Value> = lock(&artifacts)
            .iter()
            .map(|event| {
                let mut artifact = event.clone();
                if let Some(object) = artifact.as_object_mut() {
                    object.remove("type");
                }
                artifact
            })
            .collect();
        let error = done.get("error").cloned();
        Ok(Reply {
            text: serde_json::to_string_pretty(&json!({
                "exitCode": exit_code,
                "released": done.get("released"),
                "error": error,
                "artifacts": artifacts,
                "outputFolder": folder,
                "output": transcript.render(),
            }))?,
            images: transcript.images,
            is_error: exit_code != 0 || error.is_some(),
        })
    }

    pub async fn list_apps(&self) -> Result<Reply> {
        let apps: Vec<Value> = self
            .engine
            .notebooks
            .list()
            .into_iter()
            .filter_map(|notebook| {
                let app = notebook.app?;
                let widgets: HashMap<String, String> = app
                    .get("inputs")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|input| {
                        Some((
                            input.get("param")?.as_str()?.to_owned(),
                            input.get("widget")?.as_str()?.to_owned(),
                        ))
                    })
                    .collect();
                let params: Vec<Value> = notebook
                    .params
                    .iter()
                    .map(|param| {
                        let mut value = serde_json::to_value(param).unwrap_or(Value::Null);
                        if widgets.get(&param.key).map(String::as_str) == Some("file") {
                            value["isFile"] = json!(true);
                        }
                        value
                    })
                    .collect();
                Some(json!({
                    "id": notebook.id,
                    "title": notebook.title,
                    "tagline": app.get("tagline"),
                    "description": notebook.description,
                    "category": app.get("category"),
                    "runtime": app.get("runtime"),
                    "estimatedSeconds": app.get("estimates"),
                    "params": params,
                    "outputs": app.get("outputs"),
                }))
            })
            .collect();
        Ok(Reply::json(&json!({
            "apps": apps,
            "note": "Run one with run_app. File parameters (isFile) take a local path in \
                     'files'. The first run sets the model up; later runs on the same runtime \
                     are fast.",
        })))
    }

    async fn app_runtime(
        &self,
        args: &Value,
        slug: &str,
        app: Option<&Value>,
    ) -> Result<(String, bool)> {
        if str_arg(args, "runtime").is_some() {
            return Ok((self.runtime(args)?, false));
        }
        let warm = lock(&self.app_runtimes).get(slug).cloned();
        if let Some(name) = warm.filter(|name| lock(&self.held).contains_key(name)) {
            return Ok((name, false));
        }
        let suggested = app
            .and_then(|app| app.pointer("/runtime/accelerator"))
            .and_then(Value::as_str)
            .map(str::to_lowercase);
        let hardware = str_arg(args, "hardware").map(str::to_owned).or(suggested);
        let high_mem = bool_arg(args, "high_mem")
            || app
                .and_then(|app| app.pointer("/runtime/highMem"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
        let short: String = slug.chars().filter(char::is_ascii_alphanumeric).take(12).collect();
        let request =
            runtime_request(format!("app-{short}-{}", short_id()), hardware.as_deref(), high_mem)?;
        let name = request.name.clone();
        self.start(request, false).await?;
        lock(&self.app_runtimes).insert(slug.to_owned(), name.clone());
        Ok((name, true))
    }

    pub async fn run_app(&self, args: &Value, peer: &Peer) -> Result<Reply> {
        let wanted = required(args, "app")?;
        let id = if wanted.contains(':') { wanted.to_owned() } else { format!("public:{wanted}") };
        let notebook = match self.engine.notebooks.get(&id).await {
            Ok(notebook) => notebook,
            Err(Error::NotFound(_)) => {
                let local = self
                    .engine
                    .notebooks
                    .list()
                    .into_iter()
                    .find(|notebook| notebook.slug == wanted);
                match local {
                    Some(found) => self.engine.notebooks.get(&found.id).await?,
                    None => {
                        return Err(Error::not_found(format!("No app '{wanted}'. Call list_apps.")))
                    }
                }
            }
            Err(error) => return Err(error),
        };
        let mut values: Map<String, Value> = match args.get("params") {
            Some(Value::Object(map)) => map.clone(),
            Some(Value::Null) | None => Map::new(),
            Some(_) => return Err(Error::invalid("'params' must be an object.")),
        };
        // Validate local files before allocating anything.
        let mut uploads = Vec::new();
        if let Some(files) = args.get("files").and_then(Value::as_object) {
            for (param, local) in files {
                let local = local
                    .as_str()
                    .ok_or_else(|| Error::invalid("'files' maps parameters to local paths."))?;
                let (path, bytes) = self.read_local(local)?;
                uploads.push((param.clone(), path, bytes));
            }
        }

        peer.progress("preparing a runtime");
        let (runtime, started) =
            self.app_runtime(args, &notebook.slug, notebook.app.as_ref()).await?;
        for (param, path, bytes) in uploads {
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let remote = format!(
                "/content/nzap/inputs/{}/{}-{}",
                notebook.slug,
                chrono::Utc::now().timestamp_millis(),
                safe_name(&file_name)
            );
            self.engine.sessions.make_parents(&runtime, &remote).await;
            self.engine.sessions.upload_file(&runtime, &remote, &bytes).await?;
            values.insert(param, json!(remote));
        }

        let transcript = Arc::new(Mutex::new(Transcript::default()));
        let events = Arc::new(Mutex::new(Vec::<Value>::new()));
        let emit: Emit = {
            let inner = self.collecting_emit(&runtime, transcript.clone());
            let events = events.clone();
            let peer = peer.clone();
            Arc::new(move |event: Value| {
                if let Some(app) = app_event(&event) {
                    match app.get("event").and_then(Value::as_str) {
                        Some("stage") => {
                            peer.progress(
                                app.get("label").and_then(Value::as_str).unwrap_or("working"),
                            );
                        }
                        Some("ready") => peer.progress("model ready"),
                        _ => {}
                    }
                    lock(&events).push(app);
                } else {
                    inner(event);
                }
            })
        };
        peer.progress("running the app");
        self.engine
            .notebooks
            .run(&self.engine.sessions, &notebook.id, &runtime, &values, &emit)
            .await?;
        let transcript = std::mem::take(&mut *lock(&transcript));
        let events = std::mem::take(&mut *lock(&events));

        let mut warm = None;
        let mut device = None;
        let mut seconds = None;
        let mut outputs = Vec::new();
        let mut folder: Option<PathBuf> = None;
        for event in &events {
            match event.get("event").and_then(Value::as_str) {
                Some("ready") => {
                    warm = event.get("warm").cloned();
                    device = event.get("device").cloned();
                }
                Some("done") => seconds = event.get("seconds").cloned(),
                Some("output") => outputs.push(
                    self.app_output(&runtime, &notebook.slug, event, args, &mut folder).await,
                ),
                _ => {}
            }
        }
        let failed = transcript.failed();
        Ok(Reply {
            text: serde_json::to_string_pretty(&json!({
                "app": notebook.id,
                "runtime": runtime,
                "startedRuntime": started,
                "warm": warm,
                "device": device,
                "seconds": seconds,
                "outputs": outputs,
                "errors": transcript.errors,
                "log": transcript.render(),
                "next": format!(
                    "The model stays loaded on '{runtime}': pass runtime='{runtime}' to run \
                     again quickly, and stop_runtime when done."
                ),
            }))?,
            images: transcript.images,
            is_error: failed,
        })
    }

    /// One app output: media is downloaded into the output folder; text,
    /// JSON and tables are returned inline.
    async fn app_output(
        &self,
        runtime: &str,
        slug: &str,
        event: &Value,
        args: &Value,
        folder: &mut Option<PathBuf>,
    ) -> Value {
        let mut output = event.clone();
        if let Some(object) = output.as_object_mut() {
            object.remove("event");
        }
        let Some(path) = event.get("path").and_then(Value::as_str) else { return output };
        let result: Result<(PathBuf, usize)> = async {
            let dir = match folder {
                Some(dir) => dir.clone(),
                None => {
                    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
                    let dir = self
                        .options
                        .files
                        .folder(str_arg(args, "output_dir"), &format!("apps/{slug}/{stamp}"))?;
                    *folder = Some(dir.clone());
                    dir
                }
            };
            let file =
                event.get("filename").and_then(Value::as_str).unwrap_or_else(|| basename(path));
            let target = self.options.files.writable(&dir.join(safe_name(file)))?;
            let bytes = self.engine.sessions.download_file(runtime, path).await?;
            if bytes.len() as u64 > self.options.max_file_bytes {
                return Err(self.too_large(path, bytes.len() as u64));
            }
            tokio::fs::write(&target, &bytes).await?;
            Ok((target, bytes.len()))
        }
        .await;
        match result {
            Ok((target, size)) => {
                output["savedTo"] = json!(target);
                output["size"] = json!(size);
            }
            Err(error) => output["downloadError"] = json!(error.to_string()),
        }
        output
    }
}

/// Frees a job's slot however the job ends.
struct JobSlot<'a>(&'a AtomicUsize);

impl Drop for JobSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_names() {
        let gpu = runtime_request("a".into(), Some("T4"), false).unwrap();
        assert_eq!(gpu.gpu.as_deref(), Some("t4"));
        let tpu = runtime_request("a".into(), Some("v5e1"), false).unwrap();
        assert_eq!(tpu.tpu.as_deref(), Some("v5e1"));
        let cpu = runtime_request("a".into(), None, true).unwrap();
        assert!(cpu.gpu.is_none() && cpu.tpu.is_none() && cpu.high_mem);
        assert!(runtime_request("a".into(), Some("rtx5090"), false).is_err());
        assert_eq!(hardware_choices()[0], "cpu");
    }

    #[test]
    fn names_are_safe() {
        assert_eq!(safe_name("my clip (1).mp4"), "my_clip__1_.mp4");
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_name("..."), "file");
        assert_eq!(basename("/content/out/a.wav"), "a.wav");
        assert_eq!(basename("dir/"), "dir");
    }
}
