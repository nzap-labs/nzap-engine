//! The runtime's Jupyter server through `RuntimeProxy`: the contents API
//! (list, read, write, upload, download, mkdir, rename, delete), kernels and
//! sessions, token handling and error mapping.

mod common;

use nzap_core::config::RuntimeRequest;
use nzap_core::runtime::RuntimeProxy;
use nzap_core::Error;
use nzap_mock_colab::MockFile;
use serde_json::{json, Value};

async fn runtime() -> (common::Connected, RuntimeProxy, String) {
    let env = common::connected().await;
    let assignment = env.client.assign(&RuntimeRequest::default(), None).await.unwrap();
    let proxy = RuntimeProxy::new(
        env.auth.http().clone(),
        assignment["runtimeProxyInfo"]["url"].as_str().unwrap(),
        assignment["runtimeProxyInfo"]["token"].as_str().unwrap(),
    );
    let endpoint = assignment["endpoint"].as_str().unwrap().to_owned();
    (env, proxy, endpoint)
}

fn names(listing: &Value) -> Vec<String> {
    listing["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn browse_read_and_write_files() {
    let (env, proxy, endpoint) = runtime().await;

    let root = proxy.list_contents("").await.unwrap();
    assert_eq!(root["type"], "directory");
    assert_eq!(names(&root), vec!["content"]);
    let content = proxy.list_contents("/content/").await.unwrap();
    assert_eq!(names(&content), vec!["sample_data"]);

    // Both auth forms travel on every request.
    let request =
        env.mock.state().requests_to(&format!("/proxy/{endpoint}/api/contents")).remove(0);
    assert_eq!(request.param("colab-runtime-proxy-token"), Some(proxy.token()));
    assert_eq!(request.header("x-colab-runtime-proxy-token"), Some(proxy.token()));
    assert_eq!(request.param("authuser"), Some("0"));
    // …but the Google bearer token never goes to the runtime.
    assert_eq!(request.header("authorization"), None);

    let written = proxy.write_file("content/my script.py", "print(1)\n", "text").await.unwrap();
    assert_eq!(written["path"], "content/my script.py");
    let read = proxy.read_file("content/my script.py").await.unwrap();
    assert_eq!(read["content"], "print(1)\n");
    assert_eq!(proxy.download("content/my script.py").await.unwrap(), b"print(1)\n");
    // Spaces are percent-encoded on the wire.
    assert!(env
        .mock
        .state()
        .requests
        .iter()
        .any(|request| request.path.ends_with("/api/contents/content/my%20script.py")
            || request.path.ends_with("/api/contents/content/my script.py")));
}

#[tokio::test]
async fn binary_uploads_round_trip() {
    let (env, proxy, endpoint) = runtime().await;
    let bytes: Vec<u8> = (0..=255).collect();
    proxy.upload_file("content/out/model.bin", &bytes).await.unwrap();
    assert_eq!(proxy.download("content/out/model.bin").await.unwrap(), bytes);
    // Parent directories are created like Jupyter's upload flow expects.
    let state = env.mock.state();
    let files = &state.runtime(&endpoint).unwrap().files;
    assert_eq!(files.get("content/out"), Some(&MockFile::Directory));
    assert_eq!(files.get("content/out/model.bin"), Some(&MockFile::Binary(bytes)));
}

#[tokio::test]
async fn large_uploads_go_in_chunks() {
    let (env, proxy, endpoint) = runtime().await;
    let size = nzap_core::runtime::proxy::UPLOAD_CHUNK_BYTES * 2 + 5;
    let bytes: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
    proxy.upload_file("content/video.mp4", &bytes).await.unwrap();
    let state = env.mock.state();
    let runtime = state.runtime(&endpoint).unwrap();
    assert_eq!(runtime.upload_chunks, vec![1, 2, -1]);
    assert_eq!(runtime.files.get("content/video.mp4"), Some(&MockFile::Binary(bytes)));
}

#[tokio::test]
async fn notebooks_download_as_pretty_json() {
    let (env, proxy, endpoint) = runtime().await;
    env.mock
        .state()
        .runtime_mut(&endpoint)
        .unwrap()
        .files
        .insert("content/nb.ipynb".into(), MockFile::Notebook(json!({"cells": [], "nbformat": 4})));
    let bytes = proxy.download("content/nb.ipynb").await.unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("{\n \"cells\": []"));
    let directory = proxy.download("content").await.unwrap_err();
    assert!(matches!(directory, Error::InvalidInput(_)), "{directory:?}");
}

#[tokio::test]
async fn mkdir_rename_delete() {
    let (_env, proxy, _endpoint) = runtime().await;
    proxy.make_directory("content/work").await.unwrap();
    proxy.write_file("content/work/a.txt", "a", "text").await.unwrap();

    let renamed = proxy.rename("content/work", "/content/done/").await.unwrap();
    assert_eq!(renamed["path"], "content/done");
    assert_eq!(names(&proxy.list_contents("content/done").await.unwrap()), vec!["a.txt"]);
    let gone = proxy.list_contents("content/work").await.unwrap_err();
    assert_eq!(gone.status(), Some(404));
    assert!(gone.to_string().contains("Not found"));

    proxy.delete("content/done").await.unwrap();
    assert!(proxy.read_file("content/done/a.txt").await.is_err());
    assert_eq!(proxy.delete("content/done").await.unwrap_err().status(), Some(404));
}

#[tokio::test]
async fn kernels_and_named_sessions() {
    let (env, proxy, endpoint) = runtime().await;
    assert!(proxy.list_kernels().await.unwrap().is_empty());

    let kernel = proxy.start_kernel("python3").await.unwrap();
    let kernel_id = kernel["id"].as_str().unwrap().to_owned();
    let session = proxy.create_session("my-runtime", &kernel_id, "python3").await.unwrap();
    assert_eq!(session["path"], "my-runtime.ipynb");
    assert_eq!(session["kernel"]["id"], kernel_id.as_str());
    assert_eq!(proxy.list_sessions().await.unwrap().len(), 1);

    proxy.interrupt_kernel(&kernel_id).await.unwrap();
    proxy.restart_kernel(&kernel_id).await.unwrap();
    {
        let state = env.mock.state();
        let runtime = state.runtime(&endpoint).unwrap();
        assert_eq!((runtime.interrupts, runtime.restarts), (1, 1));
    }
    proxy.shutdown_kernel(&kernel_id).await.unwrap();
    assert!(proxy.list_kernels().await.unwrap().is_empty());
    assert_eq!(proxy.restart_kernel(&kernel_id).await.unwrap_err().status(), Some(404));
}

#[tokio::test]
async fn a_wrong_proxy_token_is_refused() {
    let (env, proxy, _endpoint) = runtime().await;
    let intruder = RuntimeProxy::new(env.auth.http().clone(), proxy.base_url(), "not-the-token");
    let error = intruder.list_contents("").await.unwrap_err();
    assert!(matches!(error, Error::Runtime { status: Some(403), .. }), "{error:?}");
    // Error text names the path, never the tokenised URL.
    assert!(!error.to_string().contains("not-the-token"));
}
