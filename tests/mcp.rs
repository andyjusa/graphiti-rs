use graphiti_rs::{config::Config, ingest::Memory, mcp::MemoryService};
use rmcp::{ServiceExt, model::CallToolRequestParams};
use std::sync::Arc;
#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn exact_keyword_precedes_common_hub_expansion_beyond_window() {
    use graphiti_rs::domain::*;
    let mut c = Config::from_env().unwrap();
    c.redis_url =
        std::env::var("TEST_FALKORDB_URL").unwrap_or_else(|_| "redis://127.0.0.1:16379".into());
    c.graph = format!("rank_{}", uuid::Uuid::new_v4().simple());
    c.embedding_provider = "disabled".into();
    c.candidates = 200;
    let memory = Memory::new(&c).await.unwrap();
    let facts: Vec<_> = (0..201)
        .map(|i| ExtractedFact {
            source: "Person".into(),
            target: "Common hub".into(),
            relation: "LINKED".into(),
            fact: format!("unique_marker_{i:06}"),
            supersedes: vec![],
        })
        .collect();
    let result = memory
        .add(AddEpisode {
            group_id: "synthetic".into(),
            episode_id: Some("rank-fixture".into()),
            content: "Synthetic shared-hub ranking fixture".into(),
            source: "test".into(),
            reference_time: "2026-10-01T00:00:00Z".parse().unwrap(),
            extraction: Some(Extraction {
                entities: vec![
                    ExtractedEntity {
                        name: "Person".into(),
                        summary: String::new(),
                    },
                    ExtractedEntity {
                        name: "Common hub".into(),
                        summary: String::new(),
                    },
                ],
                facts: facts.clone(),
            }),
        })
        .await
        .unwrap();
    // Equal timestamps put the greatest ID outside the recent 200-fact window.
    let (index, expected) = result
        .fact_ids
        .iter()
        .enumerate()
        .max_by_key(|(_, id)| *id)
        .unwrap();
    let hits = memory
        .search
        .facts(SearchRequest {
            group_id: "synthetic".into(),
            query: facts[index].fact.clone(),
            limit: 10,
            as_of: None,
        })
        .await
        .unwrap();
    assert_eq!(&hits[0].fact.id, expected);
}
#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn standard_mcp_initialization_discovery_and_calls() {
    let mut c = Config::from_env().unwrap();
    c.redis_url =
        std::env::var("TEST_FALKORDB_URL").unwrap_or_else(|_| "redis://127.0.0.1:16379".into());
    c.graph = format!("mcp_{}", uuid::Uuid::new_v4().simple());
    c.embedding_provider = "disabled".into();
    let memory = Arc::new(Memory::new(&c).await.unwrap());
    let (server_io, client_io) = tokio::io::duplex(65536);
    let task = tokio::spawn(async move {
        MemoryService::new(memory)
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    let client = ().serve(client_io).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 6);
    let episode = serde_json::json!({"group_id":"test","episode_id":"one","content":"Alice works at Acme","reference_time":"2026-10-01T00:00:00Z","extraction":{"entities":[{"name":"Alice","summary":"Person"},{"name":"Acme","summary":"Company"}],"facts":[{"source":"Alice","target":"Acme","relation":"works_at","fact":"Alice works at Acme"}]}});
    let result = client
        .call_tool(
            CallToolRequestParams::new("add_episode")
                .with_arguments(episode.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    let result=client.call_tool(CallToolRequestParams::new("search_facts").with_arguments(serde_json::json!({"group_id":"test","query":"Alice","as_of":"2026-10-01T01:00:00Z"}).as_object().unwrap().clone())).await.unwrap();
    assert!(
        serde_json::to_string(&result)
            .unwrap()
            .contains("Alice works at Acme")
    );
    let invalid = client
        .call_tool(
            CallToolRequestParams::new("search_facts").with_arguments(
                serde_json::json!({"group_id":"test","query":"Alice","limit":999})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    assert!(invalid.is_err() || invalid.unwrap().is_error == Some(true));
    client.cancel().await.unwrap();
    task.await.unwrap();
}
#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn http_requires_authentication_and_accepts_standard_initialize() {
    use tower::ServiceExt as _;
    let mut c = Config::from_env().unwrap();
    c.redis_url =
        std::env::var("TEST_FALKORDB_URL").unwrap_or_else(|_| "redis://127.0.0.1:16379".into());
    c.graph = format!("http_{}", uuid::Uuid::new_v4().simple());
    c.embedding_provider = "disabled".into();
    c.token = Some("test-secret".into());
    let memory = Arc::new(Memory::new(&c).await.unwrap());
    let app =
        graphiti_rs::http::router(memory, &c, tokio_util::sync::CancellationToken::new()).unwrap();
    let request = || {
        axum::http::Request::builder().method("POST").uri("/mcp").header("host","localhost").header("content-type","application/json").header("accept","application/json, text/event-stream").body(axum::body::Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#)).unwrap()
    };
    assert_eq!(app.clone().oneshot(request()).await.unwrap().status(), 401);
    let mut authorized = request();
    authorized
        .headers_mut()
        .insert("authorization", "Bearer test-secret".parse().unwrap());
    let response = app.oneshot(authorized).await.unwrap();
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 262144)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("graphiti-rs"));
}
#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn retry_does_not_call_model_again() {
    let mut c = Config::from_env().unwrap();
    c.redis_url =
        std::env::var("TEST_FALKORDB_URL").unwrap_or_else(|_| "redis://127.0.0.1:16379".into());
    c.graph = format!("retry_{}", uuid::Uuid::new_v4().simple());
    c.embedding_provider = "disabled".into();
    c.api_base = "http://127.0.0.1:1".into();
    let memory = Memory::new(&c).await.unwrap();
    let mut request:graphiti_rs::domain::AddEpisode=serde_json::from_value(serde_json::json!({"group_id":"g","episode_id":"one","content":"Alice","reference_time":"2026-10-01T00:00:00Z","extraction":{"entities":[{"name":"Alice"}],"facts":[]}})).unwrap();
    assert!(memory.add(request.clone()).await.unwrap().inserted);
    request.extraction = None;
    assert!(!memory.add(request.clone()).await.unwrap().inserted);
    request.content = "Different".into();
    assert!(memory.add(request).await.is_err());
}
#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn concurrent_searches_are_rejected_before_queueing() {
    use std::sync::Arc;
    let started = Arc::new(tokio::sync::Barrier::new(3));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let app = axum::Router::new().route(
        "/embeddings",
        axum::routing::post({
            let started = started.clone();
            let release = release.clone();
            move || {
                let started = started.clone();
                let release = release.clone();
                async move {
                    started.wait().await;
                    let _permit = release.acquire().await.unwrap();
                    axum::Json(serde_json::json!({"data":[{"index":0,"embedding":[1.0,0.0]}]}))
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut c = Config::from_env().unwrap();
    c.redis_url =
        std::env::var("TEST_FALKORDB_URL").unwrap_or_else(|_| "redis://127.0.0.1:16379".into());
    c.graph = format!("bounded_{}", uuid::Uuid::new_v4().simple());
    c.embedding_provider = "external".into();
    c.embedding_dim = 2;
    c.api_base = format!("http://{address}");
    let memory = Memory::new(&c).await.unwrap();
    let request = graphiti_rs::domain::SearchRequest {
        group_id: "g".into(),
        query: "Alice".into(),
        limit: 10,
        as_of: None,
    };
    let mut tasks = vec![];
    for _ in 0..2 {
        let search = memory.search.clone();
        let request = request.clone();
        tasks.push(tokio::spawn(async move { search.facts(request).await }));
    }
    // Both accepted calls reach the model and retain their permits.
    tokio::time::timeout(std::time::Duration::from_secs(5), started.wait())
        .await
        .unwrap();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        memory.search.facts(request),
    )
    .await;
    assert!(outcome.is_ok(), "extra search queued instead of rejecting");
    assert!(
        outcome
            .unwrap()
            .err()
            .expect("busy error")
            .to_string()
            .contains("busy")
    );
    release.add_permits(2);
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn semantic_search_finds_old_fact_outside_recent_window() {
    use graphiti_rs::domain::*;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new().route(
        "/embeddings",
        axum::routing::post(|| async {
            axum::Json(serde_json::json!({"data":[{"index":0,"embedding":[0.,1.]}]}))
        }),
    );
    let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut c = Config::from_env().unwrap();
    c.redis_url =
        std::env::var("TEST_FALKORDB_URL").unwrap_or_else(|_| "redis://127.0.0.1:16379".into());
    c.graph = format!("semantic_mcp_{}", uuid::Uuid::new_v4().simple());
    c.api_base = base;
    c.api_key = None;
    c.embedding_provider = "external".into();
    c.embedding_model = "synthetic".into();
    c.embedding_dim = 2;
    c.candidates = 50;
    let memory = Memory::new(&c).await.unwrap();
    let ep = |id: &str, time: &str| Episode {
        id: id.into(),
        group_id: "synthetic".into(),
        content: id.into(),
        source: "fixture".into(),
        reference_time: time.parse().unwrap(),
        created_at: chrono::Utc::now(),
    };
    let mut x = Extraction {
        entities: vec![
            ExtractedEntity {
                name: "Source".into(),
                summary: String::new(),
            },
            ExtractedEntity {
                name: "Hub".into(),
                summary: String::new(),
            },
        ],
        facts: vec![ExtractedFact {
            source: "Source".into(),
            target: "Hub".into(),
            relation: "LINK".into(),
            fact: "A cyclist on a road".into(),
            supersedes: vec![],
        }],
    };
    let target = memory
        .store
        .ingest(
            &ep("old", "2026-10-01T00:00:00Z"),
            &x,
            Some(&[vec![0., 1.]]),
            memory.embedder.space().as_deref(),
        )
        .await
        .unwrap()
        .fact_ids[0]
        .clone();
    x.facts = (0..51)
        .map(|i| ExtractedFact {
            source: "Source".into(),
            target: "Hub".into(),
            relation: "LINK".into(),
            fact: format!("unrelated {i}"),
            supersedes: vec![],
        })
        .collect();
    memory
        .store
        .ingest(
            &ep("recent", "2026-10-02T00:00:00Z"),
            &x,
            Some(&vec![vec![1., 0.]; 51]),
            memory.embedder.space().as_deref(),
        )
        .await
        .unwrap();
    let hits = memory
        .search
        .facts(SearchRequest {
            group_id: "synthetic".into(),
            query: "Someone rides a bike".into(),
            limit: 10,
            as_of: Some("2026-10-02T01:00:00Z".parse().unwrap()),
        })
        .await
        .unwrap();
    assert_eq!(hits[0].fact.id, target);
    mock.abort();
}
