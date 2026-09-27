//! VM automations — port of `google-colab-cli/src/colab_cli/commands/automation.py`
//! (via colab-studio's `automation.py`):
//!
//! * `install`    — `colab install [-r FILE | PKG...]`: `uv pip install
//!   --system` with a `pip` fallback
//! * `drivemount` — `colab drivemount [PATH]`: `drive.mount(path)`; the VM's
//!   `dfs_ephemeral` request is answered by the session manager
//! * `gcp-auth`   — `colab auth`: `auth.authenticate_user()`; the VM's
//!   `auth_user_ephemeral` request is answered the same way

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::history::collect_outputs;
use crate::python;
use crate::session::{Emit, SessionManager};

/// The kernel goes quiet while the user completes a consent screen
/// (automation.py `INTERACTIVE_AUTOMATION_TIMEOUT_SEC`).
pub const INTERACTIVE_TIMEOUT: Duration = Duration::from_secs(600);
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(1800);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Install,
    DriveMount,
    GcpAuth,
}

impl Operation {
    pub fn parse(op: &str) -> Result<Self> {
        match op {
            "install" => Ok(Self::Install),
            "drivemount" => Ok(Self::DriveMount),
            "gcp-auth" => Ok(Self::GcpAuth),
            other => Err(Error::invalid(format!(
                "Unknown automation '{other}' (use install, drivemount or gcp-auth)."
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::DriveMount => "drivemount",
            Self::GcpAuth => "gcp-auth",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Requirements {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub content: String,
}

/// What the UI sends.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AutomationRequest {
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub requirements: Option<Requirements>,
    #[serde(default)]
    pub path: Option<String>,
}

/// What to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub code: String,
    pub timeout: Duration,
    /// `(remote path, text)` to upload first (a requirements file).
    pub upload: Option<(String, String)>,
    /// Extra fields for the history log.
    pub log: Value,
}

/// The CLI's install cell, verbatim apart from literal quoting.
pub fn install_code(packages: &[String], requirements_path: Option<&str>) -> String {
    let mut args = Vec::new();
    if let Some(path) = requirements_path {
        args.push("-r".to_owned());
        args.push(path.to_owned());
    }
    args.extend(packages.iter().cloned());
    format!(
        "\nimport subprocess, sys\n\
         def install():\n    \
         packages = {}\n    \
         try:\n        \
         subprocess.check_call(['uv', 'pip', 'install', '--system'] + packages)\n        \
         print('Installation Complete (via uv)!')\n    \
         except:\n        \
         subprocess.check_call([sys.executable, '-m', 'pip', 'install'] + packages)\n        \
         print('Installation Complete (via pip)!')\n\
         install()\n",
        python::list(&args)
    )
}

pub fn drivemount_code(path: &str) -> String {
    format!("from google.colab import drive\ndrive.mount({})", python::literal(path))
}

/// The ephemeral flow: the VM sends a `colab_request` (`auth_user_ephemeral`)
/// that the engine answers through `/tun/m/credentials-propagation`.
pub fn gcp_auth_code() -> String {
    "from google.colab import auth\nauth.authenticate_user()\nprint('Authenticated with Google Cloud.')"
        .to_owned()
}

/// A requirements file name without directories.
fn basename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if base.is_empty() || base == "." || base == ".." {
        "requirements.txt".to_owned()
    } else {
        base.to_owned()
    }
}

/// Validate a request and return what to run.
pub fn plan(op: Operation, request: &AutomationRequest) -> Result<Plan> {
    match op {
        Operation::Install => {
            let packages: Vec<String> = request
                .packages
                .iter()
                .map(|package| package.trim().to_owned())
                .filter(|package| !package.is_empty())
                .collect();
            let mut upload = None;
            let mut requirements_path = None;
            if let Some(requirements) = &request.requirements {
                if requirements.content.trim().is_empty() {
                    return Err(Error::invalid("The requirements file is empty."));
                }
                // automation.py uploads it to content/<basename> and installs -r.
                let filename = basename(&requirements.filename);
                requirements_path = Some(format!("/content/{filename}"));
                upload = Some((format!("content/{filename}"), requirements.content.clone()));
            }
            if packages.is_empty() && requirements_path.is_none() {
                return Err(Error::invalid("No packages or requirements specified."));
            }
            Ok(Plan {
                code: install_code(&packages, requirements_path.as_deref()),
                timeout: INSTALL_TIMEOUT,
                upload,
                log: json!({ "packages": packages, "requirements": requirements_path }),
            })
        }
        Operation::DriveMount => {
            let path = request
                .path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .unwrap_or("/content/drive")
                .to_owned();
            if !path.starts_with('/') {
                return Err(Error::invalid(
                    "The mount path must be absolute, e.g. /content/drive.",
                ));
            }
            Ok(Plan {
                code: drivemount_code(&path),
                timeout: INTERACTIVE_TIMEOUT,
                upload: None,
                log: json!({ "path": path }),
            })
        }
        Operation::GcpAuth => Ok(Plan {
            code: gcp_auth_code(),
            timeout: INTERACTIVE_TIMEOUT,
            upload: None,
            log: json!({}),
        }),
    }
}

/// Collects a stream's events while forwarding them.
pub(crate) fn tee(emit: &Emit) -> (Emit, Arc<Mutex<Vec<Value>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let forward = emit.clone();
    let sink = seen.clone();
    let tee: Emit = Arc::new(move |event: Value| {
        if let Ok(mut seen) = sink.lock() {
            seen.push(event.clone());
        }
        forward(event);
    });
    (tee, seen)
}

pub(crate) fn last_status(seen: &[Value]) -> String {
    seen.iter()
        .rev()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("execute_reply"))
        .and_then(|event| event.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("error")
        .to_owned()
}

/// Logs `automation_result` even when the stream is cancelled.
struct ResultLog {
    manager: SessionManager,
    name: String,
    op: &'static str,
    seen: Arc<Mutex<Vec<Value>>>,
}

impl Drop for ResultLog {
    fn drop(&mut self) {
        let seen = self.seen.lock().map(|seen| seen.clone()).unwrap_or_default();
        self.manager.history().log(
            &self.name,
            "automation_result",
            json!({
                "op": self.op,
                "status": last_status(&seen),
                "outputs": collect_outputs(&seen),
            }),
        );
    }
}

/// Run `install` / `drivemount` / `gcp-auth` and stream it. Logged as
/// `automation` + `automation_result` rather than as an ordinary cell.
pub async fn run(
    manager: &SessionManager,
    name: &str,
    op: Operation,
    request: &AutomationRequest,
    emit: &Emit,
) -> Result<String> {
    let plan = plan(op, request)?;
    manager.get(name)?;
    if let Some((path, text)) = &plan.upload {
        manager.write_file(name, path, text).await?;
    }
    let mut event = json!({ "op": op.as_str(), "code": plan.code });
    if let (Some(event), Value::Object(extra)) = (event.as_object_mut(), plan.log.clone()) {
        event.extend(extra);
    }
    manager.history().log(name, "automation", event);

    emit(json!({ "type": "automation", "op": op.as_str(), "state": "started" }));
    let (tee, seen) = tee(emit);
    let status = {
        let _log = ResultLog {
            manager: manager.clone(),
            name: name.to_owned(),
            op: op.as_str(),
            seen: seen.clone(),
        };
        manager.execute(name, &plan.code, plan.timeout, false, &tee).await?;
        let seen = seen.lock().map(|seen| seen.clone()).unwrap_or_default();
        last_status(&seen)
    };
    emit(json!({ "type": "automation", "op": op.as_str(), "state": "finished", "status": status }));
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: Value) -> AutomationRequest {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn install_plans() {
        let plan =
            plan(Operation::Install, &request(json!({"packages": [" numpy ", "", "pandas==2.2"]})))
                .unwrap();
        assert!(plan.code.contains("packages = [\"numpy\", \"pandas==2.2\"]"));
        assert!(plan.code.contains("['uv', 'pip', 'install', '--system']"));
        assert!(plan.code.contains("[sys.executable, '-m', 'pip', 'install']"));
        assert_eq!(plan.timeout, INSTALL_TIMEOUT);
        assert_eq!(plan.upload, None);

        let with_file = plan_for(
            json!({"requirements": {"filename": "../../etc/reqs.txt", "content": "torch\n"}}),
        );
        assert_eq!(with_file.upload, Some(("content/reqs.txt".into(), "torch\n".into())));
        assert!(with_file.code.contains("[\"-r\", \"/content/reqs.txt\"]"));

        assert!(super::plan(Operation::Install, &request(json!({}))).is_err());
        assert!(super::plan(
            Operation::Install,
            &request(json!({"requirements": {"filename": "r.txt", "content": "  "}}))
        )
        .is_err());
    }

    fn plan_for(value: Value) -> Plan {
        plan(Operation::Install, &request(value)).unwrap()
    }

    #[test]
    fn install_code_is_valid_python_shape() {
        let code = install_code(&["a".into()], None);
        let lines: Vec<&str> = code.lines().collect();
        assert_eq!(lines[1], "import subprocess, sys");
        assert_eq!(lines[2], "def install():");
        assert_eq!(lines[3], "    packages = [\"a\"]");
        assert_eq!(lines[4], "    try:");
        assert_eq!(
            lines[5],
            "        subprocess.check_call(['uv', 'pip', 'install', '--system'] + packages)"
        );
        assert_eq!(lines[7], "    except:");
        assert_eq!(lines.last(), Some(&"install()"));
    }

    #[test]
    fn drive_and_auth_plans() {
        let default = plan(Operation::DriveMount, &request(json!({}))).unwrap();
        assert_eq!(default.code, "from google.colab import drive\ndrive.mount(\"/content/drive\")");
        assert_eq!(default.timeout, INTERACTIVE_TIMEOUT);
        let injected =
            plan(Operation::DriveMount, &request(json!({"path": "/x')\nimport os#"}))).unwrap();
        assert!(injected.code.ends_with("drive.mount(\"/x')\\nimport os#\")"));
        assert!(plan(Operation::DriveMount, &request(json!({"path": "relative"}))).is_err());
        assert!(plan(Operation::GcpAuth, &request(json!({})))
            .unwrap()
            .code
            .contains("authenticate_user()"));
        assert!(Operation::parse("rm -rf").is_err());
        assert_eq!(Operation::parse("gcp-auth").unwrap().as_str(), "gcp-auth");
    }
}
