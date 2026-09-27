//! The Colab control-plane client against the mock: the assign XSRF dance,
//! headers and query parameters, XSSI stripping, error mapping, quota
//! fallback, keep-alive, telemetry and credential propagation.

mod common;

use nzap_core::colab::{quota, resources, AuthType};
use nzap_core::config::RuntimeRequest;
use nzap_core::Error;
use serde_json::{json, Value};

fn request(gpu: Option<&str>, tpu: Option<&str>, high_mem: bool) -> RuntimeRequest {
    RuntimeRequest {
        name: "box".into(),
        gpu: gpu.map(str::to_owned),
        tpu: tpu.map(str::to_owned),
        high_mem,
    }
}

#[tokio::test]
async fn assign_performs_the_xsrf_two_step() {
    let env = common::connected().await;
    let assignment = env.client.assign(&request(None, None, false), None).await.unwrap();

    // XSSI stripped, runtimeProxyInfo intact.
    let endpoint = assignment["endpoint"].as_str().unwrap().to_owned();
    assert!(endpoint.starts_with("m-s-none-"));
    assert_eq!(assignment["variant"], 0);
    let proxy = &assignment["runtimeProxyInfo"];
    assert!(proxy["url"].as_str().unwrap().ends_with(&format!("/proxy/{endpoint}")));
    assert!(proxy["token"].as_str().unwrap().starts_with("proxy-token-"));

    let state = env.mock.state();
    let calls = state.requests_to("/tun/m/assign");
    assert_eq!(calls.len(), 2);
    let (get, post) = (&calls[0], &calls[1]);
    assert_eq!(get.method, "GET");
    assert_eq!(post.method, "POST");
    for call in [get, post] {
        assert_eq!(call.param("authuser"), Some("0"));
        assert_eq!(call.param("variant"), Some("DEFAULT"));
        assert_eq!(call.param("accelerator"), Some("NONE"));
        assert_eq!(call.param("shape"), None);
        assert_eq!(call.param("nbh").map(str::len), Some(44));
        assert_eq!(call.header("x-colab-client-agent"), Some("nzap-engine"));
        assert_eq!(call.header("accept"), Some("application/json"));
        assert!(call.header("authorization").unwrap().starts_with("Bearer access-"));
    }
    // The POST replays the token the GET issued, on the same notebook hash.
    assert!(post.header("x-goog-colab-token").unwrap().starts_with("xsrf-"));
    assert_eq!(get.param("nbh"), post.param("nbh"));
}

#[tokio::test]
async fn gpu_tpu_and_high_ram_shapes() {
    let env = common::connected().await;
    env.mock.state().max_assignments = 10;

    let t4 = env.client.assign(&request(Some("t4"), None, true), None).await.unwrap();
    assert_eq!((t4["accelerator"].clone(), t4["variant"].clone()), (json!("T4"), json!(1)));
    assert_eq!(t4["machineShape"], 1);

    // L4 only exists as High-RAM: no shape parameter is sent.
    env.client.assign(&request(Some("l4"), None, true), None).await.unwrap();
    let tpu = env.client.assign(&request(None, Some("v5e1"), false), None).await.unwrap();
    assert_eq!(tpu["variant"], 2);

    let state = env.mock.state();
    let posts = state.requests_matching("POST", "/tun/m/assign");
    let shapes: Vec<Option<&str>> = posts.iter().map(|post| post.param("shape")).collect();
    assert_eq!(shapes, vec![Some("hm"), None, None]);
    let accelerators: Vec<Option<&str>> =
        posts.iter().map(|post| post.param("accelerator")).collect();
    assert_eq!(accelerators, vec![Some("T4"), Some("L4"), Some("V5E1")]);
}

#[tokio::test]
async fn allocation_refusals_are_typed() {
    let env = common::connected().await;
    env.mock.state().max_assignments = 1;
    env.client.assign(&request(None, None, false), None).await.unwrap();
    let full = env.client.assign(&request(None, None, false), None).await.unwrap_err();
    assert!(matches!(full, Error::TooManyAssignments), "{full:?}");

    env.mock.state().max_assignments = 5;
    env.mock.state().denied_accelerators.insert("H100".into());
    let denied = env.client.assign(&request(Some("h100"), None, false), None).await.unwrap_err();
    assert!(matches!(denied, Error::Quota(ref message) if message.contains("H100")), "{denied:?}");
}

#[tokio::test]
async fn list_and_unassign() {
    let env = common::connected().await;
    let a = env.client.assign(&request(None, None, false), None).await.unwrap();
    env.client.assign(&request(Some("t4"), None, false), None).await.unwrap();
    assert_eq!(env.client.list_assignments().await.unwrap().len(), 2);

    let endpoint = a["endpoint"].as_str().unwrap();
    env.client.unassign(endpoint).await.unwrap();
    let remaining = env.client.list_assignments().await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_ne!(remaining[0]["endpoint"], a["endpoint"]);
    assert_eq!(env.mock.state().unassigned, vec![endpoint.to_owned()]);

    // The unassign POST replayed the GET's token.
    let posts = env.mock.state().requests_matching("POST", "/tun/m/unassign/");
    assert!(posts[0].header("x-goog-colab-token").unwrap().starts_with("unassign-"));

    // Unknown endpoints surface Colab's 404; malformed ones never leave.
    let missing = env.client.unassign("m-s-gone").await.unwrap_err();
    assert_eq!(missing.status(), Some(404));
    assert!(matches!(env.client.unassign("../etc").await, Err(Error::InvalidInput(_))));
}

#[tokio::test]
async fn keep_alive_sends_the_tunnel_header() {
    let env = common::connected().await;
    let a = env.client.assign(&request(None, None, false), None).await.unwrap();
    let endpoint = a["endpoint"].as_str().unwrap();
    env.client.keep_alive(endpoint).await.unwrap();
    env.client.keep_alive(endpoint).await.unwrap();
    assert_eq!(env.mock.state().keepalives[endpoint], 2);
    let calls = env.mock.state().requests_to(&format!("/tun/m/{endpoint}/keep-alive/"));
    assert_eq!(calls[0].header("x-colab-tunnel"), Some("Google"));
    assert_eq!(calls[0].param("authuser"), Some("0"));
}

#[tokio::test]
async fn a_rejected_token_is_refreshed_once() {
    let env = common::connected().await;
    env.client.list_assignments().await.unwrap();
    let before = env.mock.state().refresh_count;

    // Google drops the access token early; the client refreshes and retries.
    env.mock.state().expire_access_tokens();
    env.client.list_assignments().await.unwrap();
    assert_eq!(env.mock.state().refresh_count, before + 1);

    // With the grant revoked there is nothing to retry with.
    env.mock.state().revoke_grant();
    let error = env.client.list_assignments().await.unwrap_err();
    assert!(matches!(error, Error::AuthExpired(_)), "{error:?}");
}

#[tokio::test]
async fn quota_falls_back_to_ccu_info() {
    let env = common::connected().await;
    let fallback = quota::fetch(&env.client).await.unwrap();
    assert_eq!(fallback.source, "ccu-info");
    assert!(fallback.errors[0].starts_with("user-info:"));
    assert_eq!(fallback.eligible_accelerators, vec!["T4", "V5E1"]);
    assert_eq!(fallback.free_ccu_remaining, Some(36.0));

    env.mock.state().user_info = Some(json!({
        "subscriptionTier": "SUBSCRIPTION_TIER_PRO_PLUS",
        "paidComputeUnitsBalance": 480.5,
        "consumptionRateHourly": 1.84,
    }));
    let direct = quota::fetch(&env.client).await.unwrap();
    assert_eq!(direct.source, "user-info");
    assert!(direct.errors.is_empty());
    assert_eq!(direct.status_text, "1.84/hr");
    let user_info = env.mock.state().requests_to("/v1/user-info");
    assert_eq!(user_info.last().unwrap().param("get_ccu_consumption_info"), Some("true"));
}

#[tokio::test]
async fn runtime_specs_degrade_to_an_error() {
    let env = common::connected().await;
    assert!(env.client.list_runtime_specs().await.is_err());
    env.mock.state().runtime_specs =
        Some(json!({"runtimeSpecs": [{"accelerator": "T4", "shape": "STANDARD"}]}));
    let specs = env.client.list_runtime_specs().await.unwrap();
    assert_eq!(specs[0]["accelerator"], "T4");
}

#[tokio::test]
async fn telemetry_is_read_through_the_proxy() {
    let env = common::connected().await;
    let a = env.client.assign(&request(None, None, false), None).await.unwrap();
    let proxy = &a["runtimeProxyInfo"];
    let raw = env
        .client
        .get_resources(proxy["url"].as_str().unwrap(), proxy["token"].as_str().unwrap())
        .await
        .unwrap();
    let normalized = resources::normalize(&raw);
    let ram = normalized.ram.unwrap();
    assert_eq!(ram.limit, Some(13_609_431_040.0));
    assert_eq!(ram.usage, Some(2_000_000_000.0));
    assert!(normalized.gpu.is_none());

    let bad = env.client.get_resources(proxy["url"].as_str().unwrap(), "wrong").await;
    assert_eq!(bad.unwrap_err().status(), Some(403));
}

#[tokio::test]
async fn credential_propagation_with_and_without_consent() {
    let env = common::connected().await;
    let a = env.client.assign(&request(None, None, false), None).await.unwrap();
    let endpoint = a["endpoint"].as_str().unwrap().to_owned();

    let granted = env.client.propagate_credentials(&endpoint, AuthType::DfsEphemeral).await.unwrap();
    assert!(granted.success);
    assert_eq!(granted.unauthorized_redirect_uri, None);
    let expected: Vec<(String, String, String)> = vec![
        (endpoint.clone(), "dfs_ephemeral".into(), "true".into()),
        (endpoint.clone(), "dfs_ephemeral".into(), "false".into()),
    ];
    assert_eq!(env.mock.state().propagations, expected);

    env.mock.state().propagations.clear();
    env.mock.state().drive_consent = false;
    let pending =
        env.client.propagate_credentials(&endpoint, AuthType::AuthUserEphemeral).await.unwrap();
    assert!(!pending.success);
    assert_eq!(
        pending.unauthorized_redirect_uri.as_deref(),
        Some("https://accounts.google.com/o/oauth2/consent?x=1&y=2")
    );
    // Without consent only the dry run is sent.
    let posts = env.mock.state().propagations.clone();
    let dry_run_only = vec![(endpoint.clone(), String::from("auth_user_ephemeral"), String::from("true"))];
    assert_eq!(posts, dry_run_only);

    let calls = env.mock.state().requests_matching("POST", "/tun/m/credentials-propagation/");
    let call = calls.last().unwrap();
    assert!(call.header("content-type").unwrap().starts_with("multipart/form-data"));
    assert_eq!(call.param("version"), Some("2"));
    assert_eq!(call.param("propagate"), Some("true"));
    assert_eq!(call.param("record"), Some("false"));
}

#[tokio::test]
async fn user_info_is_refused_like_the_real_api() {
    let env = common::connected().await;
    let error = env.client.get_user_info(true).await.unwrap_err();
    assert_eq!(error.status(), Some(403));
    let info: Value = env.client.get_ccu_info().await.unwrap();
    assert_eq!(info["assignmentsCount"], 0);
}
