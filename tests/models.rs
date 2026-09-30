use graphiti_rs::{embeddings::*, models::parse_extraction};
#[test]
fn extracts_valid_structured_response() {
    let x = parse_extraction(r#"{"entities":[{"name":"Alice","summary":"Person"}],"facts":[]}"#)
        .unwrap();
    assert_eq!(x.entities[0].name, "Alice");
}
#[test]
fn invalid_extraction_does_not_pass() {
    assert!(parse_extraction(r#"{"entities":[],"facts":[{"source":"A","target":"B","relation":"knows","fact":"A knows B"}]}"#).is_err());
}
#[test]
fn validates_vector_shape_and_finiteness() {
    assert!(validate_vectors(&[vec![1.0]], 1, 2).is_err());
    assert!(validate_vectors(&[vec![f32::NAN, 0.0]], 1, 2).is_err());
    assert!(validate_vectors(&[], 1, 2).is_err());
    assert!(validate_vectors(&[vec![1.0, 0.0]], 1, 2).is_ok());
}
#[test]
fn cosine_checks_dimensions_and_normalizes() {
    assert!(cosine(&[1.0], &[1.0, 0.0]).is_err());
    assert!((cosine(&[2.0, 0.0], &[5.0, 0.0]).unwrap() - 1.0).abs() < 1e-6);
    assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]).unwrap(), 0.0);
}
#[cfg(feature = "local-embeddings")]
#[test]
fn local_model_missing_files_is_actionable() {
    let e = graphiti_rs::embeddings::local::LocalEmbedder::load("/does-not-exist", "test", 384)
        .err()
        .unwrap();
    assert!(e.to_string().contains("config.json"));
}
#[cfg(feature = "local-embeddings")]
#[test]
#[ignore = "requires explicit LOCAL_TEST_MODEL_DIR"]
fn real_local_model_produces_semantic_vectors() {
    let dir = std::env::var("LOCAL_TEST_MODEL_DIR").expect("set LOCAL_TEST_MODEL_DIR");
    let backend =
        graphiti_rs::embeddings::local::LocalEmbedder::load(&dir, "all-MiniLM-L6-v2", 384).unwrap();
    let v = backend
        .embed(&[
            "A person is riding a bicycle".into(),
            "Someone rides a bike".into(),
            "Database connection timed out".into(),
        ])
        .unwrap();
    assert_eq!(v.len(), 3);
    assert_eq!(v[0].len(), 384);
    let related = cosine(&v[0], &v[1]).unwrap();
    let unrelated = cosine(&v[0], &v[2]).unwrap();
    assert!(related > 0.5);
    assert!(related > unrelated + 0.2);
}
#[tokio::test]
async fn api_client_validates_actual_http_response() {
    use graphiti_rs::{config::Config, models::ModelApi};
    let app=axum::Router::new().route("/chat/completions",axum::routing::post(||async{axum::Json(serde_json::json!({"choices":[{"message":{"content":"{\"entities\":[{\"name\":\"Alice\"}],\"facts\":[]}"}}]}))}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut c = Config::from_env().unwrap();
    c.api_base = format!("http://{address}");
    c.api_key = None;
    let api = ModelApi::new(&c).unwrap();
    let episode = graphiti_rs::domain::Episode {
        id: "one".into(),
        group_id: "g".into(),
        content: "Alice".into(),
        source: "test".into(),
        reference_time: chrono::Utc::now(),
        created_at: chrono::Utc::now(),
    };
    assert_eq!(
        api.extract(&episode, &[]).await.unwrap().entities[0].name,
        "Alice"
    );
    server.abort();
}
#[tokio::test]
async fn embedding_client_rejects_duplicate_response_indices() {
    use graphiti_rs::{config::Config, embeddings::Embedder};
    let app=axum::Router::new().route("/embeddings",axum::routing::post(||async{axum::Json(serde_json::json!({"data":[{"index":0,"embedding":[1.0,0.0]},{"index":0,"embedding":[0.0,1.0]}]}))}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut c = Config::from_env().unwrap();
    c.api_base = format!("http://{address}");
    c.api_key = None;
    c.embedding_provider = "external".into();
    c.embedding_dim = 2;
    let embedder = Embedder::new(&c).await.unwrap();
    assert!(
        embedder
            .embed(&["one".into(), "two".into()])
            .await
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    server.abort();
}
#[tokio::test]
async fn external_embedding_spaces_distinguish_api_endpoints() {
    let mut c = graphiti_rs::config::Config::from_env().unwrap();
    c.embedding_provider = "external".into();
    c.api_base = "http://127.0.0.1:10001/v1".into();
    let a = graphiti_rs::embeddings::Embedder::new(&c)
        .await
        .unwrap()
        .space();
    c.api_base = "http://127.0.0.1:10002/v1".into();
    let b = graphiti_rs::embeddings::Embedder::new(&c)
        .await
        .unwrap()
        .space();
    assert_ne!(a, b);
}
#[cfg(feature = "local-embeddings")]
#[test]
#[ignore = "requires explicit LOCAL_TEST_MODEL_DIR"]
fn local_space_changes_when_tokenizer_changes() {
    let dir = std::path::PathBuf::from(std::env::var("LOCAL_TEST_MODEL_DIR").unwrap());
    let first =
        graphiti_rs::embeddings::local::LocalEmbedder::load(dir.to_str().unwrap(), "minilm", 384)
            .unwrap()
            .space();
    let temp = std::env::temp_dir().join(format!("graphiti-tokenizer-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    std::fs::copy(dir.join("config.json"), temp.join("config.json")).unwrap();
    std::fs::hard_link(
        dir.join("model.safetensors"),
        temp.join("model.safetensors"),
    )
    .unwrap();
    let mut tokenizer: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("tokenizer.json")).unwrap()).unwrap();
    tokenizer["normalizer"]["lowercase"] = serde_json::json!(false);
    std::fs::write(
        temp.join("tokenizer.json"),
        serde_json::to_vec(&tokenizer).unwrap(),
    )
    .unwrap();
    let second =
        graphiti_rs::embeddings::local::LocalEmbedder::load(temp.to_str().unwrap(), "minilm", 384)
            .unwrap()
            .space();
    std::fs::remove_dir_all(temp).unwrap();
    assert_ne!(first, second);
}
