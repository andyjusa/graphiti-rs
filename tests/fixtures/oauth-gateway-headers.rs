//! Compiled inside an isolated copy of the pinned OAuth gateway source.
use super::*;
use graphiti_rs::compat::{
    config::{CompatConfig, IngestionPipeline, WriteMode},
    engine::Engine,
    mcp::CompatService,
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn gateway(upstream: String) -> App {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE tokens(hash TEXT PRIMARY KEY,expires INTEGER);")
        .unwrap();
    for (token, expiry) in [("fixture-owner", now() + 600), ("fixture-expired", 0)] {
        db.execute(
            "INSERT INTO tokens VALUES (?1,?2)",
            params![challenge(token), expiry],
        )
        .unwrap();
    }
    App {
        db: Arc::new(Mutex::new(db)),
        pending: Arc::default(),
        http: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .unwrap(),
        issuer: PUBLIC_ISSUER.into(),
        secret: "synthetic-unused".into(),
        backend: "Bearer fixture-backend".into(),
        upstream,
    }
}

fn request(version: &str, method: &str, name: Option<&str>) -> (HeaderMap, Value) {
    let mut params = json!({});
    if version == "2026-07-28" {
        params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":version,"io.modelcontextprotocol/clientCapabilities":{}});
    }
    if method == "initialize" {
        params = json!({"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"synthetic-gateway","version":"1"}});
    }
    let mut headers = HeaderMap::new();
    for (key, value) in [
        ("authorization", "Bearer fixture-owner"),
        ("accept", "application/json, text/event-stream"),
        ("content-type", "application/json"),
        ("cookie", "synthetic-private-cookie"),
        ("x-private-sentinel", "must-not-forward"),
        ("mcp-protocol-version", version),
    ] {
        headers.insert(key, value.parse().unwrap());
    }
    if version == "2026-07-28" {
        headers.insert("mcp-method", method.parse().unwrap());
    }
    if let Some(name) = name {
        params["name"] = json!(name);
        params["arguments"] = json!({});
        if version == "2026-07-28" {
            headers.insert("mcp-name", name.parse().unwrap());
        }
    }
    (
        headers,
        json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
    )
}

async fn call(a: &App, headers: HeaderMap, body: Value) -> (StatusCode, Value) {
    let r = mcp(
        State(a.clone()),
        Method::POST,
        headers,
        axum::body::Bytes::from(serde_json::to_vec(&body).unwrap()),
    )
    .await;
    let status = r.status();
    if status == StatusCode::UNAUTHORIZED {
        assert!(r.headers().contains_key("www-authenticate"));
    }
    let bytes = axum::body::to_bytes(r.into_body(), 262144).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[ignore = "explicit exclusively owned disposable DB on loopback port 16379 required"]
async fn modern_headers_and_legacy_requests_preserve_authentication_and_transport_validation() {
    let url = std::env::var("COMPAT_TEST_DB_URL").expect("explicit disposable DB URL required");
    assert_eq!(url, "redis://127.0.0.1:16379");
    let forwarded = std::env::var("OAUTH_HEADER_EXPECT_FORWARDING").unwrap() == "1";
    let engine = Engine::new(CompatConfig {
        redis_url: url,
        mode: WriteMode::ReadOnly,
        pipeline: IngestionPipeline::Graphiti,
        entity_types: vec![],
        journal_dir: None,
        llm_base: "http://127.0.0.1:1".into(),
        llm_model: "gpt-6-luna".into(),
        llm_key_file: None,
        gemini_transport: graphiti_rs::compat::config::GeminiTransport::Native,
        gemini_base: "http://127.0.0.1:1".into(),
        gemini_key_file: None,
        max_tokens: 512,
        timeout: std::time::Duration::from_secs(3),
        capacity: 8,
        candidates: 20,
    })
    .await
    .unwrap();
    let mut config =
        graphiti_rs::config::Config::from_env_with_token(Some("fixture-backend".into())).unwrap();
    config.allowed_hosts = vec!["localhost:8000".into()];
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let duplicates = Arc::new(AtomicUsize::new(0));
    let duplicate_count = duplicates.clone();
    let backend = graphiti_rs::http::router_service(
        CompatService::new(engine),
        &config,
        tokio_util::sync::CancellationToken::new(),
    )
    .unwrap()
    .layer(axum::middleware::from_fn(
        move |r: axum::extract::Request, next: axum::middleware::Next| {
            let count = count.clone();
            let duplicate_count = duplicate_count.clone();
            async move {
                assert_eq!(r.headers()["authorization"], "Bearer fixture-backend");
                assert_eq!(r.headers()["host"], "localhost:8000");
                for name in ["cookie", "origin", "x-private-sentinel"] {
                    assert!(!r.headers().contains_key(name));
                }
                count.fetch_add(1, Ordering::SeqCst);
                if ["mcp-method", "mcp-name"]
                    .iter()
                    .any(|k| r.headers().get_all(*k).iter().count() > 1)
                {
                    duplicate_count.fetch_add(1, Ordering::SeqCst);
                }
                next.run(r).await
            }
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = gateway(format!("http://{}/mcp", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move { axum::serve(listener, backend).await.unwrap() });
    let mut events = vec![];
    for (method, name) in [
        ("server/discover", None),
        ("tools/list", None),
        ("tools/call", Some("get_status")),
    ] {
        let (h, b) = request("2026-07-28", method, name);
        let (status, payload) = call(&a, h, b).await;
        if forwarded {
            assert_eq!(status, StatusCode::OK, "{method}: {payload}");
            assert!(payload.get("result").is_some());
            if method == "tools/list" {
                assert_eq!(payload["result"]["tools"].as_array().unwrap().len(), 10);
            }
            if method == "tools/call" {
                assert_eq!(payload["result"]["structuredContent"]["status"], "ok");
            }
        } else {
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(payload["error"]["code"], -32020);
        }
        events.push(json!({"case":format!("modern_{method}"),"status":status.as_u16(),"rpc_error":payload["error"]["code"]}));
    }
    for (method, name) in [
        ("initialize", None),
        ("tools/list", None),
        ("tools/call", Some("get_status")),
    ] {
        let (h, b) = request("2025-11-25", method, name);
        let (status, payload) = call(&a, h, b).await;
        assert_eq!(status, StatusCode::OK, "legacy {method}: {payload}");
        assert!(payload.get("result").is_some());
        if method == "tools/list" {
            assert_eq!(payload["result"]["tools"].as_array().unwrap().len(), 10);
        }
        events.push(json!({"case":format!("legacy_{method}"),"status":status.as_u16()}));
    }
    for (header, value) in [
        ("mcp-method", Some("tools/list")),
        ("mcp-method", None),
        ("mcp-name", Some("clear_graph")),
        ("mcp-name", None),
    ] {
        let (mut h, b) = request("2026-07-28", "tools/call", Some("get_status"));
        if let Some(value) = value {
            h.insert(header, value.parse().unwrap());
        } else {
            h.remove(header);
        }
        let (status, payload) = call(&a, h, b).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(payload["error"]["code"], -32020);
        events.push(json!({"case":format!("reject_{header}_{}",if value.is_some(){"mismatch"}else{"missing"}),"status":status.as_u16(),"rpc_error":-32020}));
    }
    for (header, value) in [
        ("mcp-method", "tools/list"),
        ("mcp-method", "tools/call"),
        ("mcp-name", "clear_graph"),
        ("mcp-name", "get_status"),
    ] {
        let (mut h, b) = request("2026-07-28", "tools/call", Some("get_status"));
        h.append(header, value.parse().unwrap());
        let (status, payload) = call(&a, h, b).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "duplicate {header} was collapsed"
        );
        assert_eq!(payload["error"]["code"], -32020);
        events.push(
            json!({"case":format!("reject_{header}_duplicate"),"status":400,"rpc_error":-32020}),
        );
    }
    assert_eq!(
        duplicates.load(Ordering::SeqCst),
        if forwarded { 4 } else { 0 }
    );
    let before = calls.load(Ordering::SeqCst);
    for auth in [None, Some("Bearer invalid"), Some("Bearer fixture-expired")] {
        let (mut h, b) = request("2026-07-28", "tools/list", None);
        if let Some(auth) = auth {
            h.insert("authorization", auth.parse().unwrap());
        } else {
            h.remove("authorization");
        }
        assert_eq!(call(&a, h, b).await.0, StatusCode::UNAUTHORIZED);
        events.push(json!({"case":"reject_missing_invalid_expired_token","status":401}));
    }
    let (mut h, b) = request("2026-07-28", "tools/list", None);
    h.insert("origin", "https://evil.test".parse().unwrap());
    assert_eq!(call(&a, h, b).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        before,
        "auth/Origin refusal reached upstream"
    );
    events.push(json!({"case":"reject_origin_before_upstream","status":401}));
    if let Some(path) = std::env::var_os("OAUTH_HEADER_REPORT") {
        std::fs::write(path,serde_json::to_vec_pretty(&json!({"headers_forwarded":forwarded,"events":events,"backend_http_requests":calls.load(Ordering::SeqCst),"duplicate_header_requests_received_by_backend":duplicates.load(Ordering::SeqCst),"actual_provider_calls":0,"operational_writes":0,"fixed_backend_credential_preserved":true,"unrelated_headers_not_forwarded":true,"auth_origin_rejected_before_upstream":true})).unwrap()).unwrap();
    }
    server.abort();
}
