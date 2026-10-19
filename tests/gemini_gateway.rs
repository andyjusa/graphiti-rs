#![cfg(feature = "graphiti-compat")]

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use graphiti_rs::compat::{
    config::{CompatConfig, GeminiTransport, IngestionPipeline, WriteMode},
    providers::Providers,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;

const GATEWAY: &str = "https://factchat-cloud.mindlogic.ai/v1/gateway";
const GOOGLE: &str = "https://generativelanguage.googleapis.com/v1beta";
const DUMMY_KEY: &str = "synthetic-gateway-test-key";

struct PrivateFiles(PathBuf);

impl PrivateFiles {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("graphiti-gateway-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let key = root.join("dummy.key");
        std::fs::write(&key, DUMMY_KEY).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        Self(root)
    }

    fn config(&self, base: &str) -> CompatConfig {
        CompatConfig {
            redis_url: "redis://127.0.0.1:16379".into(),
            pipeline: IngestionPipeline::Graphiti,
            entity_types: vec![],
            mode: WriteMode::Synthetic,
            journal_dir: Some(self.0.join("journal")),
            llm_base: base.into(),
            llm_model: "gpt-6-luna".into(),
            llm_key_file: Some(self.0.join("dummy.key")),
            gemini_transport: GeminiTransport::FactChat,
            gemini_base: base.into(),
            // A nonexistent Google key proves that FactChat never reads or sends it.
            gemini_key_file: Some(self.0.join("unused-google.key")),
            max_tokens: 512,
            timeout: Duration::from_secs(3),
            capacity: 8,
            candidates: 20,
        }
    }
}

impl Drop for PrivateFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
struct Captured {
    headers: HeaderMap,
    body: Value,
}

#[derive(Clone)]
enum Reply {
    Reversed,
    Fixed(StatusCode, String),
}

#[derive(Clone)]
struct MockState {
    calls: Arc<Mutex<Vec<Captured>>>,
    reply: Reply,
}

async fn embeddings(
    State(state): State<MockState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> impl IntoResponse {
    state.calls.lock().await.push(Captured {
        headers,
        body: body.clone(),
    });
    match state.reply {
        Reply::Reversed => {
            let inputs = body["input"].as_array().unwrap();
            let data: Vec<Value> = inputs
                .iter()
                .enumerate()
                .rev()
                .map(|(index, text)| {
                    let marker: f32 = text
                        .as_str()
                        .unwrap()
                        .split('|')
                        .next()
                        .unwrap()
                        .parse()
                        .unwrap();
                    json!({"object":"embedding","index":index,"embedding":vec![marker;3072]})
                })
                .collect();
            (
                StatusCode::OK,
                json!({"model":"gemini-embedding-2","data":data}).to_string(),
            )
        }
        Reply::Fixed(status, body) => (status, body),
    }
}

struct MockServer {
    base: String,
    calls: Arc<Mutex<Vec<Captured>>>,
    task: tokio::task::JoinHandle<()>,
}

impl MockServer {
    async fn start(reply: Reply) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(Mutex::new(vec![]));
        // Register only the exact documented trailing-slash route.
        let app = axum::Router::new()
            .route("/embeddings/", post(embeddings))
            .with_state(MockState {
                calls: calls.clone(),
                reply,
            });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { base, calls, task }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn factchat_preserves_input_order_and_wire_contract_across_eight_plus_one_batch() {
    let files = PrivateFiles::new();
    let server = MockServer::start(Reply::Reversed).await;
    let provider = Providers::new(files.config(&format!("{}/", server.base))).unwrap();
    let texts: Vec<String> = (1..=9).map(|i| format!("{i}|line\nnext\r끝")).collect();
    let result = provider.embed(&texts).await.unwrap();
    assert_eq!(
        result.len(),
        texts.len(),
        "every input must receive exactly one vector"
    );
    for (i, vector) in result.iter().enumerate() {
        assert_eq!(
            vector,
            &vec![(i + 1) as f32; 3072],
            "vector must follow input index {i}"
        );
    }
    let calls = server.calls.lock().await;
    assert_eq!(calls.len(), 2, "nine inputs must use two bounded requests");
    for (call, batch) in calls.iter().zip(texts.chunks(8)) {
        assert!(
            call.headers["user-agent"]
                .to_str()
                .unwrap()
                .starts_with("graphiti-rs/"),
            "the gateway requires an identifiable User-Agent"
        );
        assert_eq!(
            call.headers["authorization"].to_str().unwrap(),
            format!("Bearer {DUMMY_KEY}")
        );
        assert!(
            !call.headers.contains_key("x-goog-api-key"),
            "Google authentication must not reach FactChat"
        );
        assert_eq!(
            call.body,
            json!({
                "model":"gemini-embedding-2",
                "input":batch.iter().map(|s|s.replace('\n', " ")).collect::<Vec<_>>(),
                "dimensions":3072,
                "encoding_format":"float"
            }),
            "request must preserve CR, replace LF, and omit task_type"
        );
    }
}

#[tokio::test]
async fn empty_and_oversized_inputs_spend_no_requests_and_maximum_batch_is_accepted() {
    let files = PrivateFiles::new();
    let server = MockServer::start(Reply::Reversed).await;
    let provider = Providers::new(files.config(&server.base)).unwrap();
    assert!(provider.embed(&[]).await.unwrap().is_empty());
    assert!(
        provider
            .embed(&vec!["1|too-many".into(); 257])
            .await
            .is_err()
    );
    assert!(
        server.calls.lock().await.is_empty(),
        "empty and rejected inputs must not call the provider"
    );
    let result = provider
        .embed(&vec!["1|at-bound".into(); 256])
        .await
        .unwrap();
    assert_eq!(
        result.len(),
        256,
        "the documented maximum batch must be accepted"
    );
    let calls = server.calls.lock().await;
    assert_eq!(
        calls.len(),
        32,
        "256 inputs must remain in batches of eight"
    );
    assert!(
        calls
            .iter()
            .all(|c| c.body["input"].as_array().unwrap().len() == 8)
    );
}

fn valid_response() -> Value {
    json!({"model":"gemini-embedding-2","data":[
        {"index":0,"embedding":vec![1.0;3072]},
        {"index":1,"embedding":vec![2.0;3072]}
    ]})
}

#[tokio::test]
async fn malformed_gateway_responses_fail_without_returning_partial_vectors() {
    let mut cases: Vec<(&str, Value)> = vec![];
    for (name, index) in [
        ("negative index", json!(-1)),
        ("out-of-range index", json!(2)),
        ("fractional index", json!(0.5)),
        ("string index", json!("0")),
        ("null index", Value::Null),
        ("oversized index", json!(u64::MAX)),
    ] {
        let mut response = valid_response();
        response["data"][0]["index"] = index;
        cases.push((name, response));
    }
    let mut duplicate = valid_response();
    duplicate["data"][1]["index"] = json!(0);
    cases.push(("duplicate index and missing index", duplicate));
    let mut missing_index = valid_response();
    missing_index["data"][0]
        .as_object_mut()
        .unwrap()
        .remove("index");
    cases.push(("missing index field", missing_index));
    let mut too_few = valid_response();
    too_few["data"].as_array_mut().unwrap().pop();
    cases.push(("missing vector", too_few));
    let mut too_many = valid_response();
    too_many["data"]
        .as_array_mut()
        .unwrap()
        .push(json!({"index":2,"embedding":vec![3.0;3072]}));
    cases.push(("extra vector", too_many));
    for (name, model) in [
        ("wrong model", json!("gemini-embedding-001")),
        ("null model", Value::Null),
    ] {
        let mut response = valid_response();
        response["model"] = model;
        cases.push((name, response));
    }
    let mut missing_model = valid_response();
    missing_model.as_object_mut().unwrap().remove("model");
    cases.push(("missing model", missing_model));
    for (name, vector) in [
        ("dimension drift", json!(vec![1.0; 1536])),
        ("empty vector", json!([])),
        ("zero vector", json!(vec![0.0; 3072])),
        ("not an array", json!("base64-data")),
        ("null vector", Value::Null),
    ] {
        let mut response = valid_response();
        response["data"][1]["embedding"] = vector;
        cases.push((name, response));
    }
    let mut overflow = valid_response();
    overflow["data"][1]["embedding"][0] = json!(1e39);
    cases.push(("nonfinite f32 overflow", overflow));
    let mut null_component = valid_response();
    null_component["data"][1]["embedding"][0] = Value::Null;
    cases.push(("null component", null_component));
    let mut missing_vector = valid_response();
    missing_vector["data"][1]
        .as_object_mut()
        .unwrap()
        .remove("embedding");
    cases.push(("missing embedding field", missing_vector));
    cases.push((
        "wrong response shape",
        json!({"model":"gemini-embedding-2","data":{}}),
    ));
    let files = PrivateFiles::new();
    for (name, response) in cases {
        let server = MockServer::start(Reply::Fixed(StatusCode::OK, response.to_string())).await;
        let provider = Providers::new(files.config(&server.base)).unwrap();
        assert!(
            provider
                .embed(&["1|first".into(), "2|second".into()])
                .await
                .is_err(),
            "must reject {name}"
        );
        assert_eq!(
            server.calls.lock().await.len(),
            1,
            "{name} must fail after one request without retry"
        );
    }
}

#[tokio::test]
async fn gateway_http_failures_redirects_and_invalid_json_fail_without_retry() {
    let files = PrivateFiles::new();
    for (name, status, body) in [
        ("unauthorized", StatusCode::UNAUTHORIZED, "credential-error"),
        (
            "forbidden",
            StatusCode::FORBIDDEN,
            "forbidden-response-body",
        ),
        ("rate limited", StatusCode::TOO_MANY_REQUESTS, "rate-error"),
        ("server error", StatusCode::BAD_GATEWAY, "server-error"),
        ("redirect", StatusCode::TEMPORARY_REDIRECT, "redirect-error"),
        ("invalid JSON", StatusCode::OK, "not-json"),
        (
            "nonstandard NaN",
            StatusCode::OK,
            "{\"model\":\"gemini-embedding-2\",\"data\":[{\"index\":0,\"embedding\":[NaN]}]}",
        ),
    ] {
        let server = MockServer::start(Reply::Fixed(status, body.into())).await;
        let provider = Providers::new(files.config(&server.base)).unwrap();
        let error = provider.embed(&["1|first".into()]).await.unwrap_err();
        if !status.is_success() {
            assert!(
                error
                    .to_string()
                    .contains(&format!("HTTP {}", status.as_u16())),
                "{name} must include its HTTP status code"
            );
        }
        assert!(
            !error.to_string().contains(body),
            "{name} must not expose the provider response body"
        );
        assert!(
            !error.to_string().contains(DUMMY_KEY),
            "{name} must not expose the credential"
        );
        assert_eq!(server.calls.lock().await.len(), 1, "{name} must not retry");
    }
}

#[test]
fn operational_factchat_profile_requires_pinned_endpoint_and_luna_key_only() {
    let files = PrivateFiles::new();
    let mut config = files.config(GATEWAY);
    config.mode = WriteMode::Operational;
    config.redis_url = "redis://falkordb:6379".into();
    config.gemini_key_file = None;
    config
        .validate()
        .expect("FactChat must accept the private Luna key without a Google key");
    let mut with_slash = config.clone();
    with_slash.gemini_base.push('/');
    with_slash
        .validate()
        .expect("the pinned base may include a trailing slash");
    for endpoint in [
        GOOGLE,
        "https://example.com/v1/gateway",
        "http://127.0.0.1:12345",
        "http://factchat-cloud.mindlogic.ai/v1/gateway",
    ] {
        let mut drift = config.clone();
        drift.gemini_base = endpoint.into();
        assert!(
            drift.validate().is_err(),
            "operational FactChat must reject endpoint {endpoint}"
        );
    }
    let mut missing = config.clone();
    missing.llm_key_file = None;
    assert!(
        missing.validate().is_err(),
        "operational FactChat must require the Luna key"
    );
    let mut wrong_pipeline = config.clone();
    wrong_pipeline.pipeline = IngestionPipeline::CombinedPreview;
    assert!(
        wrong_pipeline.validate().is_err(),
        "operational gateway must retain the staged pipeline guard"
    );
    let mut native = config.clone();
    native.gemini_transport = GeminiTransport::Native;
    native.gemini_base = GOOGLE.into();
    assert!(
        native.validate().is_err(),
        "native operational transport still requires the Google key"
    );
    native.gemini_key_file = Some(files.0.join("dummy.key"));
    native
        .validate()
        .expect("the unchanged native profile must remain valid");
    native.gemini_base = GATEWAY.into();
    assert!(
        native.validate().is_err(),
        "native transport must reject the FactChat base"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            files.0.join("dummy.key"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(
            config.validate().is_err(),
            "operational Luna credentials must be owner-private"
        );
    }
}
