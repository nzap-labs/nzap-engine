//! The MCP server end to end: a client speaking JSON-RPC lines to `serve`,
//! an engine connected to the mock Google, and a temporary project folder.

use std::path::PathBuf;
use std::sync::Arc;

use nzap_core::config::Endpoints;
use nzap_core::notebooks::{NotebookDraft, NotebookParam, ParamType};
use nzap_core::paths::AppPaths;
use nzap_core::secrets::{FileStore, SecretStore};
use nzap_core::{auth::OAuthClient, Engine, EngineOptions};
use nzap_mcp::{serve, Options, Server};
use nzap_mock_colab::{MockFile, MockGoogle};
use serde_json::{json, Value};
use tokio::io::{
    AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};

/// What the app stores when the user clicks Connect Google.
fn connect_in_app(mock: &MockGoogle, paths: &AppPaths) {
    let token = mock.state().grant_refresh_token();
    FileStore::new(paths.secrets_fallback_file()).set("google-refresh-token", &token).unwrap();
    let identity =
        json!({"user": {"sub": "1", "email": "ada@example.com", "name": "Ada Lovelace"}});
    std::fs::write(paths.identity_file(), identity.to_string()).unwrap();
}

struct Client {
    mock: MockGoogle,
    _dir: tempfile::TempDir,
    paths: AppPaths,
    project: PathBuf,
    server: Arc<Server>,
    writer: Option<WriteHalf<DuplexStream>>,
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
    notifications: Vec<Value>,
    next_id: u64,
    serving: tokio::task::JoinHandle<std::io::Result<()>>,
}

async fn client(args: &[&str]) -> Client {
    client_with(args, true).await
}

async fn client_with(args: &[&str], connected: bool) -> Client {
    let mock = MockGoogle::start().await;
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(&dir.path().join("app"));
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    if connected {
        connect_in_app(&mock, &paths);
    }
    let engine = Engine::new(EngineOptions {
        paths: paths.clone(),
        endpoints: Endpoints::single_host(&mock.base_url),
        use_keychain: false,
        oauth_client: Some(OAuthClient { client_id: "test-client".into(), client_secret: None }),
        sessions_file: Some(paths.data_dir.join("agent-sessions.json")),
    })
    .unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let options = Options::parse(args.iter().copied(), &project).unwrap();
    let server = Server::new(Arc::new(engine), options);

    let (near, far) = tokio::io::duplex(1 << 20);
    let (far_read, far_write) = tokio::io::split(far);
    let serving = tokio::spawn(serve(server.clone(), far_read, far_write));
    let (read, writer) = tokio::io::split(near);
    Client {
        mock,
        _dir: dir,
        paths,
        project,
        server,
        writer: Some(writer),
        lines: BufReader::new(read).lines(),
        notifications: Vec::new(),
        next_id: 0,
        serving,
    }
}

impl Client {
    async fn send(&mut self, message: Value) {
        let mut line = serde_json::to_vec(&message).unwrap();
        line.push(b'\n');
        let writer = self.writer.as_mut().unwrap();
        writer.write_all(&line).await.unwrap();
        writer.flush().await.unwrap();
    }

    async fn next_message(&mut self) -> Value {
        let line = tokio::time::timeout(std::time::Duration::from_secs(20), self.lines.next_line())
            .await
            .expect("the server answers")
            .unwrap()
            .expect("the server is still running");
        serde_json::from_str(&line).unwrap()
    }

    /// The response to `id`, keeping notifications that arrive first.
    async fn response(&mut self, id: &Value) -> Value {
        loop {
            let message = self.next_message().await;
            if message.get("id") == Some(id) {
                return message;
            }
            assert!(message.get("method").is_some(), "unexpected message: {message}");
            self.notifications.push(message);
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = json!(self.next_id);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await;
        self.response(&id).await
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let response =
            self.request("tools/call", json!({"name": tool, "arguments": arguments})).await;
        response.get("result").cloned().unwrap_or_else(|| panic!("no result: {response}"))
    }

    /// A tool's JSON answer (fails the test if the tool failed).
    async fn ok(&mut self, tool: &str, arguments: Value) -> Value {
        let result = self.call(tool, arguments).await;
        assert_eq!(result["isError"], false, "{tool} failed: {}", text(&result));
        serde_json::from_str(text(&result)).unwrap_or_else(|_| json!(text(&result)))
    }

    fn assignments(&self) -> Vec<String> {
        self.mock.state().assignments.iter().map(|assignment| assignment.endpoint.clone()).collect()
    }

    /// The agent disconnects: input closes, then the server cleans up.
    async fn disconnect(mut self) -> Self {
        let mut writer = self.writer.take().unwrap();
        writer.shutdown().await.unwrap();
        (&mut self.serving).await.unwrap().unwrap();
        self.server.shutdown().await;
        self
    }
}

fn text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap_or_default()
}

// --------------------------------------------------------------- protocol

#[tokio::test(flavor = "multi_thread")]
async fn handshake_tools_and_protocol_errors() {
    let mut client = client(&[]).await;
    let init = client
        .request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}),
        )
        .await;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "nzap-engine");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(init["result"]["instructions"].as_str().unwrap().contains("run_job"));
    client.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;

    // An unknown version gets the newest one.
    let init = client.request("initialize", json!({"protocolVersion": "1999-01-01"})).await;
    assert_eq!(init["result"]["protocolVersion"], nzap_mcp::PROTOCOL_VERSIONS[0]);

    let tools = client.request("tools/list", json!({})).await;
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    for expected in ["status", "start_runtime", "run_code", "run_job", "run_app", "stop_runtime"] {
        assert!(names.contains(&expected), "{expected} missing from {names:?}");
    }

    assert_eq!(client.request("ping", json!({})).await["result"], json!({}));
    assert_eq!(client.request("resources/list", json!({})).await["error"]["code"], -32601);
    assert_eq!(
        client.request("tools/call", json!({"name": "rm_rf"})).await["error"]["code"],
        -32602
    );

    client.writer.as_mut().unwrap().write_all(b"{not json\n").await.unwrap();
    let parse_error = client.next_message().await;
    assert_eq!(parse_error["error"]["code"], -32700);
    assert_eq!(parse_error["id"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_call_stops_and_the_server_keeps_answering() {
    let mut client = client(&[]).await;
    let runtime =
        client.ok("start_runtime", json!({})).await["runtime"]["name"].as_str().unwrap().to_owned();
    client
        .send(json!({"jsonrpc": "2.0", "id": "slow", "method": "tools/call",
                     "params": {"name": "run_code", "arguments": {"runtime": runtime, "code": "wait_for_interrupt()"}}}))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    client.send(json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": "slow"}})).await;
    // No answer for the cancelled call; the next request is answered.
    let pong = client.request("ping", json!({})).await;
    assert_eq!(pong["result"], json!({}));
    assert!(client.notifications.iter().all(|message| message.get("id").is_none()));
}

// ------------------------------------------------------------------ tools

#[tokio::test(flavor = "multi_thread")]
async fn status_uses_the_apps_google_connection() {
    let mut client = client_with(&[], false).await;
    let status = client.ok("status", json!({})).await;
    assert_eq!(status["connected"], false);
    assert!(status["hint"].as_str().unwrap().contains("Connect Google"));
    let refused = client.call("start_runtime", json!({})).await;
    assert_eq!(refused["isError"], true);
    assert!(text(&refused).contains("Connect Google"), "{}", text(&refused));

    // The user connects in the app while the server runs.
    connect_in_app(&client.mock, &client.paths);
    let status = client.ok("status", json!({})).await;
    assert_eq!(status["connected"], true, "{status}");
    assert_eq!(status["email"], "ada@example.com");
    assert_eq!(status["maxRuntimes"], 2);
    assert!(status["sharedFolders"][0].as_str().unwrap().ends_with("project"));
}

#[tokio::test(flavor = "multi_thread")]
async fn runtimes_code_and_files() {
    let mut client = client(&[]).await;
    let started = client.ok("start_runtime", json!({"hardware": "t4", "name": "work"})).await;
    assert_eq!(started["runtime"]["name"], "work");
    assert_eq!(started["runtime"]["accelerator"], "T4");
    assert_eq!(started["runtime"]["kernelConnected"], true);
    assert_eq!(client.assignments().len(), 1);

    let printed =
        client.call("run_code", json!({"runtime": "work", "code": "print(\"hello\")"})).await;
    assert_eq!(text(&printed), "hello\n");
    assert_eq!(printed["isError"], false);

    let failed = client.call("run_code", json!({"runtime": "work", "code": "raise"})).await;
    assert_eq!(failed["isError"], true);
    assert!(text(&failed).contains("ValueError: boom"));

    let figure =
        client.call("run_code", json!({"runtime": "work", "code": "display_image()"})).await;
    assert_eq!(
        figure["content"][1],
        json!({"type": "image", "mimeType": "image/png", "data": "aGVsbG8="})
    );

    // input() cannot block an agent: it gets an empty line.
    let asked = client.call("run_code", json!({"runtime": "work", "code": "name = input()"})).await;
    assert!(text(&asked).contains("Hello, !"), "{}", text(&asked));

    let unknown = client.call("run_code", json!({"runtime": "nope", "code": "print(1)"})).await;
    assert_eq!(unknown["isError"], true);
    assert!(text(&unknown).contains("list_runtimes"));

    // Files go both ways, inside the project only.
    std::fs::write(client.project.join("clip.mp4"), b"not really a video").unwrap();
    let uploaded =
        client.ok("upload_file", json!({"runtime": "work", "local_path": "clip.mp4"})).await;
    assert_eq!(uploaded["remotePath"], "/content/clip.mp4");
    let endpoint = client.server.engine().sessions.get("work").unwrap().endpoint;
    assert_eq!(
        client.mock.state().runtime(&endpoint).unwrap().files.get("content/clip.mp4"),
        Some(&MockFile::Binary(b"not really a video".to_vec()))
    );
    let listing = client.ok("list_files", json!({"runtime": "work"})).await;
    assert!(listing["entries"].as_array().unwrap().iter().any(|entry| entry["name"] == "clip.mp4"));

    let downloaded =
        client.ok("download_file", json!({"runtime": "work", "remote_path": "clip.mp4"})).await;
    let saved = PathBuf::from(downloaded["savedTo"].as_str().unwrap());
    assert!(saved.starts_with(dunce::canonicalize(&client.project).unwrap().join("nzap-output")));
    assert_eq!(std::fs::read(&saved).unwrap(), b"not really a video");

    std::fs::write(client.project.parent().unwrap().join("secret.txt"), b"no").unwrap();
    let outside =
        client.call("upload_file", json!({"runtime": "work", "local_path": "../secret.txt"})).await;
    assert_eq!(outside["isError"], true);
    assert!(text(&outside).contains("outside the folders shared"));

    // The limit holds: two runtimes at most by default.
    client.ok("start_runtime", json!({})).await;
    let third = client.call("start_runtime", json!({})).await;
    assert_eq!(third["isError"], true);
    assert!(text(&third).contains("limit"));

    let stopped = client.ok("stop_runtime", json!({"runtime": "work"})).await;
    assert_eq!(stopped["released"], true);
    assert_eq!(client.assignments().len(), 1);
    let listed = client.ok("list_runtimes", json!({})).await;
    assert_eq!(listed["runtimes"].as_array().unwrap().len(), 1);
    assert_eq!(listed["otherRuntimesOnAccount"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_uploads_inputs_runs_and_brings_back_artifacts() {
    let mut client = client(&[]).await;
    let video: Vec<u8> = (0..=255).collect();
    std::fs::write(client.project.join("talk.mp4"), &video).unwrap();

    client.next_id += 1;
    let id = json!(client.next_id);
    client
        .send(json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {
            "name": "run_job",
            "_meta": {"progressToken": "job-1"},
            "arguments": {
                "script": "print(\"converted\")",
                "inputs": [{"local_path": "talk.mp4", "remote_path": "out/talk.mp4"}],
                "artifacts": ["out/*"],
            },
        }}))
        .await;
    let response = client.response(&id).await;
    let result = &response["result"];
    assert_eq!(result["isError"], false, "{}", text(result));
    let job: Value = serde_json::from_str(text(result)).unwrap();
    assert_eq!(job["exitCode"], 0);
    assert_eq!(job["released"], true);
    assert_eq!(job["output"], "converted\n");
    let artifact = &job["artifacts"][0];
    assert_eq!(artifact["path"], "/content/out/talk.mp4");
    assert_eq!(std::fs::read(artifact["savedTo"].as_str().unwrap()).unwrap(), video);

    // The VM is gone, and progress was reported along the way.
    assert!(client.assignments().is_empty());
    let progress: Vec<&str> = client
        .notifications
        .iter()
        .filter(|message| message["method"] == "notifications/progress")
        .map(|message| message["params"]["message"].as_str().unwrap())
        .collect();
    assert!(progress.contains(&"job uploading"), "{progress:?}");
    assert!(progress.contains(&"job released"), "{progress:?}");
    assert!(client
        .notifications
        .iter()
        .all(|message| message["params"]["progressToken"] == "job-1"));

    let failing = client.call("run_job", json!({"script": "import sys\nsys.exit(3)"})).await;
    assert_eq!(failing["isError"], true);
    assert_eq!(serde_json::from_str::<Value>(text(&failing)).unwrap()["exitCode"], 3);
    let missing = client.call("run_job", json!({})).await;
    assert!(text(&missing).contains("script"));
}

#[tokio::test(flavor = "multi_thread")]
async fn apps_run_save_their_media_and_stay_warm() {
    let mut client = client(&[]).await;
    let app = json!({
        "format": "nzap-app/1",
        "category": "audio",
        "tagline": "Say it out loud.",
        "runtime": {"accelerator": "CPU"},
        "estimates": {"setup": 10, "run": 1},
        "inputs": [{"param": "reference", "widget": "file"}],
        "outputs": [{"id": "audio", "kind": "audio"}, {"id": "summary", "kind": "text"}],
    });
    client
        .server
        .engine()
        .notebooks
        .create(NotebookDraft {
            slug: "demo-tts".into(),
            title: "Demo TTS".into(),
            description: "A test app.".into(),
            source: "nzap_app_demo()\nprint(params[\"text\"])\n".into(),
            params: vec![
                NotebookParam {
                    key: "text".into(),
                    label: "Text".into(),
                    kind: ParamType::Text,
                    default: None,
                    required: true,
                    options: None,
                    description: None,
                },
                NotebookParam {
                    key: "reference".into(),
                    label: "Reference voice".into(),
                    kind: ParamType::String,
                    default: Some(json!("")),
                    required: false,
                    options: None,
                    description: None,
                },
            ],
            forked_from: None,
            app: Some(app),
        })
        .unwrap();

    let apps = client.ok("list_apps", json!({})).await;
    let listed = apps["apps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|app| app["title"] == "Demo TTS")
        .unwrap()
        .clone();
    assert_eq!(listed["tagline"], "Say it out loud.");
    assert_eq!(listed["params"][1]["isFile"], true);
    // The bundled public apps are listed too.
    assert!(apps["apps"].as_array().unwrap().iter().any(|app| app["id"] == "public:kokoro-tts"));

    std::fs::write(client.project.join("voice.wav"), b"RIFF voice").unwrap();
    let first = client
        .ok("run_app", json!({"app": "demo-tts", "params": {"text": "Hi there"}, "files": {"reference": "voice.wav"}}))
        .await;
    assert_eq!(first["startedRuntime"], true, "{first}");
    assert_eq!(first["warm"], false);
    assert_eq!(first["log"], "Hi there\n");
    let audio =
        first["outputs"].as_array().unwrap().iter().find(|output| output["id"] == "audio").unwrap();
    assert!(std::fs::read(audio["savedTo"].as_str().unwrap()).unwrap().starts_with(b"RIFF"));
    let summary = first["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|output| output["id"] == "summary")
        .unwrap();
    assert_eq!(summary["text"], "Spoke 3 words.");

    // The reference file was uploaded and passed as a VM path.
    let runtime = first["runtime"].as_str().unwrap().to_owned();
    let endpoint = client.server.engine().sessions.get(&runtime).unwrap().endpoint;
    let executed = client.mock.state().runtime(&endpoint).unwrap().executed.clone();
    let cell = executed.iter().find(|code| code.contains("nzap_app_demo")).unwrap();
    assert!(cell.contains("/content/nzap/inputs/demo-tts/"), "{cell}");

    let unknown = client.call("run_app", json!({"app": "no-such-app"})).await;
    assert!(text(&unknown).contains("list_apps"), "{}", text(&unknown));

    // The second run reuses the runtime where the model is loaded.
    let again = client.ok("run_app", json!({"app": "demo-tts", "params": {"text": "Again"}})).await;
    assert_eq!(again["startedRuntime"], false);
    assert_eq!(again["runtime"], runtime);
    assert_eq!(client.assignments().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn disconnecting_releases_only_what_the_agent_started() {
    let mut client = client(&["--max-runtimes", "3"]).await;
    let temporary = client.ok("start_runtime", json!({"name": "temporary"})).await;
    let kept =
        client.ok("start_runtime", json!({"name": "kept", "keep_after_disconnect": true})).await;
    let external = client.mock.state().add_assignment("T4", 1, 0);
    let listed = client.ok("list_runtimes", json!({})).await;
    assert_eq!(listed["otherRuntimesOnAccount"][0]["endpoint"], json!(external.endpoint));
    let attached = client
        .ok("attach_runtime", json!({"endpoint": external.endpoint, "name": "borrowed"}))
        .await;
    assert_eq!(attached["runtime"]["name"], "borrowed");
    let printed =
        client.call("run_code", json!({"runtime": "borrowed", "code": "print(\"shared\")"})).await;
    assert_eq!(text(&printed), "shared\n");

    let temporary_endpoint = temporary["runtime"]["endpoint"].as_str().unwrap().to_owned();
    let kept_endpoint = kept["runtime"]["endpoint"].as_str().unwrap().to_owned();
    let client = client.disconnect().await;
    let left = client.assignments();
    assert!(!left.contains(&temporary_endpoint), "the agent's runtime is released");
    assert!(left.contains(&kept_endpoint), "a kept runtime stays");
    assert!(left.contains(&external.endpoint), "an attached runtime is never released");
}
