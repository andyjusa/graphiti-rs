//! Latest upstream exact-name and classification fixes through the staged pipeline.
use super::*;
use graphiti_rs::compat::{config::EntityType, journal::Job, pipeline};

#[derive(Clone, Default)]
struct CoreMock {
    phase: Arc<AtomicUsize>,
    dedupe_calls: Arc<AtomicUsize>,
}

async fn core_llm(
    axum::extract::State(state): axum::extract::State<CoreMock>,
    axum::Json(body): axum::Json<Value>,
) -> axum::Json<Value> {
    let user = body["messages"][1]["content"].as_str().unwrap();
    let schema: Value = serde_json::from_str(user.rsplit_once("input:\n").unwrap().1).unwrap();
    let phase = state.phase.load(Ordering::SeqCst);
    let response = match schema["title"].as_str().unwrap() {
        "ExtractedEntities" if phase == 0 => json!({"extracted_entities":[
            {"name":"가온","entity_type_id":0}, {"name":"Nova","entity_type_id":0}
        ]}),
        "ExtractedEntities" => json!({"extracted_entities":[
            {"name":"가온","entity_type_id":0}, {"name":" 가온 ","entity_type_id":1},
            {"name":"Nova","entity_type_id":0}
        ]}),
        "NodeResolutions" => {
            state.dedupe_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                phase, 0,
                "exact short names should never require model deduplication"
            );
            json!({"entity_resolutions":[]})
        }
        "ExtractedEdges" if phase == 0 => json!({"edges":[]}),
        "ExtractedEdges" => json!({"edges":[{
            "source_entity_name":" 가온 ", "target_entity_name":" NOVA ",
            "relation_type":"WORKS_ON", "fact":"가온 will complete Nova by Friday.",
            "valid_at":"2026-10-23T00:00:00Z", "invalid_at":"2026-10-23T00:00:00Z"
        }]}),
        "SummarizedEntities" => {
            let nodes: Value = serde_json::from_str(tag(user, "ENTITIES").unwrap()).unwrap();
            json!({"summaries":nodes.as_array().unwrap().iter().map(|n|
                json!({"name":n["name"],"summary":"Synthetic retained classification fixture."})).collect::<Vec<_>>()})
        }
        other => panic!("unexpected mock schema {other}"),
    };
    axum::Json(json!({"choices":[{"message":{"content":response.to_string()}}]}))
}

async fn core_embeddings(axum::Json(body): axum::Json<Value>) -> axum::Json<Value> {
    axum::Json(
        json!({"embeddings":body["requests"].as_array().unwrap().iter()
        .map(|_|json!({"values":vector()})).collect::<Vec<_>>()}),
    )
}

fn core_job(group: &str, day: u8) -> Job {
    let id = uuid::Uuid::new_v4().to_string();
    Job {
        group: group.into(),
        id: id.clone(),
        created: format!("2026-10-{day:02}T00:00:00Z"),
        request: AddMemory {
            name: format!("upstream-core-{day}"),
            episode_body: "가온 works on Nova.".into(),
            group_id: Some(group.into()),
            source: "text".into(),
            source_description: String::new(),
            uuid: Some(id),
        },
        prepared: None,
    }
}

#[tokio::test]
#[ignore = "requires exclusively owned disposable FalkorDB on loopback port 16379"]
async fn exact_names_collapse_promote_and_normalize_endpoints_with_original_snapshot() {
    assert_eq!(
        std::env::var("COMPAT_TEST_DB_URL").unwrap(),
        "redis://127.0.0.1:16379"
    );
    let state = CoreMock::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route("/chat/completions", axum::routing::post(core_llm))
        .route(
            "/models/gemini-embedding-2:batchEmbedContents",
            axum::routing::post(core_embeddings),
        )
        .with_state(state.clone());
    let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut config = config(dir(), &base);
    config.entity_types = vec![EntityType {
        name: "Person".into(),
        description: "A person.".into(),
    }];
    let providers = Providers::new(config.clone()).unwrap();
    let store = LegacyStore::connect(&config.redis_url, WriteMode::Synthetic)
        .await
        .unwrap();
    let group = format!(
        "project_upstream_core_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let mut seed = pipeline::prepare(&config, &providers, &store, &core_job(&group, 14))
        .await
        .unwrap();
    seed.payload.entities[0]
        .attributes
        .insert("unknown".into(), json!("keep"));
    let planned = store.plan(&seed.payload).await.unwrap();
    assert!(store.commit(&planned).await.unwrap());
    let original = store
        .nodes(&group, "", None, 10)
        .await
        .unwrap()
        .into_iter()
        .find(|n| n.name == "가온")
        .unwrap();
    state.phase.store(1, Ordering::SeqCst);
    let prepared = pipeline::prepare(&config, &providers, &store, &core_job(&group, 15))
        .await
        .unwrap();
    assert_eq!(
        prepared.payload.entities.len(),
        2,
        "duplicate names must collapse before candidate resolution"
    );
    assert_eq!(state.dedupe_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        prepared
            .nodes_before
            .iter()
            .find(|n| n.uuid == original.uuid),
        Some(&original)
    );
    let promoted = prepared
        .payload
        .entities
        .iter()
        .find(|n| n.uuid == original.uuid)
        .unwrap();
    assert_eq!(promoted.labels, ["Entity", "Person"]);
    assert_eq!(promoted.attributes, original.attributes);
    assert_eq!(promoted.created_at, original.created_at);
    assert_eq!(
        prepared.payload.facts.len(),
        1,
        "normalized endpoint spelling must retain its fact"
    );
    assert_eq!(prepared.payload.facts[0].source_node_uuid, original.uuid);
    assert_eq!(
        prepared.payload.facts[0].valid_at.as_deref(),
        Some("2026-10-15T00:00:00Z")
    );
    assert_eq!(prepared.payload.facts[0].invalid_at, None);
    let planned = store
        .plan_observed(
            &prepared.payload,
            &prepared.nodes_before,
            &prepared.facts_before,
        )
        .await
        .unwrap();
    assert!(store.commit(&planned).await.unwrap());
    assert_eq!(
        store
            .node(&group, &original.uuid)
            .await
            .unwrap()
            .unwrap()
            .labels,
        ["Entity", "Person"]
    );

    // A real concurrent graph mutation must still invalidate the model snapshot.
    let stale = pipeline::prepare(&config, &providers, &store, &core_job(&group, 16))
        .await
        .unwrap();
    let mut concurrent = stale.payload.clone();
    let edit_job = core_job(&group, 17);
    concurrent.episode.uuid = edit_job.id;
    concurrent.episode.name = edit_job.request.name;
    concurrent.episode.created_at = edit_job.created.clone();
    concurrent.episode.valid_at = edit_job.created;
    concurrent.episode.entity_edges.clear();
    concurrent.facts.clear();
    concurrent.invalidated_fact_ids.clear();
    concurrent.entities[0].summary = "Concurrent edit.".into();
    let planned = store.plan(&concurrent).await.unwrap();
    assert!(store.commit(&planned).await.unwrap());
    let error = store
        .plan_observed(&stale.payload, &stale.nodes_before, &stale.facts_before)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("entity changed during model resolution")
    );
    store.clear(&group).await.unwrap();
    mock.abort();
}
