//! Ephemeral jobs — port of `colab run` (google-colab-cli
//! `commands/run.py`, via colab-studio `jobs.py`).
//!
//! `colab new` + `colab exec` + `colab stop` in one call: allocate a fresh
//! VM, run a script with `python script.py ARGS…` semantics, and release the
//! VM when it finishes (unless `keep`). Artifacts (paths or globs under
//! `/content`) are downloaded before the VM goes away and written to a local
//! folder the caller chooses.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::automation::tee;
use super::runfile::{env_prelude, parse_env, strip_shebang, EnvSpec, CHDIR_CONTENT};
use crate::colab::client::quota_message;
use crate::config::{Accelerator, RuntimeRequest};
use crate::error::{Error, Result};
use crate::python;
use crate::session::{Emit, SessionManager};

pub const MAX_ARTIFACT_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_ARTIFACTS_TOTAL: u64 = 50 * 1024 * 1024;
pub const MAX_ARTIFACT_PATTERNS: usize = 20;
pub const MAX_ARTIFACT_FILES: usize = 50;
const ARTIFACT_MARKER: &str = "__NZAP_ARTIFACTS__";

/// run.py's message for the common allocation refusal.
pub const TOO_MANY_MESSAGE: &str = "Allocation refused (precondition failed). This can mean too \
    many active sessions, or a temporary usage or capacity limit for the requested runtime. \
    Stop a session, wait and retry, or try a different accelerator.";

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRequest {
    #[serde(default)]
    pub filename: Option<String>,
    pub script: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: EnvSpec,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub gpu: Option<String>,
    #[serde(default)]
    pub tpu: Option<String>,
    #[serde(default, alias = "high_mem")]
    pub high_mem: bool,
    #[serde(default)]
    pub keep: bool,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
}

/// A validated job.
#[derive(Clone, Debug)]
pub struct JobSpec {
    pub filename: String,
    pub script: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub artifacts: Vec<String>,
    pub runtime: RuntimeRequest,
    pub keep: bool,
    pub timeout: Duration,
}

pub fn plan(request: &JobRequest) -> Result<JobSpec> {
    let filename = request
        .filename
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "script.py".into());
    if request.script.trim().is_empty() {
        return Err(Error::invalid("Missing script."));
    }
    if !filename.ends_with(".py") {
        return Err(Error::invalid("Jobs run Python scripts (.py)."));
    }
    let artifacts: Vec<String> = request
        .artifacts
        .iter()
        .map(|pattern| pattern.trim().to_owned())
        .filter(|pattern| !pattern.is_empty())
        .collect();
    if artifacts.len() > MAX_ARTIFACT_PATTERNS {
        return Err(Error::invalid(format!("At most {MAX_ARTIFACT_PATTERNS} artifact patterns.")));
    }
    let name = match request.name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => name.to_owned(),
        None => format!("run-{}", &uuid::Uuid::new_v4().simple().to_string()[..6]),
    };
    Ok(JobSpec {
        filename,
        script: request.script.clone(),
        args: request.args.clone(),
        env: parse_env(&request.env)?,
        artifacts,
        runtime: RuntimeRequest {
            name,
            gpu: request.gpu.clone(),
            tpu: request.tpu.clone(),
            high_mem: request.high_mem,
        },
        keep: request.keep,
        timeout: Duration::from_secs(request.timeout_seconds.unwrap_or(3600)),
    })
}

/// run.py `_build_script_payload`: argv, `__main__`, env, no shebang.
pub fn build_script_payload(
    filename: &str,
    body: &str,
    args: &[String],
    env: &[(String, String)],
) -> String {
    let basename =
        filename.rsplit(['/', '\\']).next().filter(|name| !name.is_empty()).unwrap_or("script.py");
    let mut argv = vec![basename.to_owned()];
    argv.extend(args.iter().cloned());
    format!(
        "import sys, warnings\nsys.argv = {}\n__name__ = '__main__'\n\
         warnings.filterwarnings('ignore', message=\"To exit: use\")\n{}{}",
        python::list(&argv),
        env_prelude(env),
        strip_shebang(body)
    )
}

pub fn is_systemexit(event: &Value) -> bool {
    event["type"] == "error" && event["ename"] == "SystemExit"
}

/// CPython's convention: None/0 → 0, int → int, anything else → 1.
pub fn systemexit_code(event: &Value) -> i64 {
    let value = match &event["evalue"] {
        Value::String(text) => text.trim().to_owned(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    if value.is_empty() || value == "None" || value == "0" {
        0
    } else {
        value.parse().unwrap_or(1)
    }
}

/// run.py `_exit_code_from_outputs`: the last non-zero SystemExit wins; any
/// other error is 1.
pub fn exit_code(events: &[Value]) -> i64 {
    let mut code = 0;
    for event in events.iter().filter(|event| event["type"] == "error") {
        if is_systemexit(event) {
            let value = systemexit_code(event);
            if value != 0 {
                code = value;
            }
        } else {
            return 1;
        }
    }
    code
}

/// A cell that expands artifact patterns on the VM and prints JSON.
pub fn glob_code(patterns: &[String]) -> String {
    format!(
        "import glob, json, os\n\
         _patterns = {}\n\
         _found = []\n\
         for _p in _patterns:\n    \
         _p = _p if os.path.isabs(_p) else os.path.join('/content', _p)\n    \
         for _m in sorted(glob.glob(_p, recursive=True)):\n        \
         if os.path.isfile(_m):\n            \
         _found.append({{'path': _m, 'size': os.path.getsize(_m)}})\n\
         print('{ARTIFACT_MARKER}' + json.dumps(_found))\n",
        python::list(patterns)
    )
}

pub fn parse_glob_output(events: &[Value]) -> Vec<(String, u64)> {
    let text: String = events
        .iter()
        .filter(|event| event["type"] == "stream")
        .filter_map(|event| event["text"].as_str())
        .collect();
    let Some((_, rest)) = text.split_once(ARTIFACT_MARKER) else {
        return Vec::new();
    };
    let line = rest.lines().next().unwrap_or_default().trim();
    serde_json::from_str::<Vec<Value>>(line)
        .unwrap_or_default()
        .iter()
        .filter_map(|item| {
            let path = item["path"].as_str()?.to_owned();
            Some((path, item["size"].as_u64().unwrap_or_default()))
        })
        .collect()
}

/// Where a remote artifact lands locally: its path relative to `/content`,
/// with anything that could escape `dir` removed.
pub fn local_artifact_path(dir: &Path, remote: &str) -> Option<PathBuf> {
    let relative = remote.trim_start_matches('/');
    let relative = relative.strip_prefix("content/").unwrap_or(relative);
    let mut path = dir.to_path_buf();
    let mut pushed = false;
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => {
                path.push(part);
                pushed = true;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    pushed.then_some(path)
}

/// `colab run`: allocate → run → collect artifacts → release. The VM is
/// released on every path (including errors and cancellation) unless `keep`.
pub async fn run(
    manager: &SessionManager,
    spec: JobSpec,
    artifacts_dir: Option<PathBuf>,
    emit: &Emit,
) -> Result<Value> {
    let name = spec.runtime.name.clone();
    let hardware = spec.runtime.label();
    emit(json!({"type": "job", "phase": "assigning", "session": name, "hardware": hardware}));

    let view = match manager.create(spec.runtime.clone()).await {
        Ok(view) => view,
        Err(error) => {
            let message = match &error {
                Error::TooManyAssignments => TOO_MANY_MESSAGE.to_owned(),
                Error::Colab { status: Some(400), .. } => {
                    let (_, accelerator, _) = spec.runtime.resolved();
                    if accelerator == Accelerator::None {
                        error.to_string()
                    } else {
                        quota_message(accelerator)
                    }
                }
                other => other.to_string(),
            };
            let done = json!({"type": "job_done", "exit_code": 1, "released": true, "error": message, "session": name});
            emit(done.clone());
            return Ok(done);
        }
    };

    // Released on drop unless `keep` — covers errors and cancellation.
    let mut release =
        ReleaseGuard { manager: manager.clone(), name: name.clone(), armed: !spec.keep };
    let outcome =
        run_on(manager, &spec, &view.endpoint, &view.accelerator, artifacts_dir, emit).await;
    let released = if spec.keep {
        false
    } else {
        release.armed = false;
        let released =
            manager.stop(&name, true).await.map(|outcome| outcome.released).unwrap_or(false);
        emit(json!({"type": "job", "phase": "released", "session": name}));
        released
    };
    let (code, artifacts, error) = match outcome {
        Ok((code, artifacts)) => (code, artifacts, None),
        Err(error) => (1, Vec::new(), Some(error.to_string())),
    };
    manager.history().log(
        &name,
        "automation_result",
        json!({"op": "run", "exit_code": code, "artifacts": artifacts, "kept": spec.keep}),
    );
    let mut done = json!({"type": "job_done", "exit_code": code, "released": released, "kept": spec.keep, "session": name});
    if let Some(error) = error {
        done["error"] = json!(error);
    }
    emit(done.clone());
    Ok(done)
}

async fn run_on(
    manager: &SessionManager,
    spec: &JobSpec,
    endpoint: &str,
    hardware: &str,
    artifacts_dir: Option<PathBuf>,
    emit: &Emit,
) -> Result<(i64, Vec<Value>)> {
    let name = &spec.runtime.name;
    emit(
        json!({"type": "job", "phase": "connecting", "session": name, "endpoint": endpoint, "hardware": hardware}),
    );
    let silent: Emit = Arc::new(|_| {});
    manager.execute(name, CHDIR_CONTENT, Duration::from_secs(120), false, &silent).await?;

    emit(json!({"type": "job", "phase": "running", "session": name}));
    let payload = build_script_payload(&spec.filename, &spec.script, &spec.args, &spec.env);
    // SystemExit tracebacks are suppressed, as in the CLI.
    let forward = emit.clone();
    let filtered: Emit = Arc::new(move |event: Value| {
        if !is_systemexit(&event) {
            forward(event);
        }
    });
    let (script_emit, seen) = tee(&filtered);
    manager.execute(name, &payload, spec.timeout, true, &script_emit).await?;
    let seen = seen.lock().map(|seen| seen.clone()).unwrap_or_default();
    let code = exit_code(&seen);

    let mut artifacts = Vec::new();
    if spec.artifacts.is_empty() {
        return Ok((code, artifacts));
    }
    emit(json!({"type": "job", "phase": "collecting", "session": name}));
    let (glob_emit, glob_seen) = tee(&silent);
    manager
        .execute(name, &glob_code(&spec.artifacts), Duration::from_secs(120), false, &glob_emit)
        .await?;
    let found = parse_glob_output(&glob_seen.lock().map(|seen| seen.clone()).unwrap_or_default());
    let Some(dir) = artifacts_dir else {
        for (path, size) in found {
            emit(
                json!({"type": "artifact", "path": path, "size": size, "skipped": "no download folder"}),
            );
        }
        return Ok((code, artifacts));
    };
    let mut total = 0u64;
    for (path, size) in found.into_iter().take(MAX_ARTIFACT_FILES) {
        if size > MAX_ARTIFACT_BYTES || total + size > MAX_ARTIFACTS_TOTAL {
            emit(json!({"type": "artifact", "path": path, "size": size, "skipped": "too large"}));
            continue;
        }
        let Some(local) = local_artifact_path(&dir, &path) else {
            emit(json!({"type": "artifact", "path": path, "size": size, "skipped": "unsafe path"}));
            continue;
        };
        let bytes = manager.download_file(name, &path).await?;
        total += bytes.len() as u64;
        if let Some(parent) = local.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&local, &bytes).await?;
        let saved = local.to_string_lossy().into_owned();
        artifacts.push(json!({"path": path, "size": bytes.len(), "savedTo": saved}));
        emit(json!({"type": "artifact", "path": path, "size": bytes.len(), "savedTo": saved}));
    }
    Ok((code, artifacts))
}

/// Releases the job's VM if the job future is dropped mid-way.
struct ReleaseGuard {
    manager: SessionManager,
    name: String,
    armed: bool,
}

impl Drop for ReleaseGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let manager = self.manager.clone();
        let name = std::mem::take(&mut self.name);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Err(error) = manager.stop(&name, true).await {
                    tracing::warn!("Could not release cancelled job {name}: {error}");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_validate() {
        let ok = plan(&JobRequest {
            script: "print(1)".into(),
            gpu: Some("t4".into()),
            ..JobRequest::default()
        })
        .unwrap();
        assert_eq!(ok.filename, "script.py");
        assert!(ok.runtime.name.starts_with("run-"));
        assert_eq!(ok.runtime.label(), "T4");
        assert!(plan(&JobRequest { script: " ".into(), ..JobRequest::default() }).is_err());
        assert!(plan(&JobRequest {
            script: "x".into(),
            filename: Some("a.ipynb".into()),
            ..JobRequest::default()
        })
        .is_err());
        assert!(plan(&JobRequest {
            script: "x".into(),
            artifacts: vec!["*".into(); 21],
            ..JobRequest::default()
        })
        .is_err());
    }

    #[test]
    fn script_payload() {
        let payload = build_script_payload(
            "dir/train.py",
            "#!/usr/bin/env python\nprint(1)",
            &["--epochs".into(), "3".into()],
            &[("SEED".into(), "7".into())],
        );
        assert!(payload
            .starts_with("import sys, warnings\nsys.argv = [\"train.py\", \"--epochs\", \"3\"]\n"));
        assert!(payload.contains("__name__ = '__main__'"));
        assert!(payload.contains("os.environ[\"SEED\"] = \"7\""));
        assert!(payload.ends_with("print(1)"));
        assert!(!payload.contains("#!"));
    }

    #[test]
    fn exit_codes_follow_cpython() {
        let exit = |value: Value| json!({"type": "error", "ename": "SystemExit", "evalue": value});
        assert_eq!(exit_code(&[]), 0);
        assert_eq!(exit_code(&[exit(json!("3"))]), 3);
        assert_eq!(exit_code(&[exit(json!("None"))]), 0);
        assert_eq!(exit_code(&[exit(json!("oops"))]), 1);
        assert_eq!(exit_code(&[exit(json!("2")), exit(json!("0"))]), 2);
        assert_eq!(exit_code(&[json!({"type": "error", "ename": "ValueError"})]), 1);
        assert!(is_systemexit(&exit(json!("1"))));
    }

    #[test]
    fn artifact_discovery() {
        let code = glob_code(&["out/*.bin".into()]);
        assert!(code.contains("_patterns = [\"out/*.bin\"]"));
        assert!(code.contains("{'path': _m, 'size': os.path.getsize(_m)}"));
        assert!(code.contains("print('__NZAP_ARTIFACTS__' + json.dumps(_found))"));
        let events = vec![
            json!({"type": "stream", "text": "noise\n__NZAP_ARTIFACTS__[{\"path\": \"/content/out/a.bin\", \"size\": 9}]\n"}),
        ];
        assert_eq!(parse_glob_output(&events), vec![("/content/out/a.bin".to_owned(), 9)]);
        assert!(parse_glob_output(&[json!({"type": "stream", "text": "none"})]).is_empty());
    }

    #[test]
    fn artifact_paths_stay_inside_the_folder() {
        let dir = Path::new("/downloads/job");
        assert_eq!(
            local_artifact_path(dir, "/content/out/a.bin"),
            Some(dir.join("out").join("a.bin"))
        );
        assert_eq!(local_artifact_path(dir, "/tmp/x.txt"), Some(dir.join("tmp").join("x.txt")));
        assert_eq!(local_artifact_path(dir, "/content/../etc/passwd"), None);
        assert_eq!(local_artifact_path(dir, "/content/"), None);
    }
}
