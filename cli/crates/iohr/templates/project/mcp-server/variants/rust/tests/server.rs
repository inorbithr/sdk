//! The passes-on-init test: the server is started the way a host starts it (a child process over
//! stdio) and spoken to by the official client. It checks the 2026-07-28 revision (no handshake:
//! server/discover), that a client of the older revision still gets its handshake, the tool list
//! and its order, a tool call and its structured result, a refused bad argument, and the rules
//! every tool must meet.
use rmcp::{
    ClientHandler, ClientLifecycleMode, ClientServiceExt, RoleClient,
    model::{CallToolRequestParams, ClientConfig, ProtocolVersion, ReadResourceRequestParams},
    service::RunningService,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_@@NAME@@");

#[derive(Clone)]
struct Host(ProtocolVersion);

impl ClientHandler for Host {
    fn get_info(&self) -> ClientConfig {
        let mut info = ClientConfig::default();
        info.protocol_version = self.0.clone();
        info
    }
}

type Session = RunningService<RoleClient, Host>;

fn child() -> TokioChildProcess {
    TokioChildProcess::new(tokio::process::Command::new(BIN).configure(|_| {})).expect("start the server")
}

/// A client of the 2026-07-28 revision: it opens with server/discover and no handshake.
async fn discover() -> Session {
    Host(ProtocolVersion::V_2026_07_28)
        .serve_with_lifecycle(
            child(),
            ClientLifecycleMode::Discover { preferred_versions: vec![ProtocolVersion::V_2026_07_28] },
        )
        .await
        .expect("discover")
}

/// A client of 2025-11-25: initialize was removed in 2026-07-28, the SDK still answers it.
async fn legacy() -> Session {
    Host(ProtocolVersion::V_2025_11_25)
        .serve_with_lifecycle(child(), ClientLifecycleMode::Initialize)
        .await
        .expect("initialize")
}

fn args(v: Value) -> CallToolRequestParams {
    CallToolRequestParams::new("word_count").with_arguments(v.as_object().cloned().unwrap_or_default())
}

#[tokio::test]
async fn discover_answers_on_the_20260728_revision() {
    let s = discover().await;
    let info = s.peer_info().expect("server info");
    assert_eq!(info.server_info.as_ref().map(|i| i.name.as_str()), Some("@@NAME@@"));
    assert_eq!(info.protocol_version, ProtocolVersion::V_2026_07_28);
    s.cancel().await.expect("close");
}

#[tokio::test]
async fn a_client_of_the_older_revision_still_gets_its_handshake() {
    let s = legacy().await;
    let info = s.peer_info().expect("server info");
    assert_eq!(info.server_info.as_ref().map(|i| i.name.as_str()), Some("@@NAME@@"));
    assert_eq!(info.protocol_version, ProtocolVersion::V_2025_11_25);
    s.cancel().await.expect("close");
}

#[tokio::test]
async fn tools_list_names_the_tool_and_comes_in_the_same_order_twice() {
    let s = discover().await;
    let first = s.list_tools(None).await.expect("list");
    let second = s.list_tools(None).await.expect("list again");
    let names = |r: &rmcp::model::ListToolsResult| r.tools.iter().map(|t| t.name.to_string()).collect::<Vec<_>>();
    assert_eq!(names(&first), vec!["word_count"]);
    assert_eq!(names(&first), names(&second));
    // Lists say how long a client may keep them (2026-07-28): ttlMs and cacheScope are on the wire.
    assert!(first.ttl_ms.is_some() && first.cache_scope.is_some());
    s.cancel().await.expect("close");
}

#[tokio::test]
async fn a_tool_call_returns_the_structured_result() {
    let s = discover().await;
    let r = s.call_tool(args(json!({"text": "one two  three"}))).await.expect("call");
    assert_ne!(r.is_error, Some(true));
    assert_eq!(r.structured_content, Some(json!({"words": 3, "characters": 14})));
    s.cancel().await.expect("close");
}

#[tokio::test]
async fn a_bad_argument_is_refused_and_the_tool_does_not_run() {
    let s = discover().await;
    for (what, bad) in [
        ("too long", json!({"text": "x".repeat(10_001)})),
        ("missing", json!({})),
        ("extra", json!({"text": "a", "other": 1})),
    ] {
        let refused = match s.call_tool(args(bad)).await {
            Err(_) => true,
            Ok(r) => r.is_error == Some(true),
        };
        assert!(refused, "{what}: was not refused");
    }
    s.cancel().await.expect("close");
}

#[tokio::test]
async fn the_resource_is_listed_and_can_be_read() {
    let s = discover().await;
    let listed = s.list_resources(None).await.expect("list");
    assert_eq!(listed.resources.len(), 1);
    assert_eq!(listed.resources[0].uri, "about://server");
    let read = s.read_resource(ReadResourceRequestParams::new("about://server")).await.expect("read");
    let text = serde_json::to_string(&read).expect("json");
    assert!(text.contains("counts words"), "{text}");
    s.cancel().await.expect("close");
}

#[tokio::test]
async fn every_tool_has_a_description_an_annotation_and_bounded_inputs() {
    let s = discover().await;
    for tool in s.list_all_tools().await.expect("list") {
        let name = tool.name.to_string();
        assert!(tool.description.as_deref().is_some_and(|d| !d.is_empty()), "{name}: no description");
        let a = tool.annotations.as_ref();
        assert!(
            a.is_some_and(|a| a.read_only_hint == Some(true) || a.destructive_hint.is_some()),
            "{name}: say whether it only reads (readOnlyHint) or whether it destroys (destructiveHint)"
        );
        let schema = Value::Object((*tool.input_schema).clone());
        for (prop, p) in schema["properties"].as_object().cloned().unwrap_or_default() {
            match p["type"].as_str() {
                Some("string") => assert!(
                    p.get("maxLength").is_some() || p.get("enum").is_some() || p.get("pattern").is_some(),
                    "{name}.{prop}: bound the string (maxLength, enum or pattern)"
                ),
                Some("integer" | "number") => assert!(
                    p.get("minimum").is_some() || p.get("maximum").is_some(),
                    "{name}.{prop}: bound the number (minimum, maximum)"
                ),
                _ => {}
            }
        }
    }
    s.cancel().await.expect("close");
}
