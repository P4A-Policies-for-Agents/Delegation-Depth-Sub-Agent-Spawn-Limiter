// Copyright 2026 Salesforce, Inc. All rights reserved.
//
// Integration test (requires Docker; run with `make test`). Composes a local Flex
// instance + an HttpMock backend and asserts the delegation depth ceiling: a
// tools/call at depth == maxDepth is allowed and depth > maxDepth is rejected
// (HTTP 403). The pure logic is covered by `cargo test --lib`.

mod common;

use httpmock::MockServer;
use pdk_test::port::Port;
use pdk_test::services::flex::{ApiConfig, Flex, FlexConfig, PolicyConfig};
use pdk_test::services::httpmock::{HttpMock, HttpMockConfig};
use pdk_test::{pdk_test, TestComposite};

use common::*;

const FLEX_PORT: Port = 8081;

#[pdk_test]
async fn enforces_depth_ceiling() -> anyhow::Result<()> {
    let httpmock_config = HttpMockConfig::builder()
        .port(80)
        .version("latest")
        .hostname("backend")
        .build();

    let policy_config = PolicyConfig::builder()
        .name(POLICY_NAME)
        .configuration(serde_json::json!({
            "maxDepth": 2,
            "maxFanout": 0,           // disable fan-out for a pure depth test
            "onExceed": "reject"
        }))
        .build();

    let api_config = ApiConfig::builder()
        .name("myApi")
        .upstream(&httpmock_config)
        .path("/")
        .port(FLEX_PORT)
        .policies([policy_config])
        .build();

    let flex_config = FlexConfig::builder()
        .version("1.10.0")
        .hostname("local-flex")
        .with_api(api_config)
        .config_mounts([(POLICY_DIR, "policy"), (COMMON_CONFIG_DIR, "common")])
        .build();

    let composite = TestComposite::builder()
        .with_service(flex_config)
        .with_service(httpmock_config)
        .build()
        .await?;

    let flex: Flex = composite.service()?;
    let flex_url = flex.external_url(FLEX_PORT).unwrap();
    let httpmock: HttpMock = composite.service()?;
    let mock_server = MockServer::connect_async(httpmock.socket()).await;

    mock_server
        .mock_async(|when, then| {
            when.method(httpmock::Method::POST);
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"jsonrpc":"2.0","id":2,"result":{"structuredContent":{"resultSummary":"ok"}}}"#);
        })
        .await;

    let client = reqwest::Client::new();
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"run_subtask","arguments":{"taskDescription":"x"}}}"#;

    let send = |depth: i64| {
        let client = client.clone();
        let url = flex_url.clone();
        let deleg = format!(r#"{{"root":"r","depth":{depth},"agent":"a{depth}","parent":"a"}}"#);
        async move {
            client
                .post(format!("{url}/"))
                .header("content-type", "application/json")
                .header("x-agent-delegation", deleg)
                .body(call)
                .send()
                .await
                .map(|r| r.status().as_u16())
        }
    };

    assert_eq!(send(2).await?, 200); // at the ceiling -> allowed
    assert_eq!(send(3).await?, 403); // beyond the ceiling -> rejected

    Ok(())
}
