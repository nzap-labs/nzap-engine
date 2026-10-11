//! GitHub Codespaces and CodeSandbox runtimes end to end against the mock:
//! sign-in, create (wait for the VM and its Jupyter Server), run cells,
//! files, a terminal, waking a stopped VM, deleting on request and when
//! idle, and jobs.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::Events;
use nzap_core::auth::OAuthClient;
use nzap_core::config::{Endpoints, Provider, RuntimeRequest};
use nzap_core::ops::jobs::{self, JobRequest};
use nzap_core::paths::AppPaths;
use nzap_core::runtime::{TerminalEvent, TerminalSink};
use nzap_core::{Engine, EngineOptions, Error};
use nzap_mock_colab::MockGoogle;

struct Env {
    mock: MockGoogle,
    engine: Engine,
    _dir: tempfile::TempDir,
}

fn env_with(mock: MockGoogle) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(EngineOptions {
        paths: AppPaths::under(dir.path()),
        endpoints: Endpoints::single_host(&mock.base_url),
        use_keychain: false,
        oauth_client: Some(OAuthClient { client_id: "test-client".into(), client_secret: None }),
        sessions_file: None,
    })
    .unwrap();
    engine.providers.set_timing(Duration::from_millis(10), Duration::from_secs(5));
    Env { mock, engine, _dir: dir }
}

async fn env() -> Env {
    env_with(MockGoogle::start().await)
}

fn request(name: &str, provider: Provider, machine: Option<&str>) -> RuntimeRequest {
    RuntimeRequest {
        name: name.into(),
        provider,
        machine: machine.map(str::to_owned),
        ..RuntimeRequest::default()
    }
}

async fn run(engine: &Engine, name: &str, code: &str) -> String {
    let events = Events::default();
    engine
        .sessions
        .execute(name, code, Duration::from_secs(10), true, &events.emit())
        .await
        .unwrap();
    events.text()
}

// ------------------------------------------------------------ Codespaces

#[tokio::test(flavor = "multi_thread")]
async fn codespaces_sign_in_with_a_token_the_cli_or_the_device_flow() {
    let env = env().await;
    let codespaces = &env.engine.providers.codespaces;
    let refused =
        env.engine.sessions.create(request("cs", Provider::Codespaces, None)).await.unwrap_err();
    assert!(refused.to_string().contains("Connect GitHub"), "{refused}");

    assert!(matches!(
        codespaces.set_token("nope").await,
        Err(Error::Provider { status: Some(401), .. })
    ));
    // A classic token without the codespace scope is refused up front.
    env.mock.state().github_scopes = Some("repo, gist".into());
    let token = env.mock.state().github_token.clone();
    let scope = codespaces.set_token(&token).await.unwrap_err();
    assert!(scope.to_string().contains("codespace"), "{scope}");
    assert!(codespaces.token().is_none());
    // Fine-grained tokens list no scopes and are accepted.
    env.mock.state().github_scopes = None;
    assert_eq!(codespaces.set_token(&token).await.unwrap().login, "ada");
    codespaces.disconnect().unwrap();

    // GitHub's device flow: pending once, then the token.
    let login = codespaces.begin_device("Iv1.nzap").await.unwrap();
    assert_eq!(login.user_code, "WDJB-MJHT");
    assert!(login.verification_uri.ends_with("/login/device"));
    assert_eq!(codespaces.finish_device(&login).await.unwrap().login, "ada");
    assert_eq!(codespaces.token().as_deref(), Some(token.as_str()));

    let statuses = env.engine.providers.status().await;
    let github = statuses.iter().find(|status| status.provider == Provider::Codespaces).unwrap();
    assert!(github.connected);
    assert_eq!(github.account.as_deref(), Some("ada"));
    let machines = env.engine.providers.machines(Provider::Codespaces).await.unwrap();
    assert_eq!(
        machines.iter().map(|machine| machine.name.as_str()).collect::<Vec<_>>(),
        ["basicLinux32gb", "standardLinux32gb"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_codespace_runs_cells_files_and_a_terminal_wakes_up_and_is_deleted() {
    let env = env().await;
    let token = env.mock.state().github_token.clone();
    env.engine.providers.codespaces.set_token(&token).await.unwrap();
    let sessions = &env.engine.sessions;

    let view = sessions
        .create(request("gpu-less", Provider::Codespaces, Some("standardLinux32gb")))
        .await
        .unwrap();
    assert_eq!(view.provider, Provider::Codespaces);
    assert_eq!(view.accelerator, "standardLinux32gb");
    assert!(view.colab_url.is_none());
    assert!(view.web_url.unwrap().contains("/codespaces/"));
    let codespace = env.mock.state().codespaces.values().next().unwrap().clone();
    assert_eq!(codespace.repository, "nzap-labs/nzap-runtime");
    assert_eq!(codespace.devcontainer_path, ".devcontainer/devcontainer.json");
    assert_eq!(codespace.machine, "standardLinux32gb");
    // GitHub itself deletes it after the auto-release period.
    assert_eq!(codespace.retention_period_minutes, Some(30));

    assert_eq!(run(&env.engine, "gpu-less", "print(\"on a codespace\")").await, "on a codespace\n");
    sessions.upload_file("gpu-less", "content/in.wav", b"RIFF").await.unwrap();
    let listing = sessions.list_files("gpu-less", "content").await.unwrap();
    assert!(listing.entries.iter().any(|entry| entry.name == "in.wav"));

    // Jupyter terminals speak terminado; the UI still sees `{"data": …}`.
    let frames = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink: TerminalSink = {
        let frames = frames.clone();
        Arc::new(move |event| {
            if let TerminalEvent::Frame(frame) = event {
                frames.lock().unwrap().push(frame);
            }
        })
    };
    let terminal = sessions.open_terminal("gpu-less", sink).await.unwrap();
    terminal.send(r#"{"data": "whoami\r"}"#).unwrap();
    common::wait_until("terminal output", || {
        frames.lock().unwrap().iter().any(|frame| frame.contains("root\\r\\n"))
    })
    .await;
    terminal.send(r#"{"cols": 100, "rows": 30}"#).unwrap();
    common::wait_until("resize", || !env.mock.state().tty_resizes.is_empty()).await;
    assert_eq!(env.mock.state().tty_resizes[0].1, 100);
    terminal.close();

    // GitHub stopped it for inactivity: the next cell starts it again.
    sessions.disconnect("gpu-less").unwrap();
    env.mock.state().codespaces.get_mut(&codespace.name).unwrap().state = "Shutdown".into();
    assert_eq!(run(&env.engine, "gpu-less", "print(\"back\")").await, "back\n");
    assert!(env.mock.state().requests.iter().any(|request| request.path.ends_with("/start")));

    // Releasing deletes it.
    let outcome = sessions.stop("gpu-less", true).await.unwrap();
    assert!(outcome.released);
    assert!(env.mock.state().codespaces.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_codespace_that_never_serves_jupyter_is_deleted() {
    let env = env().await;
    let token = env.mock.state().github_token.clone();
    env.engine.providers.codespaces.set_token(&token).await.unwrap();
    env.engine.providers.set_timing(Duration::from_millis(10), Duration::from_millis(200));
    // The codespace never leaves "Provisioning".
    env.mock.state().codespace_boot_polls = 10_000;
    let error =
        env.engine.sessions.create(request("slow", Provider::Codespaces, None)).await.unwrap_err();
    assert!(error.to_string().contains("in time"), "{error}");
    assert!(env.mock.state().codespaces.is_empty(), "no orphaned codespace");
    assert!(env.engine.sessions.names().is_empty());

    env.mock.state().codespace_repositories.clear();
    let missing =
        env.engine.sessions.create(request("x", Provider::Codespaces, None)).await.unwrap_err();
    assert!(missing.to_string().contains("runtime repository"), "{missing}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_on_a_codespace_returns_artifacts_and_deletes_it() {
    let env = env().await;
    let token = env.mock.state().github_token.clone();
    env.engine.providers.codespaces.set_token(&token).await.unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let input = downloads.path().join("talk.mp4");
    std::fs::write(&input, b"video").unwrap();
    let mut spec = jobs::plan(&JobRequest {
        script: "print('converted')".into(),
        artifacts: vec!["out/*".into()],
        provider: Provider::Codespaces,
        machine: Some("basicLinux32gb".into()),
        name: Some("ffmpeg-job".into()),
        ..JobRequest::default()
    })
    .unwrap();
    spec.inputs = vec![jobs::JobInput { local: input, remote: "out/talk.mp4".into() }];
    let events = Events::default();
    let done =
        jobs::run(&env.engine.sessions, spec, Some(downloads.path().join("out")), &events.emit())
            .await
            .unwrap();
    assert_eq!(done["exit_code"], 0, "{done}");
    assert_eq!(done["released"], true);
    assert!(events.text().contains("converted"));
    assert_eq!(std::fs::read(downloads.path().join("out/out/talk.mp4")).unwrap(), b"video");
    assert!(env.mock.state().codespaces.is_empty());
}

// ------------------------------------------------------------ CodeSandbox

#[tokio::test(flavor = "multi_thread")]
async fn a_sandbox_boots_jupyter_through_its_agent_and_survives_hibernation_and_reboots() {
    let env = env().await;
    let providers = &env.engine.providers;
    assert!(providers.codesandbox.set_api_key("csb_wrong").await.is_err());
    let key = env.mock.state().csb_api_key.clone();
    let running = providers.codesandbox.set_api_key(&key).await.unwrap();
    assert_eq!(running["concurrent_vm_limit"], 10);
    let sessions = &env.engine.sessions;

    let view = sessions.create(request("csb", Provider::Codesandbox, Some("Micro"))).await.unwrap();
    assert_eq!(view.provider, Provider::Codesandbox);
    assert_eq!(view.accelerator, "Micro");
    let sandbox = env.mock.state().sandboxes.values().next().unwrap().clone();
    assert_eq!(sandbox.template, "pcz35m");
    assert_eq!(sandbox.tier.as_deref(), Some("Micro"));
    assert!(sandbox.jupyter_running);
    assert_eq!(sandbox.commands.len(), 1);
    assert!(sandbox.commands[0].starts_with("bash -c '(mkdir -p /content"));
    let jupyter_token = sandbox.jupyter_token.clone().unwrap();
    assert_eq!(jupyter_token.len(), 64);
    // The token never reaches the UI.
    assert!(!serde_json::to_string(&sessions.views()).unwrap().contains(&jupyter_token));

    assert_eq!(run(&env.engine, "csb", "print(\"on a sandbox\")").await, "on a sandbox\n");

    // Hibernated: the next request wakes it, Jupyter is still there.
    sessions.disconnect("csb").unwrap();
    env.mock.state().sandboxes.get_mut(&sandbox.id).unwrap().hibernated = true;
    assert_eq!(run(&env.engine, "csb", "print(\"awake\")").await, "awake\n");
    assert_eq!(env.mock.state().sandboxes[&sandbox.id].commands.len(), 1);

    // Shut down: a clean boot, and Jupyter is started again with the same token.
    sessions.disconnect("csb").unwrap();
    {
        let mut mock = env.mock.state();
        let sandbox = mock.sandboxes.get_mut(&sandbox.id).unwrap();
        sandbox.running = false;
        sandbox.jupyter_running = false;
    }
    assert_eq!(run(&env.engine, "csb", "print(\"rebooted\")").await, "rebooted\n");
    let rebooted = env.mock.state().sandboxes[&sandbox.id].clone();
    assert_eq!(rebooted.starts, 2);
    assert_eq!(rebooted.commands.len(), 2);
    assert_eq!(rebooted.jupyter_token.as_deref(), Some(jupyter_token.as_str()));

    assert!(sessions.resources("csb").await.is_err(), "Colab-only telemetry");
    assert!(sessions.stop("csb", true).await.unwrap().released);
    assert!(env.mock.state().sandboxes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_provider_runtimes_are_deleted_automatically_but_colab_ones_are_not() {
    let env = env().await;
    let key = env.mock.state().csb_api_key.clone();
    env.engine.providers.codesandbox.set_api_key(&key).await.unwrap();
    let sessions = &env.engine.sessions;
    sessions.create(request("warm", Provider::Codesandbox, None)).await.unwrap();
    assert_eq!(sessions.names(), ["warm"]);

    // Still warm: nothing to reap.
    env.engine.providers.override_auto_release(Some(Duration::from_secs(3600)));
    assert!(sessions.reap().await.is_empty());
    // Auto-release off: nothing either.
    env.engine.providers.override_auto_release(None);
    assert!(sessions.reap().await.is_empty());

    env.engine.providers.override_auto_release(Some(Duration::from_millis(1)));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(sessions.reap().await, ["warm"]);
    assert!(sessions.names().is_empty());
    assert!(env.mock.state().sandboxes.is_empty());
    let history = sessions.history().get("warm");
    assert!(history.iter().any(|event| event["reason"] == "idle_auto_release"));

    // The background reaper does the same on its own.
    env.engine.providers.override_auto_release(Some(Duration::from_secs(3600)));
    sessions.set_reaper_interval(Duration::from_millis(20));
    sessions.create(request("later", Provider::Codesandbox, None)).await.unwrap();
    env.engine.providers.override_auto_release(Some(Duration::from_millis(1)));
    common::wait_until("the reaper", || env.mock.state().sandboxes.is_empty()).await;
    assert!(sessions.names().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sandbox_that_fails_to_boot_is_deleted_and_bad_templates_are_reported() {
    let env = env().await;
    let key = env.mock.state().csb_api_key.clone();
    env.engine.providers.codesandbox.set_api_key(&key).await.unwrap();
    let sessions = &env.engine.sessions;
    assert!(sessions.create(request("x", Provider::Codesandbox, Some("Gigantic"))).await.is_err());

    env.mock.state().csb_templates.clear();
    let missing = sessions.create(request("x", Provider::Codesandbox, None)).await.unwrap_err();
    assert!(matches!(missing, Error::Provider { status: Some(404), .. }), "{missing}");
    assert!(sessions.names().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_runtimes_survive_a_restart_unless_deleted_elsewhere() {
    let mock = MockGoogle::start().await;
    let dir = tempfile::tempdir().unwrap();
    let options = || EngineOptions {
        paths: AppPaths::under(dir.path()),
        endpoints: Endpoints::single_host(&mock.base_url),
        use_keychain: false,
        oauth_client: Some(OAuthClient { client_id: "test-client".into(), client_secret: None }),
        sessions_file: None,
    };
    let engine = Engine::new(options()).unwrap();
    engine.providers.set_timing(Duration::from_millis(10), Duration::from_secs(5));
    let token = mock.state().github_token.clone();
    engine.providers.codespaces.set_token(&token).await.unwrap();
    let key = mock.state().csb_api_key.clone();
    engine.providers.codesandbox.set_api_key(&key).await.unwrap();
    engine.sessions.create(request("kept", Provider::Codespaces, None)).await.unwrap();
    engine.sessions.create(request("sandbox", Provider::Codesandbox, None)).await.unwrap();
    let gone = engine.sessions.create(request("gone", Provider::Codespaces, None)).await.unwrap();
    engine.shutdown();

    // Deleted on GitHub while the app was closed.
    mock.state().codespaces.remove(&gone.endpoint);
    let restarted = Engine::new(options()).unwrap();
    restarted.providers.set_timing(Duration::from_millis(10), Duration::from_secs(5));
    // No Google connection: provider runtimes still resume.
    restarted.resume().await;
    assert_eq!(restarted.sessions.names(), ["kept", "sandbox"]);
    assert_eq!(run(&restarted, "kept", "print(\"resumed\")").await, "resumed\n");
    // Disconnecting Google leaves them alone.
    restarted.disconnect().await.unwrap();
    assert_eq!(restarted.sessions.names(), ["kept", "sandbox"]);
}
