//! Multiple relation reuse/invalidation in one episode, including temporal history.
use super::*;
use graphiti_rs::compat::config::{EntityType, IngestionPipeline};

#[derive(Clone, Default)]
struct RelationMock {
    reuse_first: bool,
    trace: Arc<tokio::sync::Mutex<Vec<Value>>>,
}
fn relation(source: &str, target: &str, name: &str, fact: &str, time: &str) -> Value {
    json!({"source_entity_name":source,"target_entity_name":target,"relation_type":name,"fact":fact,"valid_at":time,"invalid_at":null})
}
fn stage(text: &str) -> usize {
    (0..4)
        .find(|i| text.contains(&format!("RELATION_STAGE_{i}")))
        .unwrap()
}
async fn relation_llm(
    axum::extract::State(s): axum::extract::State<RelationMock>,
    axum::Json(v): axum::Json<Value>,
) -> axum::Json<Value> {
    let user = v["messages"][1]["content"].as_str().unwrap();
    let schema: Value = serde_json::from_str(user.rsplit_once("input:\n").unwrap().1).unwrap();
    let content = match schema["title"].as_str().unwrap() {
        "ExtractedEntities" => {
            let current = tag(user, "CURRENT MESSAGE")
                .or_else(|| tag(user, "JSON"))
                .or_else(|| tag(user, "TEXT"))
                .unwrap();
            let phase = stage(current);
            json!({"extracted_entities":[{"name":if phase==1{"G."}else{"Gaon"},"entity_type_id":1},{"name":"Nari","entity_type_id":1},{"name":"Saebit","entity_type_id":2},{"name":"Hanul","entity_type_id":2},{"name":"Delta","entity_type_id":2},{"name":"Cedar","entity_type_id":2}]})
        }
        "NodeResolutions" => {
            let nodes: Value = serde_json::from_str(tag(user, "ENTITIES").unwrap()).unwrap();
            let old: Value = serde_json::from_str(tag(user, "EXISTING ENTITIES").unwrap()).unwrap();
            json!({"entity_resolutions":nodes.as_array().unwrap().iter().map(|n|{let name=n["name"].as_str().unwrap();let lookup=if name=="G."{"Gaon"}else{name};json!({"id":n["id"],"name":name,"duplicate_name":old.as_array().unwrap().iter().find(|e|e["name"]==lookup).map(|e|e["name"].clone()).unwrap_or(json!(""))})}).collect::<Vec<_>>()})
        }
        "ExtractedEdges" => {
            let mut edges = match stage(tag(user, "CURRENT_MESSAGE").unwrap()) {
                0 => vec![
                    relation(
                        "Gaon",
                        "Saebit",
                        "WORKS_AT",
                        "Gaon works at Saebit.",
                        "2026-09-01T00:00:00Z",
                    ),
                    relation(
                        "Nari",
                        "Saebit",
                        "WORKS_AT",
                        "Nari works at Saebit.",
                        "2026-09-01T00:00:00Z",
                    ),
                    relation(
                        "Gaon",
                        "Nari",
                        "MENTORS",
                        "Gaon mentors Nari.",
                        "2026-09-01T00:00:00Z",
                    ),
                    relation(
                        "Saebit",
                        "Delta",
                        "PARTNERS",
                        "Saebit partners with Delta.",
                        "2026-09-01T00:00:00Z",
                    ),
                    relation(
                        "Nari",
                        "Cedar",
                        "VOLUNTEERS",
                        "Nari volunteers at Cedar.",
                        "2026-09-01T00:00:00Z",
                    ),
                    relation(
                        "Gaon",
                        "Gaon",
                        "REFLECTS",
                        "Gaon reflects on Gaon.",
                        "2026-09-01T00:00:00Z",
                    ),
                ],
                1 => vec![
                    relation(
                        "G.",
                        "Saebit",
                        "WORKS_AT",
                        "Gaon is employed at Saebit.",
                        "2026-09-01T00:00:00Z",
                    ),
                    relation(
                        "G.",
                        "Saebit",
                        "WORKS_AT",
                        "Gaon receives a salary from Saebit.",
                        "2026-09-01T00:00:00Z",
                    ),
                    relation(
                        "Nari",
                        "Saebit",
                        "WORKS_AT",
                        "Nari works at Saebit.",
                        "2026-09-01T00:00:00Z",
                    ),
                ],
                2 => vec![
                    relation(
                        "Gaon",
                        "Hanul",
                        "WORKS_AT",
                        "Gaon works at Hanul.",
                        "2026-09-10T00:00:00Z",
                    ),
                    relation(
                        "Nari",
                        "Hanul",
                        "WORKS_AT",
                        "Nari works at Hanul.",
                        "2026-09-12T00:00:00Z",
                    ),
                    relation(
                        "Hanul",
                        "Delta",
                        "PARTNERS",
                        "Hanul partners with Delta.",
                        "2026-09-10T00:00:00Z",
                    ),
                    relation(
                        "Gaon",
                        "Saebit",
                        "WORKS_AT",
                        "Gaon supports Saebit as an employee.",
                        "2026-09-01T00:00:00Z",
                    ),
                ],
                3 => vec![
                    relation(
                        "Gaon",
                        "Delta",
                        "WORKS_AT",
                        "Gaon formerly worked at Delta.",
                        "2026-09-05T00:00:00Z",
                    ),
                    relation(
                        "Nari",
                        "Delta",
                        "WORKS_AT",
                        "Nari formerly worked at Delta.",
                        "2026-09-06T00:00:00Z",
                    ),
                    relation(
                        "Gaon",
                        "Nari",
                        "MENTORS",
                        "Gaon mentors Nari.",
                        "2026-09-01T00:00:00Z",
                    ),
                    relation(
                        "Gaon",
                        "Gaon",
                        "REFLECTS",
                        "Gaon reflects on Gaon.",
                        "2026-09-01T00:00:00Z",
                    ),
                ],
                _ => unreachable!(),
            };
            if stage(tag(user, "CURRENT_MESSAGE").unwrap()) == 2 && s.reuse_first {
                edges.rotate_right(1);
            }
            if stage(tag(user, "CURRENT_MESSAGE").unwrap()) == 0 {
                edges.push(relation(
                    "Gaon",
                    "Saebit",
                    "WORKS_AT",
                    "  Gaon   works at Saebit. ",
                    "2026-09-01T00:00:00Z",
                ));
                edges.push(relation(
                    "Unknown",
                    "Delta",
                    "UNKNOWN",
                    "Invalid missing entity.",
                    "2026-09-01T00:00:00Z",
                ));
                edges.push(relation(
                    "Gaon",
                    "Delta",
                    "EMPTY",
                    "  ",
                    "2026-09-01T00:00:00Z",
                ));
            }
            json!({"edges":edges})
        }
        "EdgeDuplicate" => {
            let new = tag(user, "NEW FACT").unwrap();
            let old: Value =
                serde_json::from_str(&tag(user, "EXISTING FACTS").unwrap().replace('\'', "\""))
                    .unwrap();
            let candidates: Value = serde_json::from_str(
                &tag(user, "FACT INVALIDATION CANDIDATES")
                    .unwrap()
                    .replace('\'', "\""),
            )
            .unwrap();
            let duplicate = if [
                "Gaon is employed at Saebit.",
                "Gaon receives a salary from Saebit.",
                "Gaon supports Saebit as an employee.",
            ]
            .contains(&new)
            {
                Some("Gaon works at Saebit.")
            } else {
                None
            };
            let contradiction = match new {
                "Gaon works at Hanul." => Some("Gaon works at Saebit."),
                "Nari works at Hanul." => Some("Nari works at Saebit."),
                "Hanul partners with Delta." => Some("Saebit partners with Delta."),
                "Gaon formerly worked at Delta." => Some("Gaon works at Hanul."),
                "Nari formerly worked at Delta." => Some("Nari works at Hanul."),
                _ => None,
            };
            let dup: Vec<_> = duplicate
                .into_iter()
                .map(|fact| {
                    old.as_array()
                        .unwrap()
                        .iter()
                        .find(|e| e["fact"] == fact)
                        .expect("required duplicate candidate missing")["idx"]
                        .clone()
                })
                .collect();
            let contra: Vec<_> = contradiction
                .into_iter()
                .map(|fact| {
                    old.as_array()
                        .unwrap()
                        .iter()
                        .chain(candidates.as_array().unwrap())
                        .find(|e| e["fact"] == fact)
                        .expect("required invalidation candidate missing")["idx"]
                        .clone()
                })
                .collect();
            json!({"duplicate_facts":dup,"contradicted_facts":contra})
        }
        "SummarizedEntities" => {
            let nodes: Value = serde_json::from_str(tag(user, "ENTITIES").unwrap()).unwrap();
            json!({"summaries":nodes.as_array().unwrap().iter().map(|n|json!({"name":n["name"],"summary":format!("{} is a synthetic relationship fixture.",n["name"].as_str().unwrap())})).collect::<Vec<_>>()})
        }
        other => panic!("unexpected schema {other}"),
    };
    let mut recorded = v;
    recorded["mock_response"] = content.clone();
    s.trace.lock().await.push(recorded);
    axum::Json(json!({"choices":[{"message":{"content":content.to_string()}}]}))
}
#[tokio::test]
#[ignore = "requires exclusively owned disposable FalkorDB on loopback port 16379"]
async fn interacting_reuse_invalidation_alias_self_loop_and_historical_relations() {
    interacting_case(false).await;
}
#[tokio::test]
#[ignore = "requires exclusively owned disposable FalkorDB on loopback port 16379"]
async fn overlap_order_reuse_before_invalidation() {
    interacting_case(true).await;
}
async fn interacting_case(reuse_first: bool) {
    assert_eq!(
        std::env::var("COMPAT_TEST_DB_URL").unwrap(),
        "redis://127.0.0.1:16379"
    );
    let state = RelationMock {
        reuse_first,
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route("/chat/completions", axum::routing::post(relation_llm))
        .route(
            "/models/gemini-embedding-2:batchEmbedContents",
            axum::routing::post(staged_embeddings),
        )
        .with_state(state.clone());
    let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let journal = dir();
    let mut c = config(journal.clone(), &base);
    c.pipeline = IngestionPipeline::Graphiti;
    c.entity_types = vec![
        EntityType {
            name: "Person".into(),
            description: "A person".into(),
        },
        EntityType {
            name: "Organization".into(),
            description: "A company".into(),
        },
    ];
    let engine = Engine::new(c).await.unwrap();
    let group = format!(
        "project_interacting_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let mut snapshots = vec![];
    let mut identity = std::collections::BTreeMap::new();
    for (index, source) in ["message", "json", "text", "message"].iter().enumerate() {
        let marker = format!("RELATION_STAGE_{index}");
        let body = if *source == "json" {
            json!({"stage":marker}).to_string()
        } else {
            marker
        };
        let id = uuid::Uuid::new_v4().to_string();
        engine.execute("add_memory",json!({"uuid":id,"name":format!("interacting-{index}"),"source":source,"episode_body":body,"group_id":group})).await.unwrap();
        poll(&engine, &group, &id).await;
        let nodes = engine.store.nodes(&group, "", None, 100).await.unwrap();
        let facts = engine.store.facts(&group, "", None, 100).await.unwrap();
        assert_eq!(nodes.len(), 6);
        assert_eq!(facts.len(), [6, 6, 9, 11][index]);
        for node in &nodes {
            if let Some(old) = identity.insert(node.name.clone(), node.uuid.clone()) {
                assert_eq!(old, node.uuid, "alias changed identity");
            }
        }
        let episode = engine.store.episode(&group, &id).await.unwrap().unwrap();
        assert_eq!(episode.entity_edges.len(), [6, 3, 7, 4][index]);
        if index == 1 {
            assert_eq!(episode.entity_edges[0], episode.entity_edges[1]);
        }
        if index == 2 {
            let old = facts
                .iter()
                .find(|f| f.fact == "Gaon works at Saebit.")
                .unwrap();
            if !reuse_first {
                assert!(old.invalid_at.is_none());
                assert!(old.expired_at.is_none());
                assert!(
                    old.episodes.contains(&id),
                    "reused fact must retain support"
                );
            }
            for (fact, until) in [
                ("Nari works at Saebit.", "2026-09-12T00:00:00Z"),
                ("Saebit partners with Delta.", "2026-09-10T00:00:00Z"),
            ] {
                let invalidated = facts.iter().find(|f| f.fact == fact).unwrap();
                assert_eq!(invalidated.invalid_at.as_deref(), Some(until));
                assert!(invalidated.expired_at.is_some());
                assert!(!invalidated.episodes.contains(&id));
            }
            assert_eq!(
                episode.entity_edges[if reuse_first { 0 } else { 3 }],
                episode.entity_edges[4]
            );
        }
        if index == 3 {
            for (fact, until) in [
                ("Gaon formerly worked at Delta.", "2026-09-10T00:00:00Z"),
                ("Nari formerly worked at Delta.", "2026-09-12T00:00:00Z"),
            ] {
                let old = facts.iter().find(|f| f.fact == fact).unwrap();
                assert_eq!(old.invalid_at.as_deref(), Some(until));
                assert!(old.expired_at.is_some());
            }
        }
        let calls = state.trace.lock().await.len();
        engine.execute("add_memory", json!({"uuid":id,"name":format!("interacting-{index}"),"source":source,"episode_body":body,"group_id":group})).await.unwrap();
        assert_eq!(
            state.trace.lock().await.len(),
            calls,
            "receipt replay invoked model"
        );
        snapshots.push(json!({"episode":engine.store.episode(&group,&id).await.unwrap().unwrap(),"nodes":nodes,"facts":facts}));
    }
    if let Some(path) = std::env::var_os("COMPAT_RELATION_REPORT") {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        f.write_all(&serde_json::to_vec_pretty(&json!({"actual_provider_calls":0,"fixture_order":if reuse_first{"reuse_first"}else{"reuse_last"},"snapshots":snapshots,"steps":*state.trace.lock().await})).unwrap()).unwrap();
    }
    println!(
        "INTERACTING_RELATION_REPORT {}",
        json!({"episodes":4,"entities":6,"fixture_order":if reuse_first{"reuse_first"}else{"reuse_last"},"cumulative_facts":[6,6,9,11],"mock_llm_requests":state.trace.lock().await.len(),"actual_provider_calls":0})
    );
    engine.store.clear(&group).await.unwrap();
    drop(engine);
    mock.abort();
    tokio::time::sleep(Duration::from_millis(100)).await;
    std::fs::remove_dir_all(journal).unwrap();
}
