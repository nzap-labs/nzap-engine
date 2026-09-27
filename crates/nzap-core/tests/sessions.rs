//! The session manager end to end against the mock: allocation, kernel
//! sockets, streaming execution, stdin, Drive/Cloud consent pause & resume,
//! interrupts, restarts, files, history, persistence, adoption, release and
//! the terminal.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{wait_until, Events};
use nzap_core::config::RuntimeRequest;
use nzap_core::runtime::TerminalEvent;
use nzap_core::session::{SessionManager, DEFAULT_EXECUTE_TIMEOUT};
use nzap_core::Error;
use serde_json::{json, Value};

fn request(name: &str) -> RuntimeRequest {
    RuntimeRequest { name: name.into(), ..RuntimeRequest::default() }
}

async fn run(manager: &SessionManager, name: &str, code: &str) -> (Value, Events) {
    let events = Events::default();
    let reply =
        manager.execute(name, code, DEFAULT_EXECUTE_TIMEOUT, true, &events.emit()).await.unwrap();
    (reply, events)
}

fn history_types(manager: &SessionManager, name: &str) -> Vec<String> {
    manager
        .history()
        .get(name)
        .iter()
        .map(|event| event["event_type"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn create_connect_and_run_cells() {
    let env = common::connected().await;
    let manager = env.manager();
    let (view, connected) = manager.create_and_connect(request("box")).await.unwrap();
    assert!(connected);
    assert_eq!((view.name.as_str(), view.accelerator.as_str()), ("box", "CPU"));
    assert!(view.connected);
    assert!(view.kernel_id.is_some());

    // The VM-side session is named after the runtime (Colab's "Manage sessions").
    {
        let state = env.mock.state();
        let runtime = state.runtime(&view.endpoint).unwrap();
        assert_eq!(runtime.sessions[0]["name"], "box");
        assert_eq!(runtime.sessions[0]["path"], "box.ipynb");
    }

    let (reply, events) = run(&manager, "box", "print(\"hello\")\nanswer").await;
    assert_eq!(reply, json!({"type": "execute_reply", "status": "ok", "execution_count": 1}));
    assert_eq!(events.text(), "hello\n");
    assert_eq!(events.of_type("result")[0]["data"]["text/plain"], "42");
    assert_eq!(events.of_type("input")[0]["execution_count"], 1);
    let states: Vec<Value> = events.of_type("status").iter().map(|e| e["state"].clone()).collect();
    assert_eq!(states, vec![json!("busy"), json!("idle")]);
    assert_eq!(events.all().last().unwrap()["type"], "execute_reply");

    let (reply, events) = run(&manager, "box", "fail()").await;
    assert_eq!(reply["status"], "error");
    assert_eq!(events.of_type("error")[0]["ename"], "ValueError");

    // Both cells are in the history, with their outputs.
    let history = manager.history().get("box");
    let executions: Vec<&Value> =
        history.iter().filter(|event| event["event_type"] == "execution").collect();
    assert_eq!(executions.len(), 2);
    assert_eq!(executions[0]["status"], "ok");
    assert_eq!(executions[0]["outputs"][0]["text"], "hello\n");
    assert_eq!(executions[1]["status"], "error");
    assert_eq!(history_types(&manager, "box")[0], "session_created");

    // The kernel state is visible while connected.
    assert_eq!(manager.view("box").unwrap().kernel_state.as_deref(), Some("idle"));
    assert!(matches!(
        manager.execute("box", "  ", DEFAULT_EXECUTE_TIMEOUT, true, &Events::default().emit()).await,
        Err(Error::InvalidInput(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn input_prompts_are_answered_through_stdin() {
    let env = common::connected().await;
    let manager = env.manager();
    manager.create_and_connect(request("box")).await.unwrap();

    let events = Events::default();
    let cell = {
        let manager = manager.clone();
        let emit = events.emit();
        tokio::spawn(async move {
            manager.execute("box", "name = input(\"Name? \")", DEFAULT_EXECUTE_TIMEOUT, true, &emit).await
        })
    };
    wait_until("input_request", || !events.of_type("input_request").is_empty()).await;
    assert_eq!(events.of_type("input_request")[0]["prompt"], "Name? ");
    manager.send_stdin("box", "Ada").await.unwrap();

    let reply = cell.await.unwrap().unwrap();
    assert_eq!(reply["status"], "ok");
    assert_eq!(events.text(), "Hello, Ada!\n");
    assert!(history_types(&manager, "box").contains(&"input_reply".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn drive_mount_with_existing_consent_propagates_and_resumes() {
    let env = common::connected().await;
    let manager = env.manager();
    manager.create_and_connect(request("box")).await.unwrap();

    let (reply, events) = run(&manager, "box", "from google.colab import drive\ndrive.mount('/content/drive')").await;
    assert_eq!(reply["status"], "ok");
    let notes: Vec<String> = events
        .of_type("colab_request")
        .iter()
        .map(|event| event["message"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(notes.len(), 2);
    assert!(notes[0].contains("Google Drive authorization requested"));
    assert!(notes[1].contains("propagated"));
    assert_eq!(events.text(), "Mounted at /content/drive\n");
    assert!(manager.view("box").unwrap().drive_authorized);
    assert_eq!(env.mock.state().propagations.len(), 2);
    let types = history_types(&manager, "box");
    assert!(types.contains(&"colab_request".to_owned()));
    assert!(types.contains(&"drive_auth_success".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_consent_pauses_the_cell_until_authorized() {
    let env = common::connected().await;
    env.mock.state().drive_consent = false;
    let manager = env.manager();
    manager.create_and_connect(request("box")).await.unwrap();

    let events = Events::default();
    let cell = {
        let manager = manager.clone();
        let emit = events.emit();
        tokio::spawn(async move {
            manager
                .execute("box", "from google.colab import auth\nauth.authenticate_user()", DEFAULT_EXECUTE_TIMEOUT, true, &emit)
                .await
        })
    };
    wait_until("drive_auth_required", || !events.of_type("drive_auth_required").is_empty()).await;
    let required = &events.of_type("drive_auth_required")[0];
    assert_eq!(required["auth_type"], "auth_user_ephemeral");
    assert_eq!(required["uri"], "https://accounts.google.com/o/oauth2/consent?x=1&y=2");
    assert_eq!(
        manager.view("box").unwrap().drive_pending_uri.as_deref(),
        Some("https://accounts.google.com/o/oauth2/consent?x=1&y=2")
    );
    assert!(!cell.is_finished(), "the cell stays paused while waiting");

    // Still no consent: the retry reports the link again.
    let retry = manager.authorize_drive("box").await.unwrap();
    assert_eq!(retry["success"], false);

    env.mock.state().drive_consent = true;
    let granted = manager.authorize_drive("box").await.unwrap();
    assert_eq!(granted, json!({"success": true, "resumed": true}));
    let reply = cell.await.unwrap().unwrap();
    assert_eq!(reply["status"], "ok");
    assert_eq!(events.text(), "Authenticated with Google Cloud.\n");
    let view = manager.view("box").unwrap();
    assert_eq!(view.drive_pending_uri, None);
    // Cloud auth does not mark Drive as authorised.
    assert!(!view.drive_authorized);
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupting_a_running_cell() {
    let env = common::connected().await;
    let manager = env.manager();
    manager.create_and_connect(request("box")).await.unwrap();

    let events = Events::default();
    let cell = {
        let manager = manager.clone();
        let emit = events.emit();
        tokio::spawn(async move {
            manager.execute("box", "wait_for_interrupt()", DEFAULT_EXECUTE_TIMEOUT, true, &emit).await
        })
    };
    wait_until("busy", || !events.of_type("status").is_empty()).await;
    manager.interrupt("box").await.unwrap();
    let reply = cell.await.unwrap().unwrap();
    assert_eq!(reply["status"], "error");
    assert_eq!(events.of_type("error")[0]["ename"], "KeyboardInterrupt");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_stream_is_recorded_as_interrupted() {
    let env = common::connected().await;
    let manager = env.manager();
    manager.create_and_connect(request("box")).await.unwrap();

    let events = Events::default();
    let cell = {
        let manager = manager.clone();
        let emit = events.emit();
        tokio::spawn(async move {
            manager.execute("box", "wait_for_interrupt()", DEFAULT_EXECUTE_TIMEOUT, true, &emit).await
        })
    };
    wait_until("busy", || !events.of_type("status").is_empty()).await;
    cell.abort();
    let _ = cell.await;
    let history = manager.history().get("box");
    let last = history.iter().rev().find(|event| event["event_type"] == "execution").unwrap();
    assert_eq!(last["status"], "interrupted");
    manager.interrupt("box").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_and_shutdown_recover_on_the_next_cell() {
    let env = common::connected().await;
    let manager = env.manager();
    let (view, _) = manager.create_and_connect(request("box")).await.unwrap();
    let first_kernel = view.kernel_id.unwrap();

    let restarted = manager.restart_kernel("box").await.unwrap();
    assert_eq!(restarted, json!({"id": first_kernel, "restarted": true}));
    assert!(!manager.view("box").unwrap().connected);
    let (reply, _) = run(&manager, "box", "print('again')").await;
    assert_eq!(reply["status"], "ok");

    manager.shutdown_kernel("box").await.unwrap();
    assert_eq!(manager.view("box").unwrap().kernel_id, None);
    // The next cell starts a fresh kernel.
    let (reply, events) = run(&manager, "box", "print('fresh')").await;
    assert_eq!(reply["status"], "ok");
    assert_eq!(events.text(), "fresh\n");
    assert_ne!(manager.view("box").unwrap().kernel_id.unwrap(), first_kernel);

    manager.disconnect("box").unwrap();
    assert!(!manager.view("box").unwrap().connected);
}

#[tokio::test(flavor = "multi_thread")]
async fn files_through_the_manager_are_logged() {
    let env = common::connected().await;
    let manager = env.manager();
    manager.create(request("box")).await.unwrap();

    manager.write_file("box", "content/a.py", "print(1)").await.unwrap();
    manager.upload_file("box", "content/b.bin", &[0, 1, 2]).await.unwrap();
    manager.make_directory("box", "content/dir").await.unwrap();
    let listing = manager.list_files("box", "content").await.unwrap();
    let names: Vec<(&str, &str)> =
        listing.entries.iter().map(|entry| (entry.kind.as_str(), entry.name.as_str())).collect();
    assert_eq!(
        names,
        vec![("directory", "dir"), ("directory", "sample_data"), ("file", "a.py"), ("file", "b.bin")]
    );
    assert_eq!(listing.entries[2].size, Some(8));
    assert_eq!(manager.read_file("box", "content/a.py").await.unwrap()["content"], "print(1)");
    assert_eq!(manager.download_file("box", "content/b.bin").await.unwrap(), vec![0, 1, 2]);
    manager.rename_file("box", "content/a.py", "content/c.py").await.unwrap();
    manager.delete_file("box", "content/c.py").await.unwrap();
    assert!(manager.write_file("box", "  ", "x").await.is_err());

    let ops: Vec<String> = manager
        .history()
        .get("box")
        .iter()
        .filter(|event| event["event_type"] == "file_operation")
        .map(|event| event["op"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ops, vec!["write", "upload", "mkdir", "rename", "rm"]);

    let resources = manager.resources("box").await.unwrap();
    assert!(resources.ram.unwrap().percent.unwrap() > 0.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn keep_alive_runs_in_the_background() {
    let env = common::connected().await;
    let manager = env.manager();
    manager.set_keepalive(true, Duration::from_millis(50));
    let view = manager.create(request("box")).await.unwrap();
    wait_until("three keep-alives", || {
        env.mock.state().keepalives.get(&view.endpoint).copied().unwrap_or_default() >= 3
    })
    .await;
    assert!(manager.view("box").unwrap().last_keepalive.is_some());

    manager.set_keepalive(false, Duration::from_millis(50));
    let count = env.mock.state().keepalives[&view.endpoint];
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(env.mock.state().keepalives[&view.endpoint] <= count + 1);

    let ping = manager.keepalive("box").await.unwrap();
    assert_eq!(ping["ok"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_releases_and_forgets() {
    let env = common::connected().await;
    let manager = env.manager();
    let view = manager.create(request("box")).await.unwrap();
    let outcome = manager.stop("box", true).await.unwrap();
    assert!(outcome.released);
    assert_eq!(outcome.warning, None);
    assert_eq!(env.mock.state().unassigned, vec![view.endpoint.clone()]);
    assert!(manager.views().is_empty());
    assert!(matches!(manager.get("box"), Err(Error::NotFound(_))));
    assert!(history_types(&manager, "box").contains(&"session_terminated".to_owned()));
    assert!(matches!(manager.stop("box", true).await, Err(Error::NotFound(_))));

    // Names are validated and unique.
    assert!(matches!(manager.create(request("bad name")).await, Err(Error::InvalidInput(_))));
    manager.create(request("twin")).await.unwrap();
    assert!(matches!(manager.create(request("twin")).await, Err(Error::InvalidInput(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn runtimes_survive_a_restart_and_expired_ones_are_dropped() {
    let env = common::connected().await;
    let first = env.manager();
    first.create(request("keep")).await.unwrap();
    let gone = first.create(request("gone")).await.unwrap();
    first.shutdown();
    drop(first);

    // The VM behind "gone" was released elsewhere (or timed out).
    env.mock.state().assignments.retain(|assignment| assignment.endpoint != gone.endpoint);

    let restarted = env.manager();
    assert_eq!(restarted.names(), vec!["gone".to_owned(), "keep".to_owned()]);
    restarted.resume().await.unwrap();
    assert_eq!(restarted.names(), vec!["keep".to_owned()]);
    let (reply, _) = run(&restarted, "keep", "print('still here')").await;
    assert_eq!(reply["status"], "ok");

    // The proxy token is persisted but never part of a view.
    let stored = std::fs::read_to_string(env.dir.path().join("sessions.json")).unwrap();
    assert!(stored.contains("proxy-token-"));
    let view = serde_json::to_string(&restarted.view("keep").unwrap()).unwrap();
    assert!(!view.contains("proxy-token-"));
}

#[tokio::test(flavor = "multi_thread")]
async fn adopt_and_release_external_runtimes() {
    let env = common::connected().await;
    let manager = env.manager();
    let external = env.mock.state().add_assignment("T4", 1, 0);

    let listed = manager.server_assignments().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].accelerator.as_str(), listed[0].managed), ("T4", false));

    let adopted = manager.adopt(&external.endpoint, None).await.unwrap();
    assert!(adopted.name.starts_with("imported-"));
    assert_eq!(adopted.accelerator, "T4");
    assert!(manager.server_assignments().await.unwrap()[0].managed);
    let again = manager.adopt(&external.endpoint, Some("other")).await.unwrap_err();
    assert!(again.to_string().contains("already managed"), "{again}");
    assert!(matches!(manager.adopt("m-s-missing", Some("x")).await, Err(Error::NotFound(_))));

    manager.release_endpoint(&external.endpoint).await.unwrap();
    assert!(manager.views().is_empty());
    assert!(env.mock.state().assignments.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn terminal_relays_frames() {
    let env = common::connected().await;
    let manager = env.manager();
    manager.create(request("box")).await.unwrap();

    let frames = Arc::new(std::sync::Mutex::new(Vec::<TerminalEvent>::new()));
    let sink = {
        let frames = frames.clone();
        Arc::new(move |event| frames.lock().unwrap().push(event))
    };
    let terminal = manager.open_terminal("box", sink).await.unwrap();
    let text = || -> String {
        frames
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                TerminalEvent::Frame(frame) => serde_json::from_str::<Value>(frame)
                    .ok()
                    .and_then(|value| value["data"].as_str().map(str::to_owned)),
                TerminalEvent::Closed(_) => None,
            })
            .collect()
    };
    wait_until("banner", || text().contains("root@mock")).await;

    terminal.send(r#"{"cols": 120, "rows": 40}"#).unwrap();
    terminal.send(r#"{"data": "whoami\r"}"#).unwrap();
    wait_until("whoami output", || text().contains("\r\nroot\r\n")).await;
    assert!(terminal.send(r#"{"data": 1}"#).is_err());
    {
        let state = env.mock.state();
        assert_eq!(state.tty_commands, vec!["whoami".to_owned()]);
        assert_eq!(state.tty_resizes[0].1, 120);
        // Header auth only: the token never went into the URL.
        let upgrade = state.requests_to("/colab/tty").remove(0);
        assert_eq!(upgrade.param("colab-runtime-proxy-token"), None);
        assert!(upgrade.header("x-colab-runtime-proxy-token").is_some());
    }
    assert!(history_types(&manager, "box").contains(&"console_started".to_owned()));

    terminal.close();
    assert!(manager.open_terminal("missing", Arc::new(|_: TerminalEvent| {})).await.is_err());
}
