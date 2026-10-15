//! Ten classification-only models; an optional private profile stays outside git.
use super::*;
use graphiti_rs::compat::config::{EntityType, IngestionPipeline};

const FIRST: [&str; 10] = [
    "Orion", "Velvet", "Cobalt", "Mango", "Juniper", "Quartz", "Hazel", "Ember", "Cedar", "Willow",
];
const LAST: [&str; 10] = [
    "Atlas", "Breeze", "Copper", "Dahlia", "Echo", "Fable", "Garnet", "Harbor", "Iris", "Jasmine",
];

#[derive(Clone)]
struct ProfileMock {
    trace: Arc<tokio::sync::Mutex<Vec<Value>>>,
    types: Arc<Vec<EntityType>>,
}

async fn profile_llm(
    axum::extract::State(s): axum::extract::State<ProfileMock>,
    axum::Json(v): axum::Json<Value>,
) -> axum::Json<Value> {
    let user = v["messages"][1]["content"].as_str().unwrap();
    let schema: Value = serde_json::from_str(user.rsplit_once("input:\n").unwrap().1).unwrap();
    let content = match schema["title"].as_str().unwrap() {
        "ExtractedEntities" => {
            let types = tag(user, "ENTITY TYPES").unwrap();
            assert_eq!(types.matches("entity_type_id").count(), 11);
            for kind in s.types.iter() {
                assert!(types.contains(&kind.name));
            }
            let current = tag(user, "CURRENT MESSAGE")
                .or_else(|| tag(user, "JSON"))
                .or_else(|| tag(user, "TEXT"))
                .unwrap();
            let names = if current.contains("Atlas") {
                LAST
            } else {
                FIRST
            };
            let reverse = tag(user, "JSON").is_some();
            json!({"extracted_entities":names.iter().enumerate().map(|(i,name)|
                json!({"name":name,"entity_type_id":if reverse {10-i} else {i+1}})).collect::<Vec<_>>()})
        }
        "NodeResolutions" => {
            let nodes: Value = serde_json::from_str(tag(user, "ENTITIES").unwrap()).unwrap();
            let existing: Value =
                serde_json::from_str(tag(user, "EXISTING ENTITIES").unwrap()).unwrap();
            for node in nodes.as_array().unwrap() {
                let labels = node["entity_type"].as_array().unwrap();
                let kind = s
                    .types
                    .iter()
                    .find(|t| labels.contains(&json!(t.name)))
                    .unwrap();
                assert_eq!(node["entity_type_description"], kind.description);
            }
            json!({"entity_resolutions":nodes.as_array().unwrap().iter().map(|n|
                json!({"id":n["id"],"name":n["name"],"duplicate_name":existing.as_array().unwrap().iter().find(|e|e["name"]==n["name"]).map(|e|e["name"].clone()).unwrap_or(json!(""))})).collect::<Vec<_>>()})
        }
        "ExtractedEdges" => json!({"edges":[]}),
        "SummarizedEntities" => {
            let nodes: Value = serde_json::from_str(tag(user, "ENTITIES").unwrap()).unwrap();
            json!({"summaries":nodes.as_array().unwrap().iter().map(|n|
                json!({"name":n["name"],"summary":format!("{} is a synthetic classification fixture.",n["name"].as_str().unwrap())})).collect::<Vec<_>>()})
        }
        other => panic!("unexpected schema {other}"),
    };
    let mut recorded = v;
    recorded["mock_response"] = content.clone();
    s.trace.lock().await.push(recorded);
    axum::Json(json!({"choices":[{"message":{"content":content.to_string()}}]}))
}

#[tokio::test]
#[ignore = "requires dedicated disposable FalkorDB on loopback port 16379"]
async fn ten_configured_classifications_message_json_text_and_existing_identity() {
    assert_eq!(
        std::env::var("COMPAT_TEST_DB_URL").unwrap(),
        "redis://127.0.0.1:16379"
    );
    let types: Vec<EntityType> = match std::env::var_os("COMPAT_PROFILE_TYPES_FILE") {
        Some(path) => serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap(),
        None => (1..=10)
            .map(|i| EntityType {
                name: format!("FixtureType{i}"),
                description: format!("Synthetic classification {i}"),
            })
            .collect(),
    };
    assert_eq!(types.len(), 10);
    let state = ProfileMock {
        trace: Default::default(),
        types: Arc::new(types),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route("/chat/completions", axum::routing::post(profile_llm))
        .route(
            "/models/gemini-embedding-2:batchEmbedContents",
            axum::routing::post(staged_embeddings),
        )
        .with_state(state.clone());
    let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let journal = dir();
    let mut c = config(journal.clone(), &base);
    c.pipeline = IngestionPipeline::Graphiti;
    c.entity_types = (*state.types).clone();
    let engine = Engine::new(c).await.unwrap();
    let group = "project_profile_pipeline_20261002";
    engine.store.clear(group).await.unwrap();
    let inputs = [
        ("message", FIRST.join(", ")),
        ("json", json!({"entities":FIRST}).to_string()),
        ("text", LAST.join(", ")),
    ];
    let mut snapshots = vec![];
    let mut identities = std::collections::BTreeMap::new();
    for (i, (source, body)) in inputs.iter().enumerate() {
        let id = uuid::Uuid::new_v4().to_string();
        engine.execute("add_memory", json!({"uuid":id,"name":format!("profile-{i}"),"episode_body":body,"source":source,"group_id":group})).await.unwrap();
        poll(&engine, group, &id).await;
        let nodes = engine.store.nodes(group, "", None, 100).await.unwrap();
        assert_eq!(nodes.len(), if i == 2 { 20 } else { 10 });
        for names in if i == 2 {
            vec![FIRST, LAST]
        } else {
            vec![FIRST]
        } {
            for (index, name) in names.iter().enumerate() {
                let node = nodes.iter().find(|n| n.name == *name).unwrap();
                assert_eq!(
                    node.labels
                        .iter()
                        .cloned()
                        .collect::<std::collections::BTreeSet<_>>(),
                    ["Entity".to_owned(), state.types[index].name.clone()]
                        .into_iter()
                        .collect()
                );
                assert!(node.attributes.is_empty());
                assert_eq!(
                    node.summary,
                    format!("{name} is a synthetic classification fixture.")
                );
                if let Some(old) = identities.insert(node.name.clone(), node.uuid.clone()) {
                    assert_eq!(old, node.uuid, "reclassification replaced identity");
                }
            }
        }
        snapshots.push(json!({"episode":engine.store.episode(group,&id).await.unwrap().unwrap(),"nodes":nodes,"facts":engine.store.facts(group,"",None,100).await.unwrap()}));
    }
    let trace = state.trace.lock().await;
    // The second episode reuses exact short names without another dedup call.
    // Its conflicting classification IDs must still leave canonical labels alone.
    let dedupe_calls = trace
        .iter()
        .filter(|request| {
            let user = request["messages"][1]["content"].as_str().unwrap();
            let schema: Value =
                serde_json::from_str(user.rsplit_once("input:\n").unwrap().1).unwrap();
            schema["title"] == "NodeResolutions"
        })
        .count();
    assert_eq!(dedupe_calls, 2);
    assert_eq!(trace.len(), 10);
    if let Some(path) = std::env::var_os("COMPAT_PROFILE_REPORT") {
        // Report includes private labels/descriptions when a private profile is supplied.
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(
            &serde_json::to_vec_pretty(
                &json!({"actual_provider_calls":0,"snapshots":snapshots,"steps":*trace}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    drop(trace);
    engine.store.clear(group).await.unwrap();
    drop(engine);
    mock.abort();
    tokio::time::sleep(Duration::from_millis(100)).await;
    std::fs::remove_dir_all(journal).unwrap();
}
