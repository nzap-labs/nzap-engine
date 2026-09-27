//! Automations, file runs, ephemeral jobs, imports and the notebook library
//! against the mock.

mod common;

use common::Events;
use nzap_core::config::RuntimeRequest;
use nzap_core::notebooks::{Catalog, NotebookDraft, NotebookLibrary, NotebookStore};
use nzap_core::ops::automation::{self, AutomationRequest, Operation};
use nzap_core::ops::import::{import_from_url, ImportOptions};
use nzap_core::ops::jobs::{self, JobRequest};
use nzap_core::ops::runfile::{self, EnvSpec, RunFileRequest};
use nzap_core::session::SessionManager;
use nzap_mock_colab::MockFile;
use serde_json::{json, Map, Value};

async fn connected_runtime() -> (common::Connected, SessionManager) {
    let env = common::connected().await;
    let manager = env.manager();
    manager
        .create_and_connect(RuntimeRequest { name: "box".into(), ..RuntimeRequest::default() })
        .await
        .unwrap();
    (env, manager)
}

fn executed(env: &common::Connected, manager: &SessionManager) -> Vec<String> {
    let endpoint = manager.get("box").unwrap().endpoint;
    env.mock.state().runtime(&endpoint).unwrap().executed.clone()
}

// ---------------------------------------------------------------- automations

#[tokio::test(flavor = "multi_thread")]
async fn install_uploads_requirements_and_logs_the_result() {
    let (env, manager) = connected_runtime().await;
    let events = Events::default();
    let request: AutomationRequest = serde_json::from_value(json!({
        "packages": ["numpy"],
        "requirements": {"filename": "requirements.txt", "content": "torch\n"},
    }))
    .unwrap();
    let status = automation::run(&manager, "box", Operation::Install, &request, &events.emit())
        .await
        .unwrap();
    assert_eq!(status, "ok");

    let lifecycle: Vec<Value> = events.of_type("automation");
    assert_eq!(lifecycle[0], json!({"type": "automation", "op": "install", "state": "started"}));
    assert_eq!(lifecycle[1]["state"], "finished");
    assert_eq!(lifecycle[1]["status"], "ok");
    assert!(events.text().contains("Installation Complete (via uv)!"));

    let endpoint = manager.get("box").unwrap().endpoint;
    assert_eq!(
        env.mock.state().runtime(&endpoint).unwrap().files.get("content/requirements.txt"),
        Some(&MockFile::Text("torch\n".into()))
    );
    let history = manager.history().get("box");
    let automation = history.iter().find(|event| event["event_type"] == "automation").unwrap();
    assert_eq!(automation["requirements"], "/content/requirements.txt");
    let result = history.iter().find(|event| event["event_type"] == "automation_result").unwrap();
    assert_eq!(result["status"], "ok");
    assert!(!result["outputs"].as_array().unwrap().is_empty());
    // Automations are not also logged as ordinary cells.
    assert!(!history.iter().any(|event| event["event_type"] == "execution"));
}

#[tokio::test(flavor = "multi_thread")]
async fn drive_mount_and_cloud_auth() {
    let (_env, manager) = connected_runtime().await;
    let events = Events::default();
    let mount =
        AutomationRequest { path: Some("/content/gdrive".into()), ..AutomationRequest::default() };
    automation::run(&manager, "box", Operation::DriveMount, &mount, &events.emit()).await.unwrap();
    assert!(events.text().contains("Mounted at /content/drive"));
    assert!(manager.view("box").unwrap().drive_authorized);

    let events = Events::default();
    automation::run(
        &manager,
        "box",
        Operation::GcpAuth,
        &AutomationRequest::default(),
        &events.emit(),
    )
    .await
    .unwrap();
    assert!(events.text().contains("Authenticated with Google Cloud."));

    let bad = AutomationRequest { path: Some("relative".into()), ..AutomationRequest::default() };
    assert!(automation::run(
        &manager,
        "box",
        Operation::DriveMount,
        &bad,
        &Events::default().emit()
    )
    .await
    .is_err());
}

// ------------------------------------------------------------------ run-file

#[tokio::test(flavor = "multi_thread")]
async fn running_a_script_with_env() {
    let (env, manager) = connected_runtime().await;
    let events = Events::default();
    let request = RunFileRequest {
        filename: "train.py".into(),
        content: json!("#!/usr/bin/env python\nprint('training')"),
        env: EnvSpec::List(vec!["SEED=7".into()]),
        stop_on_error: false,
        timeout_seconds: None,
    };
    let done = runfile::run(&manager, "box", &request, &events.emit()).await.unwrap();
    assert_eq!(done["status"], "ok");
    assert_eq!(done["total_cells"], 1);
    assert!(done.get("notebook").is_none());
    assert_eq!(events.text(), "training\n");

    let codes = executed(&env, &manager);
    assert_eq!(codes[0], runfile::CHDIR_CONTENT);
    assert_eq!(codes[1], "import os\nos.environ[\"SEED\"] = \"7\"\nprint('training')");
}

#[tokio::test(flavor = "multi_thread")]
async fn running_a_notebook_fills_in_outputs() {
    let (_env, manager) = connected_runtime().await;
    let notebook = json!({
        "cells": [
            {"cell_type": "markdown", "source": "# Title"},
            {"cell_type": "code", "source": ["print('one')"], "outputs": []},
            {"cell_type": "code", "source": "fail()", "outputs": []},
            {"cell_type": "code", "source": "print('three')", "outputs": []},
        ],
        "metadata": {},
    });
    let request = |stop_on_error: bool| RunFileRequest {
        filename: "exp.ipynb".into(),
        content: json!(notebook.to_string()),
        env: EnvSpec::None,
        stop_on_error,
        timeout_seconds: None,
    };

    let events = Events::default();
    let done = runfile::run(&manager, "box", &request(false), &events.emit()).await.unwrap();
    assert_eq!((done["status"].clone(), done["failed_cells"].clone()), (json!("error"), json!(1)));
    assert_eq!(done["filename"], "exp_output.ipynb");
    let cells = done["notebook"]["cells"].as_array().unwrap();
    assert_eq!(cells[1]["outputs"][0]["text"], "one\n");
    assert_eq!(cells[2]["outputs"][0]["output_type"], "error");
    assert_eq!(cells[3]["outputs"][0]["text"], "three\n");
    assert!(cells[1]["execution_count"].is_number());
    assert_eq!(done["notebook"]["nbformat"], 4);
    let markers: Vec<(Value, Value)> = events
        .of_type("cell")
        .iter()
        .map(|event| (event["index"].clone(), event["state"].clone()))
        .collect();
    assert_eq!(markers.len(), 6);
    assert_eq!(markers[0], (json!(0), json!("started")));

    let done =
        runfile::run(&manager, "box", &request(true), &Events::default().emit()).await.unwrap();
    assert_eq!(done["notebook"]["cells"][3]["outputs"], json!([]), "stop_on_error skips the rest");
}

// ---------------------------------------------------------------------- jobs

#[tokio::test(flavor = "multi_thread")]
async fn a_job_runs_collects_artifacts_and_releases_the_vm() {
    let env = common::connected().await;
    env.mock
        .state()
        .new_runtime_files
        .insert("content/out/model.bin".into(), MockFile::Binary(b"\x00WEIGHTS\xff".to_vec()));
    let manager = env.manager();
    let downloads = tempfile::tempdir().unwrap();
    let spec = jobs::plan(&JobRequest {
        script: "print('train')\nimport sys\nsys.exit(3)".into(),
        args: vec!["--epochs".into(), "1".into()],
        artifacts: vec!["out/*".into()],
        gpu: Some("t4".into()),
        name: Some("train-job".into()),
        ..JobRequest::default()
    })
    .unwrap();
    let events = Events::default();
    let done = jobs::run(&manager, spec, Some(downloads.path().to_path_buf()), &events.emit())
        .await
        .unwrap();

    assert_eq!(done["exit_code"], 3);
    assert_eq!(done["released"], true);
    let phases: Vec<String> = events
        .of_type("job")
        .iter()
        .map(|event| event["phase"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(phases, vec!["assigning", "connecting", "running", "collecting", "released"]);
    assert_eq!(events.of_type("job")[0]["hardware"], "T4");
    // SystemExit is an exit code, not an error in the output.
    assert!(events.of_type("error").is_empty());
    assert_eq!(events.text(), "train\n");

    let artifact = &events.of_type("artifact")[0];
    assert_eq!(artifact["path"], "/content/out/model.bin");
    let saved = downloads.path().join("out").join("model.bin");
    assert_eq!(std::fs::read(&saved).unwrap(), b"\x00WEIGHTS\xff");
    assert_eq!(artifact["savedTo"], saved.to_string_lossy().as_ref());

    // The VM is gone and the runtime forgotten.
    assert!(manager.views().is_empty());
    assert_eq!(env.mock.state().unassigned.len(), 1);
    let history = manager.history().get("train-job");
    let result =
        history.iter().rev().find(|event| event["event_type"] == "automation_result").unwrap();
    assert_eq!(result["exit_code"], 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn kept_jobs_and_refused_allocations() {
    let env = common::connected().await;
    let manager = env.manager();
    let spec =
        jobs::plan(&JobRequest { script: "print(1)".into(), keep: true, ..JobRequest::default() })
            .unwrap();
    let name = spec.runtime.name.clone();
    let done = jobs::run(&manager, spec, None, &Events::default().emit()).await.unwrap();
    assert_eq!((done["exit_code"].clone(), done["released"].clone()), (json!(0), json!(false)));
    assert!(manager.get(&name).is_ok(), "kept jobs leave their runtime");

    env.mock.state().max_assignments = 1;
    let spec =
        jobs::plan(&JobRequest { script: "print(1)".into(), ..JobRequest::default() }).unwrap();
    let done = jobs::run(&manager, spec, None, &Events::default().emit()).await.unwrap();
    assert_eq!(done["exit_code"], 1);
    assert!(done["error"].as_str().unwrap().contains("Allocation refused"));
}

// -------------------------------------------------------------------- import

#[tokio::test(flavor = "multi_thread")]
async fn import_from_drive_and_https() {
    let env = common::connected().await;
    let notebook = json!({"cells": [], "nbformat": 4}).to_string();
    env.mock
        .state()
        .drive_files
        .insert("DriveId1".into(), ("model.ipynb".into(), notebook.clone()));
    let permissive = ImportOptions { allow_private_hosts: true };

    // Drive ids resolve against the configured Drive endpoint.
    let from_drive =
        import_from_url(&env.auth, "https://drive.google.com/file/d/DriveId1/view", permissive)
            .await
            .unwrap();
    assert_eq!((from_drive.filename.as_str(), from_drive.kind.as_str()), ("model.ipynb", "ipynb"));
    assert_eq!(from_drive.content, notebook);
    let refused =
        import_from_url(&env.auth, "https://colab.research.google.com/drive/Nope", permissive)
            .await;
    assert!(refused.unwrap_err().to_string().contains("drive.file"));

    // Anything else must be https (the mock only speaks http).
    let plain =
        import_from_url(&env.auth, &format!("{}/static/job.py", env.mock.base_url), permissive)
            .await;
    assert!(plain.unwrap_err().to_string().contains("https://"));

    // The SSRF guard refuses private targets without the test override.
    let guarded =
        import_from_url(&env.auth, "https://127.0.0.1/static/nb", ImportOptions::default())
            .await
            .unwrap_err();
    assert!(guarded.to_string().contains("private network"), "{guarded}");
}

// ----------------------------------------------------------------- notebooks

fn host_catalog(env: &common::Connected) -> String {
    let bundle: Value = serde_json::from_str(include_str!("../catalog/bundled.json")).unwrap();
    let mut state = env.mock.state();
    let mut index = bundle.clone();
    for (position, entry) in bundle["notebooks"].as_array().unwrap().iter().enumerate() {
        let path = format!("catalog/{}", entry["source"].as_str().unwrap());
        state.static_files.insert(path, entry["sourceText"].as_str().unwrap().to_owned());
        index["notebooks"][position].as_object_mut().unwrap().remove("sourceText");
    }
    state.static_files.insert("catalog/index.json".into(), index.to_string());
    format!("{}/static/catalog/", env.mock.base_url)
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_refresh_integrity_and_running_notebooks() {
    let (env, manager) = connected_runtime().await;
    let url = host_catalog(&env);
    let library = NotebookLibrary::new(
        Catalog::new(env.auth.http().clone(), &url, env.dir.path().join("catalog")),
        NotebookStore::new(env.dir.path().join("notebooks")),
    );

    let status = library.catalog().refresh().await;
    assert_eq!(serde_json::to_value(status.origin).unwrap(), "remote");
    assert_eq!(status.error, None);
    assert_eq!(status.count, 5);
    // Revalidation uses the ETag.
    library.catalog().refresh().await;
    assert_eq!(env.mock.state().static_not_modified, 1);

    // Sources are downloaded once, verified, then served from the cache.
    let print = library.get("public:print-notebook").await.unwrap();
    assert!(print.source.unwrap().contains("string_to_print"));
    library.get("public:print-notebook").await.unwrap();
    assert_eq!(env.mock.state().static_hits["catalog/notebooks/print-notebook/notebook.py"], 1);

    // A tampered copy of a bundled notebook falls back to the verified bundle…
    env.mock.state().static_files.insert(
        "catalog/notebooks/gpu-check/notebook.py".into(),
        "import os; os.system('evil')\n".into(),
    );
    let gpu = library.get("public:gpu-check").await.unwrap();
    assert!(!gpu.source.unwrap().contains("evil"));

    // …and one the app does not ship is refused outright.
    {
        let mut state = env.mock.state();
        let mut index: Value =
            serde_json::from_str(&state.static_files["catalog/index.json"]).unwrap();
        index["notebooks"].as_array_mut().unwrap().push(json!({
            "slug": "extra-one",
            "title": "Extra",
            "source": "notebooks/extra-one/notebook.py",
            "sha256": nzap_core::notebooks::catalog::sha256_hex(b"print(1)\n"),
            "params": [],
        }));
        state.static_files.insert("catalog/index.json".into(), index.to_string());
        state
            .static_files
            .insert("catalog/notebooks/extra-one/notebook.py".into(), "print(2)\n".into());
    }
    assert_eq!(library.catalog().refresh().await.count, 6);
    let tampered = library.get("public:extra-one").await.unwrap_err();
    assert!(tampered.to_string().contains("integrity"), "{tampered}");

    // Run a public notebook with parameters.
    let events = Events::default();
    let mut values = Map::new();
    values.insert("string_to_print".into(), json!("Hi from the test"));
    let reply = library
        .run(&manager, "public:print-notebook", "box", &values, &events.emit())
        .await
        .unwrap();
    assert_eq!(reply["status"], "ok");
    assert_eq!(events.text(), "Hi from the test\n");
    let code = executed(&env, &manager).last().unwrap().clone();
    assert!(code.starts_with("# Injected by NZAP Engine"));
    assert!(code.contains("params = _nzap_json.loads("));

    // Parameter validation happens before anything runs.
    let mut bad = Map::new();
    bad.insert("string_to_print".into(), json!(""));
    let local = library
        .create(NotebookDraft {
            slug: "needs-count".into(),
            title: "Needs count".into(),
            source: "print(params['n'])".into(),
            params: serde_json::from_value(
                json!([{"key": "n", "label": "Count", "type": "integer", "required": true}]),
            )
            .unwrap(),
            ..NotebookDraft::default()
        })
        .unwrap();
    let before = executed(&env, &manager).len();
    let error =
        library.run(&manager, &local.id, "box", &bad, &Events::default().emit()).await.unwrap_err();
    assert_eq!(error.to_string(), "Count is required.");
    assert_eq!(executed(&env, &manager).len(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_catalog_falls_back_to_the_bundle() {
    let env = common::connected().await;
    let library = NotebookLibrary::new(
        Catalog::new(
            env.auth.http().clone(),
            &format!("{}/static/missing/", env.mock.base_url),
            env.dir.path().join("c"),
        ),
        NotebookStore::new(env.dir.path().join("n")),
    );
    let status = library.catalog().refresh().await;
    assert_eq!(serde_json::to_value(status.origin).unwrap(), "bundled");
    assert!(status.error.unwrap().contains("404"));
    assert!(library.get("public:print-notebook").await.is_ok());
}
