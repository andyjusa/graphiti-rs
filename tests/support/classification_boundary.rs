//! Classification fallback, empty descriptions and retained existing identities.
use super::*;
use graphiti_rs::compat::config::{EntityType, IngestionPipeline};

const FIRST: [&str; 6] = [
    "OrionBoundaryZero",
    "VelvetBoundaryNegative",
    "CobaltBoundaryOutside",
    "MangoBoundaryMaximum",
    "JuniperBoundaryFirst",
    "QuartzBoundaryLast",
];
const SECOND: [&str; 6] = [
    "HazelAlternateZero",
    "EmberAlternateNegative",
    "CedarAlternateOutside",
    "WillowAlternateMaximum",
    "AtlasAlternateFirst",
    "BreezeAlternateLast",
];

#[derive(Clone)]
struct BoundaryMock {
    trace: Arc<tokio::sync::Mutex<Vec<Value>>>,
    current: Arc<tokio::sync::Mutex<(Vec<EntityType>, Vec<Value>)>>,
}
async fn boundary_llm(
    axum::extract::State(s): axum::extract::State<BoundaryMock>,
    axum::Json(v): axum::Json<Value>,
) -> axum::Json<Value> {
    let user = v["messages"][1]["content"].as_str().unwrap();
    let schema: Value = serde_json::from_str(user.rsplit_once("input:\n").unwrap().1).unwrap();
    let content = match schema["title"].as_str().unwrap() {
        "ExtractedEntities" => json!({"extracted_entities":s.current.lock().await.1}),
        "NodeResolutions" => {
            let nodes: Value = serde_json::from_str(tag(user, "ENTITIES").unwrap()).unwrap();
            let existing: Value =
                serde_json::from_str(tag(user, "EXISTING ENTITIES").unwrap()).unwrap();
            let current = s.current.lock().await;
            for node in nodes.as_array().unwrap() {
                let labels = node["entity_type"].as_array().unwrap();
                let description = current
                    .0
                    .iter()
                    .find(|t| labels.contains(&json!(t.name)))
                    .map(|t| t.description.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Default Entity Type");
                assert_eq!(
                    node["entity_type_description"], description,
                    "dedup classification description differs from observed Python 3.11"
                );
            }
            json!({"entity_resolutions":nodes.as_array().unwrap().iter().map(|n| json!({"id":n["id"],"name":n["name"],"duplicate_name":existing.as_array().unwrap().iter().find(|e|e["name"]==n["name"]).map(|e|e["name"].clone()).unwrap_or(json!(""))})).collect::<Vec<_>>()})
        }
        "ExtractedEdges" => json!({"edges":[]}),
        "SummarizedEntities" => {
            let nodes: Value = serde_json::from_str(tag(user, "ENTITIES").unwrap()).unwrap();
            json!({"summaries":nodes.as_array().unwrap().iter().map(|n| json!({"name":n["name"],"summary":format!("{} is a synthetic boundary fixture.",n["name"].as_str().unwrap())})).collect::<Vec<_>>()})
        }
        other => panic!("unexpected schema {other}"),
    };
    let mut recorded = v;
    recorded["mock_response"] = content.clone();
    s.trace.lock().await.push(recorded);
    axum::Json(json!({"choices":[{"message":{"content":content.to_string()}}]}))
}
fn entities(names: &[&str], ids: &[i64]) -> Vec<Value> {
    assert_eq!(names.len(), ids.len());
    let mut values: Vec<_> = names
        .iter()
        .zip(ids)
        .map(|(name, id)| json!({"name":name,"entity_type_id":id}))
        .collect();
    values.extend([
        json!({"name":"","entity_type_id":1}),
        json!({"name":" \t ","entity_type_id":0}),
    ]);
    values
}

#[tokio::test]
#[ignore = "requires exclusively owned disposable FalkorDB on loopback port 16379"]
async fn default_invalid_and_empty_descriptions_preserve_identity_and_promote_generic_types() {
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
    let profiles = [
        (
            "configured",
            types,
            vec![
                ("message", entities(&FIRST, &[0, -1, 11, i64::MAX, 1, 10])),
                ("json", entities(&FIRST, &[1, 2, 3, 4, 0, -1])),
                ("text", entities(&SECOND, &[0, -1, 11, i64::MAX, 1, 10])),
            ],
        ),
        (
            "blank-descriptions",
            vec![
                EntityType {
                    name: "EmptyKind".into(),
                    description: "".into(),
                },
                EntityType {
                    name: "WhitespaceKind".into(),
                    description: " ".into(),
                },
            ],
            vec![(
                "message",
                entities(
                    &[
                        "NebulaDefault",
                        "SatelliteEmpty",
                        "GalaxyWhitespace",
                        "MeteorOutside",
                    ],
                    &[0, 1, 2, 3],
                ),
            )],
        ),
        (
            "unconfigured",
            vec![],
            vec![(
                "text",
                entities(
                    &[
                        "CometDefault",
                        "PlanetNegative",
                        "AsteroidOutside",
                        "StarlightMaximum",
                    ],
                    &[0, -1, 1, i64::MAX],
                ),
            )],
        ),
    ];
    let state = BoundaryMock {
        trace: Default::default(),
        current: Default::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route("/chat/completions", axum::routing::post(boundary_llm))
        .route(
            "/models/gemini-embedding-2:batchEmbedContents",
            axum::routing::post(staged_embeddings),
        )
        .with_state(state.clone());
    let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut reports = vec![];
    for (profile, types, inputs) in profiles {
        let journal = dir();
        let mut c = config(journal.clone(), &base);
        c.pipeline = IngestionPipeline::Graphiti;
        c.entity_types = types.clone();
        let engine = Engine::new(c).await.unwrap();
        let group = format!(
            "project_boundary_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let mut snapshots = vec![];
        let mut identities = std::collections::BTreeMap::new();
        let mut labels_before = std::collections::BTreeMap::new();
        for (index, (source, extracted)) in inputs.iter().enumerate() {
            *state.current.lock().await = (types.clone(), extracted.clone());
            let names: Vec<_> = extracted
                .iter()
                .filter_map(|n| n["name"].as_str())
                .filter(|n| !n.trim().is_empty())
                .collect();
            let body = if *source == "json" {
                json!({"entities":names}).to_string()
            } else {
                names.join(", ")
            };
            let id = uuid::Uuid::new_v4().to_string();
            engine.execute("add_memory",json!({"uuid":id,"name":format!("{profile}-{index}"),"source":source,"episode_body":body,"group_id":group})).await.unwrap();
            poll(&engine, &group, &id).await;
            let nodes = engine.store.nodes(&group, "", None, 100).await.unwrap();
            assert_eq!(
                nodes.len(),
                if profile == "configured" {
                    if index == 2 { 12 } else { 6 }
                } else {
                    4
                }
            );
            for extracted_node in extracted {
                let name = extracted_node["name"].as_str().unwrap();
                if name.trim().is_empty() {
                    continue;
                }
                let node = nodes.iter().find(|n| n.name == name).unwrap();
                let kind = extracted_node["entity_type_id"].as_i64().unwrap();
                let label = usize::try_from(kind)
                    .ok()
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| types.get(i))
                    .map(|t| t.name.as_str());
                let expected: std::collections::BTreeSet<String> = std::iter::once("Entity".into())
                    .chain(label.map(str::to_owned))
                    .collect();
                let observed = node
                    .labels
                    .iter()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>();
                if let Some(old) = identities.insert(name.to_owned(), node.uuid.clone()) {
                    assert_eq!(old, node.uuid);
                    // Classification-only models have one specific label. A
                    // generic identity may gain that label; an existing specific
                    // classification survives conflicting or generic extraction.
                    let prior = &labels_before[name];
                    if prior == &std::collections::BTreeSet::from(["Entity".to_owned()]) {
                        assert_eq!(
                            observed, expected,
                            "generic entity did not gain its specific type"
                        );
                    } else {
                        assert_eq!(
                            &observed, prior,
                            "conflicting or generic extraction changed a specific type"
                        );
                    }
                } else {
                    assert_eq!(observed, expected);
                }
                labels_before.insert(name.to_owned(), observed);
                assert_eq!(
                    node.summary,
                    format!("{name} is a synthetic boundary fixture.")
                );
                assert!(node.attributes.is_empty());
            }
            snapshots.push(json!({"episode":engine.store.episode(&group,&id).await.unwrap().unwrap(),"nodes":nodes,"facts":engine.store.facts(&group,"",None,100).await.unwrap()}));
        }
        reports.push(json!({"profile":profile,"entity_types":types,"snapshots":snapshots}));
        engine.store.clear(&group).await.unwrap();
        drop(engine);
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::fs::remove_dir_all(journal).unwrap();
    }
    let trace = state.trace.lock().await;
    if let Some(path) = std::env::var_os("COMPAT_BOUNDARY_REPORT") {
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
                &json!({"actual_provider_calls":0,"profiles":reports,"steps":*trace}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    println!(
        "CLASSIFICATION_BOUNDARY_REPORT {}",
        json!({"profiles":reports.len(),"episodes":5,"cumulative_entities":[6,6,12,4,4],"mock_llm_requests":trace.len(),"actual_provider_calls":0})
    );
    drop(trace);
    mock.abort();
}
