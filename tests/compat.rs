#![cfg(feature = "graphiti-compat")]
#[path = "support/classification_boundary.rs"]
mod classification_boundary;
#[path = "support/configured_profile.rs"]
mod configured_profile;
#[path = "support/interacting_relations.rs"]
mod interacting_relations;
#[path = "support/worker_failure.rs"]
mod worker_failure;
use graphiti_rs::compat::{
    config::{CompatConfig, WriteMode},
    dto::*,
    engine::{Engine, episode_public, fact_public, node_public, round_robin},
    journal::Journal,
    mcp::{CompatService, response, shape, tools},
    policy,
    providers::{Providers, validate_vector},
    store::LegacyStore,
};
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

fn dir() -> PathBuf {
    std::env::temp_dir().join(format!("graphiti-rs-compat-{}", uuid::Uuid::new_v4()))
}
#[test]
fn prepared_episode_references_preserve_multiplicity_with_bounded_valid_targets() {
    let mut p = fixture("user_primary");
    p.episode.entity_edges.push(p.facts[0].uuid.clone());
    p.validate().unwrap();
    let mut unknown = p.clone();
    unknown
        .episode
        .entity_edges
        .push(uuid::Uuid::new_v4().to_string());
    assert!(unknown.validate().is_err());
    let mut missing = p.clone();
    missing.episode.entity_edges.clear();
    assert!(missing.validate().is_err());
    p.episode.entity_edges = vec![p.facts[0].uuid.clone(); MAX_EPISODE_FACT_REFERENCES + 1];
    assert!(p.validate().is_err());
}
#[test]
fn fulltext_index_readiness_rejects_incomplete_or_unknown_status() {
    let ready = graphiti_rs::compat::fulltext::require_operational;
    ready("OPERATIONAL").unwrap();
    for status in ["[Indexing] 5/1200: UNDER CONSTRUCTION", "", "UNKNOWN"] {
        assert!(ready(status).unwrap_err().to_string().contains("not ready"));
    }
}
fn request(id: &str) -> AddMemory {
    serde_json::from_value(json!({"uuid":id,"name":"synthetic","episode_body":"Gaon works at Saebit","group_id":"user_primary"})).unwrap()
}
fn fixture(group: &str) -> Prepared {
    let x: Value = serde_json::from_str(include_str!("fixtures/compat-cases.json")).unwrap();
    let x = &x["fixture"];
    let mut ep = x["episode"].clone();
    ep["group_id"] = json!(group);
    ep["entity_edges"] = json!([x["fact"]["uuid"]]);
    let episode: LegacyEpisode = serde_json::from_value(ep).unwrap();
    let entities: Vec<LegacyEntity> = x["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let mut e = e.clone();
            e["group_id"] = json!(group);
            e["created_at"] = json!(episode.created_at);
            e["name_embedding"] = json!(vector());
            serde_json::from_value(e).unwrap()
        })
        .collect();
    let mut f = x["fact"].clone();
    f["group_id"] = json!(group);
    f["created_at"] = json!(episode.created_at);
    f["source_node_uuid"] = json!(entities[0].uuid);
    f["target_node_uuid"] = json!(entities[1].uuid);
    f["fact_embedding"] = json!(vector());
    let facts = vec![serde_json::from_value(f).unwrap()];
    Prepared {
        baseline: None,
        invalidated_fact_ids: vec![],
        episode,
        entities,
        facts,
    }
}
fn vector() -> Vec<f32> {
    let mut v = vec![0.; 3072];
    v[0] = 1.;
    v
}

#[tokio::test]
#[ignore = "requires dedicated disposable FalkorDB on loopback port 16379"]
async fn restore_audit_reads_all_pages_and_refuses_malformed_identity_without_writes() {
    use graphiti_rs::compat::audit;
    let url = std::env::var("COMPAT_TEST_DB_URL").expect("explicit disposable DB required");
    assert_eq!(url, "redis://127.0.0.1:16379");
    // Scope-exclusion fixtures intentionally survive clear(); isolate reruns.
    let group_storage = format!(
        "project_read_audit_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let group = group_storage.as_str();
    let writer = LegacyStore::connect(&url, WriteMode::Synthetic)
        .await
        .unwrap();
    writer.clear(group).await.unwrap();
    let mut last_node = String::new();
    let mut last_target = String::new();
    let mut last_fact = String::new();
    let mut last_episode = String::new();
    for _ in 0..129 {
        let mut p = fixture(group);
        p.episode.uuid = uuid::Uuid::new_v4().to_string();
        for node in &mut p.entities {
            node.uuid = uuid::Uuid::new_v4().to_string();
        }
        p.facts[0].uuid = uuid::Uuid::new_v4().to_string();
        p.facts[0].source_node_uuid = p.entities[0].uuid.clone();
        p.facts[0].target_node_uuid = p.entities[1].uuid.clone();
        p.facts[0].episodes = vec![p.episode.uuid.clone()];
        p.episode.entity_edges = vec![p.facts[0].uuid.clone()];
        last_node = p.entities[0].uuid.clone();
        last_target = p.entities[1].uuid.clone();
        last_fact = p.facts[0].uuid.clone();
        last_episode = p.episode.uuid.clone();
        assert!(writer.commit(&p).await.unwrap());
    }
    assert!(audit::inspect(&writer, group).await.is_err());
    let reader = LegacyStore::connect(&url, WriteMode::ReadOnly)
        .await
        .unwrap();
    let first = audit::inspect(&reader, group).await.unwrap();
    assert_eq!(
        (first.entities, first.episodes, first.facts),
        (258, 129, 129)
    );
    assert_eq!((first.entity_vectors, first.fact_vectors), (258, 129));
    let second = audit::inspect(&reader, group).await.unwrap();
    assert_eq!(first.identity_sha256, second.identity_sha256);
    assert_eq!(first.vector_sha256, second.vector_sha256);
    let mut connection =
        redis::aio::ConnectionManager::new(redis::Client::open(url.as_str()).unwrap())
            .await
            .unwrap();
    // Reproduce inherited foreign copies in a graph without an index. The
    // maintenance audit counts them; candidate queries exclude them before
    // LIMIT, and a direct lookup retains the same strict group guard.
    for query in [
        format!(
            "MATCH(n:Entity) WHERE n.uuid IN ['{last_node}','{last_target}'] SET n.group_id='general_chat'"
        ),
        format!("MATCH()-[e:RELATES_TO {{uuid:'{last_fact}'}}]->() SET e.group_id='general_chat'"),
        format!("MATCH(e:Episodic {{uuid:'{last_episode}'}}) SET e.group_id='general_chat'"),
    ] {
        let _: redis::Value = redis::cmd("GRAPH.QUERY")
            .arg(group)
            .arg(query)
            .query_async(&mut connection)
            .await
            .unwrap();
    }
    let classified = audit::inspect(&reader, group).await.unwrap();
    assert_eq!(
        (classified.entities, classified.episodes, classified.facts),
        (258, 129, 129)
    );
    assert_eq!(
        (
            classified.mis_scoped_entities,
            classified.mis_scoped_episodes,
            classified.mis_scoped_facts
        ),
        (2, 1, 1)
    );
    assert_ne!(first.identity_sha256, classified.identity_sha256);
    assert_eq!(first.vector_sha256, classified.vector_sha256);
    assert_eq!(
        reader.nodes(group, "", None, 1000).await.unwrap().len(),
        256
    );
    assert_eq!(
        reader.facts(group, "", None, 1000).await.unwrap().len(),
        128
    );
    assert_eq!(
        reader
            .keyword_facts_filtered(group, "", None, 1000, None)
            .await
            .unwrap()
            .len(),
        128
    );
    assert!(
        reader
            .related_facts(group, &last_node, &last_target)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(reader.node(group, &last_node).await.is_err());
    assert!(reader.edge(group, &last_fact).await.is_err());
    let _: redis::Value = redis::cmd("GRAPH.QUERY")
        .arg(group)
        .arg(format!(
            "MATCH(n:Entity {{uuid:'{last_node}'}}) REMOVE n.uuid"
        ))
        .query_async(&mut connection)
        .await
        .unwrap();
    let error = audit::inspect(&reader, group).await.err().unwrap();
    assert!(error.to_string().contains("omitted records"));
    writer.clear(group).await.unwrap();
}
fn config(path: PathBuf, base: &str) -> CompatConfig {
    CompatConfig {
        pipeline: graphiti_rs::compat::config::IngestionPipeline::CombinedPreview,
        entity_types: vec![],
        redis_url: "redis://127.0.0.1:16379".into(),
        mode: WriteMode::Synthetic,
        journal_dir: Some(path),
        llm_base: base.into(),
        llm_model: "gpt-6-luna".into(),
        llm_key_file: None,
        gemini_base: base.into(),
        gemini_key_file: None,
        max_tokens: 512,
        timeout: Duration::from_secs(3),
        capacity: 8,
        candidates: 20,
    }
}

#[test]
fn golden_metadata_and_result_layers() {
    let golden: Value =
        serde_json::from_str(include_str!("fixtures/production-mcp-contract.json")).unwrap();
    assert_eq!(
        serde_json::to_value(tools()).unwrap(),
        golden["tools"]["tools"]
    );
    assert_eq!(tools().len(), 10);
    let status = response(
        "get_status",
        Ok(golden["get_status"]["structuredContent"].clone()),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(status).unwrap()["content"],
        golden["get_status"]["content"]
    );
    let r = response("get_episode", Ok(json!({"found":false,"episode":null}))).unwrap();
    assert_eq!(
        r.structured_content,
        Some(json!({"result":{"found":false,"episode":null}}))
    );
    assert_eq!(r.is_error, Some(false));
    let r = response("add_memory", Err(anyhow::anyhow!("denied"))).unwrap();
    assert_eq!(
        r.structured_content,
        Some(json!({"result":{"error":"denied"}}))
    );
    assert_eq!(r.is_error, Some(false));
    assert!(shape("get_episode", &json!({"uuid":"x"})).is_err());
    assert!(shape("missing", &json!({})).is_err());
    assert!(shape("search_nodes", &json!({"query":"x","max_nodes":"ten"})).is_err());
}
#[test]
fn policy_explicit_defaults_and_destructive_targets() {
    for group in [
        "user_primary",
        "global_user",
        "general_chat",
        "project_demo_1234abcd",
    ] {
        policy::read(group).unwrap();
        policy::write(group).unwrap();
    }
    for group in ["hermes", "family_shared"] {
        policy::read(group).unwrap();
        assert!(policy::write(group).is_err());
    }
    for group in [
        "",
        "unknown",
        "project_demo_1234abcg",
        "project__1234abcd",
        "project_DEMO_1234abcd",
        "project_demo_1234abcD",
    ] {
        assert!(policy::read(group).is_err());
        assert!(policy::write(group).is_err());
    }
    assert_eq!(policy::groups(None).unwrap(), vec!["user_primary"]);
    assert_eq!(
        policy::groups(Some(&[
            "general_chat".into(),
            "user_primary".into(),
            "general_chat".into()
        ]))
        .unwrap(),
        vec!["general_chat", "user_primary"]
    );
    for groups in [
        None,
        Some(vec![]),
        Some(vec!["user_primary".into(), "user_primary".into()]),
        Some(vec!["user_primary".into(), "general_chat".into()]),
    ] {
        assert!(policy::clear(groups.as_deref()).is_err());
    }
    assert!(policy::limit(0).is_err());
    assert!(policy::limit(101).is_err());
}
#[test]
fn private_dtos_preserve_vectors_but_public_results_do_not() {
    let p = fixture("user_primary");
    p.validate().unwrap();
    let copy: Prepared = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
    assert_eq!(copy.entities[0].name_embedding, Some(vector()));
    assert_eq!(copy.facts[0].fact_embedding, Some(vector()));
    for value in [
        episode_public(&p.episode),
        node_public(&p.entities[0]),
        fact_public(&p.facts[0]),
    ] {
        assert!(!value.to_string().contains("embedding"));
    }
    assert_ne!(p.facts[0].valid_at, p.facts[0].invalid_at);
    assert_ne!(p.facts[0].invalid_at, p.facts[0].expired_at);
    let mut bad = p.clone();
    bad.facts[0].episodes = vec![uuid::Uuid::new_v4().to_string()];
    assert!(bad.validate().is_err());
    let mut bad = p;
    bad.entities[0]
        .attributes
        .insert("uuid".into(), json!("evil"));
    assert!(bad.validate().is_err());
}
#[test]
fn public_dtos_match_observed_python_datetime_and_attribute_serialization() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/observed-sdk-public-dtos.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut node: LegacyEntity = serde_json::from_value(case["expected_node"].clone()).unwrap();
        node.created_at = case["stored_timestamp"].as_str().unwrap().into();
        node.name_embedding = Some(vector());
        node.attributes
            .insert("name_embedding".into(), json!([1.0]));
        node.attributes
            .insert("embedding_note".into(), json!("hidden"));
        node.attributes
            .insert("EMBEDDING_metadata".into(), json!("hidden"));
        assert_eq!(node_public(&node), case["expected_node"]);
        let mut fact: LegacyFact = serde_json::from_value(case["expected_fact"].clone()).unwrap();
        fact.created_at = node.created_at.clone();
        fact.valid_at = Some(node.created_at.clone());
        fact.invalid_at = Some(node.created_at.clone());
        fact.expired_at = Some(node.created_at.clone());
        fact.fact_embedding = Some(vector());
        fact.attributes
            .insert("fact_embedding".into(), json!([1.0]));
        assert_eq!(fact_public(&fact), case["expected_fact"]);
        assert_eq!(fact.created_at, case["stored_timestamp"]);
    }
}
#[test]
fn deterministic_group_fairness() {
    assert_eq!(
        round_robin(
            vec![
                vec![json!("A1"), json!("A2")],
                vec![json!("B1"), json!("B2")]
            ],
            3
        ),
        vec![json!("A1"), json!("B1"), json!("A2")]
    );
}

#[tokio::test]
#[ignore = "requires dedicated disposable FalkorDB on loopback port 16379"]
async fn replay_preserves_deployed_node_windows_and_refuses_unsafe_inputs() {
    let url = std::env::var("COMPAT_TEST_DB_URL").expect("explicit disposable DB required");
    assert_eq!(url, "redis://127.0.0.1:16379");
    let groups: Vec<_> = (0..2)
        .map(|_| {
            format!(
                "project_replay_{}",
                &uuid::Uuid::new_v4().simple().to_string()[..8]
            )
        })
        .collect();
    let mut connection = redis::Client::open(url.clone())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let rows: Vec<_> = (1..=30)
        .map(|i| format!("00000000-0000-4000-8000-{i:012x}"))
        .collect();
    for group in &groups {
        let query = format!(
            "CYPHER rows={} vector={} UNWIND $rows AS row CREATE(n:Entity:Person {{uuid:row,name:'Unrelated',summary:'Synthetic',group_id:'{group}',created_at:'2026-10-02T00:00:00Z',name_embedding:vecf32($vector),embedding_note:'hidden',visible:'keep'}})",
            serde_json::to_string(&rows).unwrap(),
            serde_json::to_string(&vector()).unwrap()
        );
        let _: redis::Value = redis::cmd("GRAPH.QUERY")
            .arg(group)
            .arg(query)
            .query_async(&mut connection)
            .await
            .unwrap();
    }
    let mut cfg = config(dir(), "http://127.0.0.1:9");
    cfg.mode = WriteMode::ReadOnly;
    cfg.journal_dir = None;
    cfg.redis_url = url.clone();
    let engine = Engine::new(cfg.clone()).await.unwrap();
    let args = json!({"query":"nomatchingterm", "group_ids":[groups[0]], "max_nodes":25});
    assert!(
        engine.execute("search_nodes", args.clone()).await.unwrap()["nodes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let hits = engine
        .replay_search("search_nodes", args.clone(), &vector())
        .await
        .unwrap();
    assert_eq!(hits["nodes"].as_array().unwrap().len(), 10);
    for node in hits["nodes"].as_array().unwrap() {
        assert_eq!(node["created_at"], "2026-10-02T00:00:00+00:00");
        assert_eq!(node["attributes"], json!({"visible":"keep"}));
    }
    let hits = engine
        .replay_search(
            "search_nodes",
            json!({"query":"nomatchingterm","group_ids":groups,"max_nodes":25}),
            &vector(),
        )
        .await
        .unwrap();
    let nodes = hits["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 20);
    for (i, node) in nodes.iter().enumerate() {
        assert_eq!(node["group_id"], groups[i % 2]);
    }
    assert!(
        engine
            .replay_search("clear_graph", json!({"group_ids":[groups[0]]}), &vector())
            .await
            .is_err()
    );
    for invalid in [vec![1.0], vec![0.0; 3072], vec![f32::NAN; 3072]] {
        assert!(
            engine
                .replay_search("search_nodes", args.clone(), &invalid)
                .await
                .is_err()
        );
    }
    assert!(
        engine
            .replay_search(
                "search_memory_facts",
                json!({"query":"test","group_ids":[groups[0]],"max_facts":100}),
                &vector()
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("candidate bound")
    );
    let path = dir();
    let synthetic = Engine::new(config(path.clone(), "http://127.0.0.1:9"))
        .await
        .unwrap();
    assert!(
        synthetic
            .replay_search("search_nodes", args, &vector())
            .await
            .unwrap_err()
            .to_string()
            .contains("read-only")
    );
    drop(synthetic);
    let writer = LegacyStore::connect(&url, WriteMode::Synthetic)
        .await
        .unwrap();
    for group in &groups {
        writer.clear(group).await.unwrap();
    }
    let _ = std::fs::remove_dir_all(path);
}
#[test]
fn journal_idempotence_lock_recovery_and_private_permissions() {
    let path = dir();
    let j = Journal::open(&path, 1).unwrap();
    assert!(Journal::open(&path, 1).is_err());
    let id = uuid::Uuid::new_v4().to_string();
    let r = request(&id);
    assert_eq!(j.accept(r.clone()).unwrap().state, "queued");
    assert_eq!(j.accept(r.clone()).unwrap().uuid, id);
    assert!(
        j.accept(request(&uuid::Uuid::new_v4().to_string()))
            .is_err()
    );
    let mut changed = r.clone();
    changed.episode_body = "changed".into();
    assert!(j.accept(changed).is_err());
    let job = j.next().unwrap().unwrap();
    let original = job.created.clone();
    drop(j);
    let j = Journal::open(&path, 1).unwrap();
    assert!(j.next().unwrap().is_none());
    assert!(j.accept(r).is_err());
    drop(j);
    let path2 = dir();
    let j = Journal::open(&path2, 1).unwrap();
    let mut p = fixture("user_primary");
    let r = AddMemory {
        uuid: Some(p.episode.uuid.clone()),
        group_id: Some("user_primary".into()),
        name: p.episode.name.clone(),
        episode_body: p.episode.content.clone(),
        source: p.episode.source.clone(),
        source_description: p.episode.source_description.clone(),
    };
    j.accept(r).unwrap();
    let job = j.next().unwrap().unwrap();
    p.episode.created_at = job.created.clone();
    p.episode.valid_at = job.created.clone();
    j.prepared(&job, &p).unwrap();
    let time = job.created;
    drop(j);
    let j = Journal::open(&path2, 1).unwrap();
    let recovered = j.next().unwrap().unwrap();
    assert_eq!(recovered.created, time);
    assert_eq!(
        recovered.prepared.unwrap().facts[0].fact_embedding,
        Some(vector())
    );
    assert!(!original.is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path2.join("jobs.sqlite"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
    drop(j);
    std::fs::remove_dir_all(path).unwrap();
    std::fs::remove_dir_all(path2).unwrap();
}
#[test]
fn provider_configuration_and_vector_drift_fail_closed() {
    assert!(config(dir(), "https://example.com").validate().is_err());
    let mut c = config(dir(), "http://127.0.0.1:12345");
    c.redis_url = "redis://127.0.0.1:6379".into();
    assert!(c.validate().is_err());
    c.redis_url = "redis://127.0.0.1:16379".into();
    c.llm_model = "other".into();
    assert!(c.validate().is_err());
    for v in [vec![1.; 1536], vec![0.; 3072], vec![f32::NAN; 3072]] {
        assert!(validate_vector(&v).is_err());
    }
    validate_vector(&vector()).unwrap();
}

#[test]
fn operational_mode_requires_deployed_profile_and_private_credentials() {
    use graphiti_rs::compat::config::IngestionPipeline;
    let root = dir();
    std::fs::create_dir_all(&root).unwrap();
    let secret = root.join("synthetic-key");
    std::fs::write(&secret, "synthetic-validation-only").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let mut c = config(
        root.join("journal"),
        "https://factchat-cloud.mindlogic.ai/v1/gateway",
    );
    c.mode = WriteMode::Operational;
    c.pipeline = IngestionPipeline::Graphiti;
    c.redis_url = "redis://falkordb:6379".into();
    c.gemini_base = "https://generativelanguage.googleapis.com/v1beta".into();
    assert!(c.validate().is_err());
    c.llm_key_file = Some(secret.clone());
    c.gemini_key_file = Some(secret.clone());
    c.validate().unwrap();
    let mut changed = c.clone();
    changed.pipeline = IngestionPipeline::CombinedPreview;
    assert!(changed.validate().is_err());
    let mut changed = c.clone();
    changed.gemini_base = "http://localhost:12345".into();
    assert!(changed.validate().is_err());
    let mut changed = c.clone();
    changed.llm_base = "https://example.com/v1/gateway".into();
    assert!(changed.validate().is_err());
    let mut changed = c.clone();
    changed.redis_url = "https://falkordb:6379".into();
    assert!(changed.validate().is_err());
    let mut changed = c.clone();
    changed.max_tokens = 0;
    assert!(changed.validate().is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(c.validate().is_err());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[derive(Clone)]
struct Mock {
    calls: Arc<AtomicUsize>,
    embeds: Arc<AtomicUsize>,
    bad: Arc<AtomicBool>,
    release: Arc<tokio::sync::Semaphore>,
    bodies: Arc<tokio::sync::Mutex<Vec<Value>>>,
}
async fn llm(
    axum::extract::State(s): axum::extract::State<Mock>,
    headers: axum::http::HeaderMap,
    axum::Json(v): axum::Json<Value>,
) -> axum::Json<Value> {
    assert_eq!(headers["authorization"], "Bearer synthetic-llm-key");
    assert!(!headers.contains_key("x-goog-api-key"));
    s.calls.fetch_add(1, Ordering::SeqCst);
    s.bodies.lock().await.push(v);
    let permit = s.release.acquire().await.unwrap();
    permit.forget();
    axum::Json(
        json!({"choices":[{"message":{"content":json!({"entities":[{"name":"Gaon","summary":"Person","labels":["Person"]},{"name":"Saebit","summary":"Company","labels":["Organization"]}],"facts":[{"source":0,"target":1,"name":"WORKS_AT","fact":"Gaon works at Saebit"}]}).to_string()}}]}),
    )
}
async fn embeddings(
    axum::extract::State(s): axum::extract::State<Mock>,
    headers: axum::http::HeaderMap,
    axum::Json(v): axum::Json<Value>,
) -> axum::Json<Value> {
    assert_eq!(headers["x-goog-api-key"], "synthetic-gemini-key");
    assert!(!headers.contains_key("authorization"));
    s.embeds.fetch_add(1, Ordering::SeqCst);
    s.bodies.lock().await.push(v.clone());
    let n = v["requests"].as_array().unwrap().len();
    let values = if s.bad.load(Ordering::SeqCst) {
        vec![1.; 10]
    } else {
        vector()
    };
    axum::Json(json!({"embeddings":(0..n).map(|_|json!({"values":values})).collect::<Vec<_>>()}))
}
async fn poll(engine: &Engine, group: &str, id: &str) -> Value {
    for _ in 0..100 {
        let r = engine
            .execute("get_episode", json!({"uuid":id,"group_id":group}))
            .await
            .unwrap();
        if r["found"] == true {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("synthetic episode did not commit");
}

#[derive(Clone, Default)]
struct StagedMock {
    trace: Arc<tokio::sync::Mutex<Vec<Value>>>,
}
fn tag<'a>(input: &'a str, name: &str) -> Option<&'a str> {
    input
        .split_once(&format!("<{name}>"))?
        .1
        .split_once(&format!("</{name}>"))
        .map(|p| p.0.trim())
}
async fn staged_llm(
    axum::extract::State(s): axum::extract::State<StagedMock>,
    axum::Json(v): axum::Json<Value>,
) -> axum::Json<Value> {
    assert_eq!(v["response_format"], json!({"type":"json_object"}));
    assert_eq!(v["model"], "gpt-6-luna");
    assert_eq!(v["temperature"], 1);
    assert_eq!(v["chat_template_kwargs"]["enable_thinking"], false);
    assert!(v.get("extra_body").is_none());
    let user = v["messages"][1]["content"].as_str().unwrap();
    let schema: Value = serde_json::from_str(user.rsplit_once("input:\n").unwrap().1).unwrap();
    let title = schema["title"].as_str().unwrap();
    let content = match title {
        "ExtractedEntities" => {
            let current = tag(user, "CURRENT MESSAGE")
                .or_else(|| tag(user, "TEXT"))
                .or_else(|| tag(user, "JSON"))
                .unwrap();
            let names = if current.contains("다온") {
                vec![("다온", 1), ("누리", 2)]
            } else if current.contains("Ravi") {
                vec![("Ravi", 1), ("Delta", 2)]
            } else if current.contains("Mira") {
                vec![("Mira", 1)]
            } else if current.contains("Nari") {
                vec![("Nari", 1)]
            } else if current.contains("Hanul") {
                vec![("Gaon", 1), ("Hanul", 2)]
            } else if current.starts_with("G.") {
                vec![("G.", 1), ("Saebit", 2)]
            } else {
                vec![("Gaon", 1), ("Saebit", 2)]
            };
            json!({"extracted_entities":names.into_iter().map(|(name,kind)| json!({"name":name,"entity_type_id":kind})).collect::<Vec<_>>()})
        }
        "NodeResolutions" => {
            let nodes: Value = serde_json::from_str(tag(user, "ENTITIES").unwrap()).unwrap();
            let existing: Value =
                serde_json::from_str(tag(user, "EXISTING ENTITIES").unwrap()).unwrap();
            json!({"entity_resolutions":nodes.as_array().unwrap().iter().map(|n| {
                let name=n["name"].as_str().unwrap();
                json!({"id":n["id"],"name":name,"duplicate_name":existing.as_array().unwrap().iter().find(|n|n["name"] == if name == "G." {"Gaon"} else {name}).and_then(|n|n["name"].as_str()).unwrap_or("")})
            }).collect::<Vec<_>>()})
        }
        "ExtractedEdges" => {
            let current = tag(user, "CURRENT_MESSAGE").unwrap();
            if current.contains("Nari") || current.contains("Mira") {
                json!({"edges":[]})
            } else {
                let (source, target, fact, time) = if current.contains("다온") {
                    (
                        "다온",
                        "누리",
                        "다온은 누리에서 일한다.",
                        "2026-10-02T00:00:00Z",
                    )
                } else if current.contains("Ravi") {
                    (
                        "Ravi",
                        "Delta",
                        "Ravi works at Delta.",
                        "2026-10-02T00:00:00Z",
                    )
                } else if current.contains("Hanul") {
                    (
                        "Gaon",
                        "Hanul",
                        "Gaon works at Hanul.",
                        "2026-10-02T00:00:00Z",
                    )
                } else if current.starts_with("G.") {
                    (
                        "G.",
                        "Saebit",
                        "Gaon is employed by Saebit.",
                        "2026-10-01T00:00:00Z",
                    )
                } else {
                    (
                        "Gaon",
                        "Saebit",
                        "Gaon works at Saebit.",
                        "2026-10-01T00:00:00Z",
                    )
                };
                json!({"edges":[{"source_entity_name":source,"target_entity_name":target,"relation_type":"WORKS_AT","fact":fact,"valid_at":time,"invalid_at":null}]})
            }
        }
        "EdgeDuplicate" => {
            if tag(user, "NEW FACT").unwrap().contains("Hanul") {
                json!({"duplicate_facts":[],"contradicted_facts":[0]})
            } else if tag(user, "NEW FACT").unwrap().contains("employed by") {
                json!({"duplicate_facts":[0],"contradicted_facts":[]})
            } else {
                json!({"duplicate_facts":[],"contradicted_facts":[]})
            }
        }
        "SummarizedEntities" => {
            if user.contains("Mira likes quiet evenings.") {
                json!({"summaries":[{"name":"Mira","summary":"Mira likes quiet evenings."}]})
            } else {
                json!({"summaries":[{"name":"Nari","summary":"Nari enjoys quiet mornings."}]})
            }
        }
        _ => panic!("unexpected schema {title}"),
    };
    let mut recorded = v;
    recorded["mock_response"] = content.clone();
    s.trace.lock().await.push(recorded);
    axum::Json(json!({"choices":[{"message":{"content":content.to_string()}}]}))
}
async fn staged_embeddings(axum::Json(v): axum::Json<Value>) -> axum::Json<Value> {
    axum::Json(
        json!({"embeddings":v["requests"].as_array().unwrap().iter().map(|_|json!({"values":vector()})).collect::<Vec<_>>()}),
    )
}

#[tokio::test]
#[ignore = "requires dedicated disposable FalkorDB on loopback port 16379"]
async fn staged_ingestion_alias_duplicates_temporal_invalidation_summary_and_restart() {
    use graphiti_rs::compat::config::{EntityType, IngestionPipeline};
    let url = std::env::var("COMPAT_TEST_DB_URL").expect("explicit disposable DB required");
    assert_eq!(url, "redis://127.0.0.1:16379");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state = StagedMock::default();
    let app = axum::Router::new()
        .route("/chat/completions", axum::routing::post(staged_llm))
        .route(
            "/models/gemini-embedding-2:batchEmbedContents",
            axum::routing::post(staged_embeddings),
        )
        .with_state(state.clone());
    let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let path = dir();
    let mut c = config(path.clone(), &base);
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
    let engine = Engine::new(c.clone()).await.unwrap();
    engine
        .store
        .clear("project_staged_pipeline_20261002")
        .await
        .unwrap();
    let episodes: Vec<_> = (0..7).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let texts = [
        "Gaon works at Saebit.",
        "G. works at Saebit.",
        "Gaon now works at Hanul.",
        "Nari enjoys quiet mornings.",
        "다온은 누리에서 일한다.",
        r#"{"person":"Ravi","organization":"Delta","job":"works"}"#,
        "Mira likes quiet evenings.",
    ];
    let sources = [
        "message", "message", "message", "message", "message", "json", "text",
    ];
    let mut first_fact = None;
    let mut gaon = None;
    let mut snapshots = vec![];
    for (i, (id, text)) in episodes.iter().zip(texts).enumerate() {
        let args = json!({"uuid":id,"name":format!("staged-{i}"),"episode_body":text,"group_id":"project_staged_pipeline_20261002","source":sources[i]});
        engine.execute("add_memory", args.clone()).await.unwrap();
        poll(&engine, "project_staged_pipeline_20261002", id).await;
        let ep = engine
            .store
            .episode("project_staged_pipeline_20261002", id)
            .await
            .unwrap()
            .unwrap();
        if i == 0 {
            assert_eq!(ep.entity_edges.len(), 1);
            let f = engine
                .store
                .edge("project_staged_pipeline_20261002", &ep.entity_edges[0])
                .await
                .unwrap()
                .unwrap();
            gaon = Some(f.source_node_uuid.clone());
            first_fact = Some(f);
        } else if i == 1 {
            let old = first_fact.as_ref().unwrap();
            assert_eq!(ep.entity_edges, vec![old.uuid.clone()]);
            let f = engine
                .store
                .edge("project_staged_pipeline_20261002", &old.uuid)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(f.fact, old.fact);
            assert_eq!(f.created_at, old.created_at);
            assert_eq!(f.episodes, episodes[..2]);
            let alias = engine
                .store
                .keyword_nodes("project_staged_pipeline_20261002", "G.", None, 10)
                .await
                .unwrap();
            assert!(alias.iter().all(|n| n.name != "G."));
        } else if i == 2 {
            let old = first_fact.as_ref().unwrap();
            assert_eq!(ep.entity_edges.len(), 2);
            assert!(ep.entity_edges.contains(&old.uuid));
            let f = engine
                .store
                .edge("project_staged_pipeline_20261002", &old.uuid)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(f.invalid_at.as_deref(), Some("2026-10-02T00:00:00Z"));
            assert!(f.expired_at.is_some());
            assert_eq!(f.episodes, episodes[..2]);
            assert_eq!(f.created_at, old.created_at);
            let n = engine
                .store
                .node("project_staged_pipeline_20261002", gaon.as_ref().unwrap())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(n.summary, "Gaon works at Saebit.\nGaon works at Hanul.");
            let added = ep.entity_edges.iter().find(|id| *id != &old.uuid).unwrap();
            let new = engine
                .store
                .edge("project_staged_pipeline_20261002", added)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(new.source_node_uuid, old.source_node_uuid);
            assert_ne!(new.target_node_uuid, old.target_node_uuid);
            assert_eq!(new.episodes, vec![id.clone()]);
        } else if i == 3 {
            assert!(ep.entity_edges.is_empty());
            let n = engine
                .store
                .keyword_nodes("project_staged_pipeline_20261002", "Nari", None, 10)
                .await
                .unwrap()
                .into_iter()
                .find(|n| n.name == "Nari")
                .unwrap();
            assert_eq!(n.summary, "Nari enjoys quiet mornings.");
            assert!(n.labels.contains(&"Person".to_owned()));
        }
        if i == 4 || i == 5 {
            assert_eq!(ep.entity_edges.len(), 1);
            let fact = engine
                .store
                .edge("project_staged_pipeline_20261002", &ep.entity_edges[0])
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                fact.fact,
                if i == 4 {
                    "다온은 누리에서 일한다."
                } else {
                    "Ravi works at Delta."
                }
            );
            assert_eq!(fact.episodes, vec![id.clone()]);
        } else if i == 6 {
            assert!(ep.entity_edges.is_empty());
            let node = engine
                .store
                .nodes("project_staged_pipeline_20261002", "Mira", None, 10)
                .await
                .unwrap()
                .into_iter()
                .find(|n| n.name == "Mira")
                .unwrap();
            assert_eq!(node.summary, "Mira likes quiet evenings.");
        }
        snapshots.push(json!({"episode":ep,
            "nodes":engine.store.nodes("project_staged_pipeline_20261002","",None,100).await.unwrap(),
            "facts":engine.store.facts("project_staged_pipeline_20261002","",None,100).await.unwrap()}));
        let count = state.trace.lock().await.len();
        engine.execute("add_memory", args).await.unwrap();
        assert_eq!(state.trace.lock().await.len(), count);
    }
    // Every later node prompt receives earlier committed messages in chronological order.
    let trace = state.trace.lock().await;
    let node_prompts: Vec<_> = trace
        .iter()
        .filter(|r| {
            r["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("entity nodes from conversational messages")
        })
        .collect();
    assert_eq!(node_prompts.len(), 5);
    assert!(
        tag(
            node_prompts[3]["messages"][1]["content"].as_str().unwrap(),
            "PREVIOUS MESSAGES"
        )
        .unwrap()
        .contains(texts[0])
    );
    // JSON/text episodes receive no message-source history in their dedupe stage.
    for (id, source) in [(5, "json"), (6, "text")] {
        let dedupe = trace
            .iter()
            .find(|r| {
                r["messages"][0]["content"]
                    .as_str()
                    .unwrap()
                    .contains("duplicates of existing entities")
                    && tag(
                        r["messages"][1]["content"].as_str().unwrap(),
                        "CURRENT MESSAGE",
                    ) == Some(texts[id])
            })
            .unwrap();
        assert_eq!(
            tag(
                dedupe["messages"][1]["content"].as_str().unwrap(),
                "PREVIOUS MESSAGES"
            ),
            Some("[]"),
            "source {source}"
        );
    }
    let count = trace.len();
    if let Some(path) = std::env::var_os("COMPAT_STAGED_REPORT") {
        std::fs::write(path,serde_json::to_vec_pretty(&json!({"scope":"exclusively synthetic staged pipeline with mock providers","actual_provider_calls":0,"snapshots":snapshots,"steps":*trace})).unwrap()).unwrap();
    }
    drop(trace);
    drop(engine);
    let mut reopened = None;
    for _ in 0..100 {
        match Engine::new(c.clone()).await {
            Ok(e) => {
                reopened = Some(e);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    let engine = reopened.expect("worker released private journal");
    engine.execute("add_memory",json!({"uuid":episodes[2],"name":"staged-2","episode_body":texts[2],"group_id":"project_staged_pipeline_20261002","source":"message"})).await.unwrap();
    assert_eq!(state.trace.lock().await.len(), count);
    let original = engine
        .store
        .node("project_staged_pipeline_20261002", gaon.as_ref().unwrap())
        .await
        .unwrap()
        .unwrap();
    let mut stale = Prepared {
        baseline: None,
        invalidated_fact_ids: vec![],
        entities: vec![original.clone()],
        facts: vec![],
        episode: LegacyEpisode {
            uuid: uuid::Uuid::new_v4().to_string(),
            name: "concurrent context".into(),
            content: "synthetic".into(),
            source: "message".into(),
            source_description: "".into(),
            group_id: "project_staged_pipeline_20261002".into(),
            created_at: original.created_at.clone(),
            valid_at: original.created_at.clone(),
            entity_edges: vec![],
        },
    };
    stale.entities[0].summary = "stale model update".into();
    let mut connection =
        redis::aio::ConnectionManager::new(redis::Client::open(url.as_str()).unwrap())
            .await
            .unwrap();
    let _: redis::Value = redis::cmd("GRAPH.QUERY")
        .arg("project_staged_pipeline_20261002")
        .arg(format!(
            "MATCH(n:Entity {{uuid:'{}'}}) SET n.summary='concurrent owner update'",
            original.uuid
        ))
        .query_async(&mut connection)
        .await
        .unwrap();
    let error = engine
        .store
        .plan_observed(&stale, &[original], &[])
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("entity changed during model resolution")
    );
    assert_eq!(
        engine
            .store
            .node("project_staged_pipeline_20261002", gaon.as_ref().unwrap())
            .await
            .unwrap()
            .unwrap()
            .summary,
        "concurrent owner update"
    );
    engine
        .store
        .clear("project_staged_pipeline_20261002")
        .await
        .unwrap();
    drop(engine);
    mock.abort();
    tokio::time::sleep(Duration::from_millis(100)).await;
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
#[ignore = "requires dedicated disposable FalkorDB on loopback port 16379"]
async fn isolated_legacy_store_queue_mcp_and_provider_contract() {
    // Never accept another URL or fall back to the usual production Redis port.
    let url = std::env::var("COMPAT_TEST_DB_URL")
        .expect("explicit disposable COMPAT_TEST_DB_URL required");
    assert_eq!(url, "redis://127.0.0.1:16379");
    let state = Mock {
        calls: Arc::new(AtomicUsize::new(0)),
        embeds: Arc::new(AtomicUsize::new(0)),
        bad: Arc::new(AtomicBool::new(false)),
        release: Arc::new(tokio::sync::Semaphore::new(0)),
        bodies: Arc::new(tokio::sync::Mutex::new(vec![])),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route("/chat/completions", axum::routing::post(llm))
        .route(
            "/models/gemini-embedding-2:batchEmbedContents",
            axum::routing::post(embeddings),
        )
        .with_state(state.clone());
    let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let store = LegacyStore::connect(&url, WriteMode::Synthetic)
        .await
        .unwrap();
    let mut p = fixture("user_primary");
    p.entities[0]
        .attributes
        .insert("별칭 이름".into(), json!(["가온", "Gaon"]));
    store.clear("user_primary").await.unwrap();
    assert!(store.commit(&p).await.unwrap());
    assert!(!store.commit(&p).await.unwrap());
    assert_eq!(
        store
            .episode("user_primary", &p.episode.uuid)
            .await
            .unwrap()
            .unwrap(),
        p.episode
    );
    assert_eq!(
        store
            .edge("user_primary", &p.facts[0].uuid)
            .await
            .unwrap()
            .unwrap(),
        p.facts[0]
    );
    let n = store
        .node("user_primary", &p.entities[0].uuid)
        .await
        .unwrap()
        .unwrap();
    assert!(n.labels.contains(&"Person".into()));
    assert_eq!(n.attributes, p.entities[0].attributes);
    let mut connection =
        redis::aio::ConnectionManager::new(redis::Client::open(url.as_str()).unwrap())
            .await
            .unwrap();
    let _:redis::Value=redis::cmd("GRAPH.QUERY").arg("user_primary").arg(format!("MATCH(e:Episodic {{uuid:'{}'}}),(n:Entity {{uuid:'{}'}}) CREATE(s:Saga {{uuid:'00000000-0000-4000-8000-000000000301',group_id:'user_primary',custom:'retain'}})-[:HAS_EPISODE]->(e) CREATE(c:Community {{uuid:'00000000-0000-4000-8000-000000000302',group_id:'user_primary'}})-[:HAS_MEMBER]->(n)",p.episode.uuid,p.entities[0].uuid)).query_async(&mut connection).await.unwrap();
    // Existing entity IDs can now update without changing identity or unrelated topology.
    let mut collision = p.clone();
    collision.episode.uuid = uuid::Uuid::new_v4().to_string();
    for fact in &mut collision.facts {
        fact.uuid = uuid::Uuid::new_v4().to_string();
        fact.episodes = vec![collision.episode.uuid.clone()];
    }
    collision.episode.entity_edges = collision.facts.iter().map(|f| f.uuid.clone()).collect();
    let collision = store.plan(&collision).await.unwrap();
    assert!(store.commit(&collision).await.unwrap());
    assert!(!store.commit(&collision).await.unwrap());
    assert!(
        store
            .episode("user_primary", &collision.episode.uuid)
            .await
            .unwrap()
            .is_some()
    );
    // Identical UUIDs and names in another group remain separate identities.
    store.clear("general_chat").await.unwrap();
    let mut other = fixture("general_chat");
    other.episode.name = "Other synthetic group".into();
    store.commit(&other).await.unwrap();
    let path = dir();
    let mut cfg = config(path.clone(), &base);
    cfg.redis_url = url.clone();
    std::fs::create_dir(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    for (name, key) in [
        ("llm.key", "synthetic-llm-key"),
        ("gemini.key", "synthetic-gemini-key"),
    ] {
        let file = path.join(name);
        std::fs::write(&file, key).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(graphiti_rs::compat::config::read_secret(&file).is_err());
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert_eq!(
            graphiti_rs::compat::config::read_secret(&file).unwrap(),
            key
        );
    }
    cfg.llm_key_file = Some(path.join("llm.key"));
    cfg.gemini_key_file = Some(path.join("gemini.key"));
    let engine = Engine::new(cfg.clone()).await.unwrap();
    // Blank searches and rejected filters must not spend embedding requests.
    for query in ["", " \t\n"] {
        let nodes = engine
            .execute("search_nodes", json!({"query":query,"entity_types":[]}))
            .await
            .unwrap();
        assert_eq!(
            nodes,
            json!({"message":"No relevant nodes found","nodes":[]})
        );
        let facts = engine
            .execute("search_memory_facts", json!({"query":query}))
            .await
            .unwrap();
        assert_eq!(
            facts,
            json!({"message":"No relevant facts found","facts":[]})
        );
    }
    let nodes = engine
        .execute(
            "search_nodes",
            json!({"query":"","group_ids":[],"entity_types":["two words"]}),
        )
        .await
        .unwrap();
    assert_eq!(nodes["nodes"], json!([]));
    for labels in [vec![], vec![""], vec!["Person|Company"], vec!["two words"]] {
        assert!(
            engine
                .execute(
                    "search_nodes",
                    json!({"query":"Gaon","entity_types":labels})
                )
                .await
                .is_err()
        );
    }
    for tool in ["search_nodes", "search_memory_facts"] {
        assert!(
            engine
                .execute(tool, json!({"query":"the a and"}))
                .await
                .is_err()
        );
        assert!(engine.execute(tool, json!({"query":"?!"})).await.is_err());
        assert!(
            engine
                .execute(tool, json!({"query":"","group_ids":["unknown"]}))
                .await
                .is_err()
        );
    }
    assert_eq!(state.embeds.load(Ordering::SeqCst), 0);
    assert_eq!(state.calls.load(Ordering::SeqCst), 0);
    let across = engine
        .execute(
            "get_episodes",
            json!({"group_ids":["general_chat","user_primary"],"max_episodes":1}),
        )
        .await
        .unwrap();
    assert_eq!(across["episodes"].as_array().unwrap().len(), 1);
    let cross = engine
        .execute(
            "get_episode",
            json!({"uuid":p.episode.uuid,"group_id":"general_chat"}),
        )
        .await
        .unwrap();
    assert_eq!(cross["episode"]["name"], "Other synthetic group");
    let fair=engine.execute("search_nodes",json!({"query":"가온","group_ids":["general_chat","user_primary","general_chat"],"max_nodes":2})).await.unwrap();
    assert_eq!(fair["nodes"][0]["group_id"], "general_chat");
    assert_eq!(fair["nodes"][1]["group_id"], "user_primary");
    let ordinary = engine
        .execute(
            "search_nodes",
            json!({"query":"Gaon","entity_types":["Person"]}),
        )
        .await
        .unwrap();
    let final_lf = engine
        .execute(
            "search_nodes",
            json!({"query":"Gaon","entity_types":["Person\n"]}),
        )
        .await
        .unwrap();
    assert_eq!(ordinary, final_lf);
    let absent = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        engine
            .execute(
                "get_episode",
                json!({"uuid":absent,"group_id":"general_chat"})
            )
            .await
            .unwrap(),
        json!({"found":false,"episode":null})
    );
    let r = engine
        .execute(
            "get_episode",
            json!({"uuid":p.episode.uuid,"group_id":"user_primary"}),
        )
        .await
        .unwrap();
    assert_eq!(r["episode"]["created_at"], p.episode.created_at);
    assert!(!r.to_string().contains("embedding"));
    for group in ["hermes", "family_shared", "unknown"] {
        assert!(
            engine
                .execute(
                    "add_memory",
                    json!({"name":"synthetic","episode_body":"x","group_id":group})
                )
                .await
                .is_err()
        );
    }
    assert_eq!(state.calls.load(Ordering::SeqCst), 0);
    for gs in [
        Value::Null,
        json!([]),
        json!(["user_primary", "user_primary"]),
    ] {
        assert!(
            engine
                .execute("clear_graph", json!({"group_ids":gs}))
                .await
                .is_err()
        );
    }
    let id = uuid::Uuid::new_v4().to_string();
    let args = serde_json::to_value(request(&id)).unwrap();
    let (a, b) = tokio::join!(
        engine.execute("add_memory", args.clone()),
        engine.execute("add_memory", args.clone())
    );
    a.unwrap();
    b.unwrap();
    for _ in 0..100 {
        if state.calls.load(Ordering::SeqCst) == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        engine
            .execute("get_episode", json!({"uuid":id,"group_id":"user_primary"}))
            .await
            .unwrap()["found"],
        false
    );
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    state.release.add_permits(1);
    let committed = poll(&engine, "user_primary", &id).await;
    engine.execute("add_memory", args.clone()).await.unwrap();
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    let mut changed = args;
    changed["episode_body"] = json!("changed");
    assert!(engine.execute("add_memory", changed).await.is_err());
    let nodes=engine.execute("search_nodes",json!({"query":"Gaon","group_ids":["user_primary"],"entity_types":["Person"],"max_nodes":1})).await.unwrap();
    assert_eq!(nodes["nodes"].as_array().unwrap().len(), 1);
    let center = nodes["nodes"][0]["uuid"].as_str().unwrap();
    let facts = engine
        .execute(
            "search_memory_facts",
            json!({"query":"Gaon","group_ids":["user_primary"],"center_node_uuid":center}),
        )
        .await
        .unwrap();
    assert_eq!(facts["facts"].as_array().unwrap().len(), 3);
    assert_eq!(facts["facts"][0]["source_node_uuid"], center);
    let embeds_before_empty = state.embeds.load(Ordering::SeqCst);
    assert!(engine.execute("search_memory_facts",json!({"query":"Gaon","group_ids":["user_primary","general_chat"],"center_node_uuid":center})).await.is_err());
    assert_eq!(
        engine
            .execute("search_nodes", json!({"query":"Gaon","group_ids":[]}))
            .await
            .unwrap()["nodes"],
        json!([])
    );
    assert_eq!(
        engine
            .execute(
                "search_memory_facts",
                json!({"query":"Gaon","group_ids":[]})
            )
            .await
            .unwrap()["facts"],
        json!([])
    );
    assert_eq!(state.embeds.load(Ordering::SeqCst), embeds_before_empty);
    assert!(!facts.to_string().contains("embedding"));
    let second_id = uuid::Uuid::new_v4().to_string();
    state.release.add_permits(1);
    engine
        .execute(
            "add_memory",
            serde_json::to_value(request(&second_id)).unwrap(),
        )
        .await
        .unwrap();
    poll(&engine, "user_primary", &second_id).await;
    engine
        .execute("add_memory", serde_json::to_value(request(&id)).unwrap())
        .await
        .unwrap();
    let shared = store
        .edge("user_primary", facts["facts"][0]["uuid"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(shared.episodes.contains(&id) && shared.episodes.contains(&second_id));
    let calls = state.calls.load(Ordering::SeqCst);
    let embed_calls = state.embeds.load(Ordering::SeqCst);
    let mut read = cfg.clone();
    read.mode = WriteMode::ReadOnly;
    read.journal_dir = None;
    let read = Engine::new(read).await.unwrap();
    assert!(
        read.execute("add_memory", json!({"name":"x","episode_body":"x"}))
            .await
            .is_err()
    );
    read.execute("search_nodes", json!({"query":"Gaon"}))
        .await
        .unwrap();
    read.execute("get_status", json!({})).await.unwrap();
    assert!(
        read.execute(
            "delete_episode",
            json!({"uuid":p.episode.uuid,"group_id":"user_primary"})
        )
        .await
        .is_err()
    );
    let topology: redis::Value = redis::cmd("GRAPH.RO_QUERY")
        .arg("user_primary")
        .arg("MATCH(s:Saga)-[:HAS_EPISODE]->(e:Episodic) RETURN s.custom,e.uuid")
        .arg("--compact")
        .query_async(&mut connection)
        .await
        .unwrap();
    assert!(format!("{topology:?}").contains("retain"));
    let indexes: redis::Value = redis::cmd("GRAPH.RO_QUERY")
        .arg("user_primary")
        .arg("CALL db.indexes()")
        .arg("--compact")
        .query_async(&mut connection)
        .await
        .unwrap();
    let redis::Value::Array(parts) = indexes else {
        panic!("invalid index response")
    };
    assert_eq!(parts[1], redis::Value::Array(vec![]));
    assert_eq!(state.calls.load(Ordering::SeqCst), calls);
    assert_eq!(state.embeds.load(Ordering::SeqCst), embed_calls);
    let (server_io, client_io) = tokio::io::duplex(262144);
    let e = engine.clone();
    let server = tokio::spawn(async move {
        CompatService::new(e)
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap()
    });
    let client = ().serve(client_io).await.unwrap();
    assert_eq!(
        serde_json::to_value(client.list_all_tools().await.unwrap()).unwrap(),
        serde_json::to_value(tools()).unwrap()
    );
    let out = client
        .call_tool(
            CallToolRequestParams::new("get_episode").with_arguments(
                json!({"uuid":id,"group_id":"user_primary"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(out.structured_content.unwrap()["result"], committed);
    assert_eq!(out.is_error, Some(false));
    assert!(
        client
            .call_tool(
                CallToolRequestParams::new("get_episode")
                    .with_arguments(json!({"uuid":id}).as_object().unwrap().clone())
            )
            .await
            .is_err()
    );
    client.cancel().await.unwrap();
    server.await.unwrap();
    let provider = Providers::new(cfg).unwrap();
    provider
        .embed(&["line1\nline2".into(), "\rline3".into()])
        .await
        .unwrap();
    state.bad.store(true, Ordering::SeqCst);
    assert!(provider.embed(&["bad".into()]).await.is_err());
    let bodies = state.bodies.lock().await;
    let extraction = bodies.iter().find(|v| v.get("messages").is_some()).unwrap();
    assert_eq!(extraction["model"], "gpt-6-luna");
    assert_eq!(extraction["max_tokens"], 512);
    assert_eq!(extraction["temperature"], 1);
    assert_eq!(extraction["chat_template_kwargs"]["enable_thinking"], false);
    let sdk: Value = serde_json::from_str(include_str!("fixtures/observed-sdk-wire.json")).unwrap();
    for field in [
        "model",
        "max_tokens",
        "temperature",
        "response_format",
        "chat_template_kwargs",
    ] {
        assert_eq!(extraction[field], sdk["bodies"][0][field]);
    }
    assert!(extraction.get("extra_body").is_none());
    assert!(
        bodies
            .iter()
            .any(|b| b["requests"][0]["content"]["parts"][0]["text"] == "line1 line2")
    );
    assert!(
        bodies
            .iter()
            .any(|b| b["requests"][1]["content"]["parts"][0]["text"] == "\rline3")
    );
    drop(bodies);
    // A DB commit followed by a lost journal completion must reconcile, without another model request.
    let recovery_dir = dir();
    let recovery_journal = Journal::open(&recovery_dir, 8).unwrap();
    let recovery_id = uuid::Uuid::new_v4().to_string();
    let r = request(&recovery_id);
    recovery_journal.accept(r.clone()).unwrap();
    let recovery_job = recovery_journal.next().unwrap().unwrap();
    let mut prepared = fixture("user_primary");
    prepared.episode.uuid = recovery_id.clone();
    prepared.episode.name = r.name.clone();
    prepared.episode.content = r.episode_body.clone();
    prepared.episode.source_description = r.source_description.clone();
    prepared.episode.created_at = recovery_job.created.clone();
    prepared.episode.valid_at = recovery_job.created.clone();
    for n in &mut prepared.entities {
        n.uuid = uuid::Uuid::new_v4().to_string();
    }
    prepared.facts[0].uuid = uuid::Uuid::new_v4().to_string();
    prepared.facts[0].source_node_uuid = prepared.entities[0].uuid.clone();
    prepared.facts[0].target_node_uuid = prepared.entities[1].uuid.clone();
    prepared.facts[0].episodes = vec![recovery_id.clone()];
    prepared.episode.entity_edges = vec![prepared.facts[0].uuid.clone()];
    let prepared = store.plan(&prepared).await.unwrap();
    recovery_journal.prepared(&recovery_job, &prepared).unwrap();
    store.commit(&prepared).await.unwrap();
    drop(recovery_journal);
    let before = state.calls.load(Ordering::SeqCst);
    let mut recovery_cfg = config(recovery_dir.clone(), &base);
    recovery_cfg.redis_url = url;
    let recovered = Engine::new(recovery_cfg).await.unwrap();
    let journal_db = rusqlite::Connection::open(recovery_dir.join("jobs.sqlite")).unwrap();
    let mut done = false;
    for _ in 0..100 {
        let state: String = journal_db
            .query_row("SELECT state FROM jobs WHERE uuid=?", [&recovery_id], |r| {
                r.get(0)
            })
            .unwrap();
        if state == "committed" {
            done = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(done, "prepared commit reconciliation failed");
    assert_eq!(state.calls.load(Ordering::SeqCst), before);
    drop(journal_db);
    drop(recovered);
    tokio::time::sleep(Duration::from_millis(50)).await;
    std::fs::remove_dir_all(recovery_dir).unwrap();
    // Invalid provider output leaves no visible episode and never automatically repeats extraction.
    let failed_id = uuid::Uuid::new_v4().to_string();
    let failed = serde_json::to_value(request(&failed_id)).unwrap();
    state.release.add_permits(1);
    engine.execute("add_memory", failed.clone()).await.unwrap();
    let failure_db = rusqlite::Connection::open(path.join("jobs.sqlite")).unwrap();
    let mut failed_recorded = false;
    for _ in 0..100 {
        let phase: String = failure_db
            .query_row("SELECT state FROM jobs WHERE uuid=?", [&failed_id], |r| {
                r.get(0)
            })
            .unwrap();
        if phase == "failed" {
            failed_recorded = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(failed_recorded);
    drop(failure_db);
    assert_eq!(
        engine
            .execute(
                "get_episode",
                json!({"uuid":failed_id,"group_id":"user_primary"})
            )
            .await
            .unwrap()["found"],
        false
    );
    assert!(engine.execute("add_memory", failed).await.is_err());
    assert_eq!(state.calls.load(Ordering::SeqCst), before + 1);
    // Valid destructive MCP calls use durable tombstones and preserve baseline dangling provenance.
    engine
        .execute(
            "delete_episode",
            json!({"uuid":id,"group_id":"user_primary"}),
        )
        .await
        .unwrap();
    assert_eq!(
        engine
            .execute("get_episode", json!({"uuid":id,"group_id":"user_primary"}))
            .await
            .unwrap()["found"],
        false
    );
    assert!(
        engine
            .execute("add_memory", serde_json::to_value(request(&id)).unwrap())
            .await
            .is_err()
    );
    let edge_id = facts["facts"][0]["uuid"].as_str().unwrap();
    assert!(
        store
            .edge("user_primary", edge_id)
            .await
            .unwrap()
            .unwrap()
            .episodes
            .contains(&id)
    );
    engine
        .execute(
            "delete_entity_edge",
            json!({"uuid":edge_id,"group_id":"user_primary"}),
        )
        .await
        .unwrap();
    assert!(store.edge("user_primary", edge_id).await.unwrap().is_none());
    engine
        .execute("clear_graph", json!({"group_ids":["user_primary"]}))
        .await
        .unwrap();
    assert!(
        store
            .episode("general_chat", &other.episode.uuid)
            .await
            .unwrap()
            .is_some()
    );
    drop(read);
    drop(engine);
    tokio::time::sleep(Duration::from_millis(50)).await;
    mock.abort();
    std::fs::remove_dir_all(path).unwrap();
    store.clear("user_primary").await.unwrap();
    store.clear("general_chat").await.unwrap();
}

#[tokio::test]
#[ignore = "requires dedicated disposable FalkorDB on loopback port 16379"]
async fn compatibility_http_auth_hosts_origin_size_and_legacy_initialization() {
    use tower::ServiceExt as _;
    assert_eq!(
        std::env::var("COMPAT_TEST_DB_URL").unwrap(),
        "redis://127.0.0.1:16379"
    );
    let mut c = config(dir(), "http://127.0.0.1:1");
    c.mode = WriteMode::ReadOnly;
    c.journal_dir = None;
    let engine = Engine::new(c).await.unwrap();
    let mut http =
        graphiti_rs::config::Config::from_env_with_token(Some("dummy-test-token".into())).unwrap();
    http.allowed_hosts = vec!["localhost".into()];
    let router = graphiti_rs::http::router_service(
        CompatService::new(engine),
        &http,
        tokio_util::sync::CancellationToken::new(),
    )
    .unwrap();
    fn request(
        host: &str,
        token: Option<&str>,
        origin: Option<&str>,
        body: String,
    ) -> axum::http::Request<axum::body::Body> {
        let mut r = axum::http::Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("host", host)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");
        if let Some(token) = token {
            r = r.header("authorization", token);
        }
        if let Some(origin) = origin {
            r = r.header("origin", origin);
        }
        r.body(axum::body::Body::from(body)).unwrap()
    }
    let body = |version: &str| {
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"synthetic","version":"1"}}}).to_string()
    };
    for token in [None, Some("Bearer wrong")] {
        assert_eq!(
            router
                .clone()
                .oneshot(request("localhost", token, None, body("2025-11-25")))
                .await
                .unwrap()
                .status(),
            401
        );
    }
    for version in ["2025-11-25", "2024-11-05"] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(
                    "localhost",
                    Some("Bearer dummy-test-token"),
                    None,
                    body(version)
                ))
                .await
                .unwrap()
                .status(),
            200
        );
    }
    let bad_host = router
        .clone()
        .oneshot(request(
            "evil.example",
            Some("Bearer dummy-test-token"),
            None,
            body("2025-11-25"),
        ))
        .await
        .unwrap();
    assert!(bad_host.status().is_client_error());
    let bad_origin = router
        .clone()
        .oneshot(request(
            "localhost",
            Some("Bearer dummy-test-token"),
            Some("https://evil.example"),
            body("2025-11-25"),
        ))
        .await
        .unwrap();
    assert!(bad_origin.status().is_client_error());
    let oversized = router
        .oneshot(request(
            "localhost",
            Some("Bearer dummy-test-token"),
            None,
            " ".repeat(262145),
        ))
        .await
        .unwrap();
    assert!(oversized.status().is_client_error());
}

#[test]
fn destructive_journal_blocks_ingestion_and_preserves_receipt_history() {
    let path = dir();
    let j = Journal::open(&path, 8).unwrap();
    let mut p = fixture("user_primary");
    let mut r = request(&p.episode.uuid);
    r.name = p.episode.name.clone();
    r.episode_body = p.episode.content.clone();
    r.source_description = p.episode.source_description.clone();
    j.accept(r.clone()).unwrap();
    assert!(
        j.begin_mutation("user_primary", "clear_graph", None)
            .is_err()
    );
    let job = j.next().unwrap().unwrap();
    p.episode.created_at = job.created.clone();
    p.episode.valid_at = job.created.clone();
    j.prepared(&job, &p).unwrap();
    j.finish(&job, false).unwrap();
    assert!(
        j.begin_mutation("user_primary", "clear_graph", None)
            .is_err()
    );
    j.accept(r.clone()).unwrap();
    let job = j.next().unwrap().unwrap();
    j.finish(&job, true).unwrap();
    let op = j
        .begin_mutation("user_primary", "delete_episode", Some(&p.episode.uuid))
        .unwrap();
    assert!(
        j.accept(request(&uuid::Uuid::new_v4().to_string()))
            .is_err()
    );
    assert!(
        j.begin_mutation("user_primary", "clear_graph", None)
            .is_err()
    );
    drop(j);
    let j = Journal::open(&path, 8).unwrap();
    assert_eq!(j.mutations().unwrap()[0].id, op.id);
    assert_eq!(
        j.begin_mutation("user_primary", "delete_episode", Some(&p.episode.uuid))
            .unwrap()
            .id,
        op.id
    );
    j.finish_mutation(&op).unwrap();
    assert!(j.accept(r).is_err());
    assert!(
        j.committed("user_primary", &p.episode.uuid)
            .unwrap()
            .is_none()
    );
    let other = request(&uuid::Uuid::new_v4().to_string());
    j.accept(other.clone()).unwrap();
    let job = j.next().unwrap().unwrap();
    let mut p = fixture("user_primary");
    p.episode.uuid = job.id.clone();
    p.episode.name = job.request.name.clone();
    p.episode.content = job.request.episode_body.clone();
    p.episode.source_description = job.request.source_description.clone();
    p.episode.created_at = job.created.clone();
    p.episode.valid_at = job.created.clone();
    p.facts[0].episodes = vec![job.id.clone()];
    assert!(j.finish(&job, true).is_err());
    j.prepared(&job, &p).unwrap();
    j.finish(&job, true).unwrap();
    assert!(j.prepared(&job, &p).is_err());
    let op = j
        .begin_mutation("user_primary", "delete_entity_edge", Some(&p.facts[0].uuid))
        .unwrap();
    j.finish_mutation(&op).unwrap();
    assert!(j.accept(other.clone()).is_err());
    let op = j
        .begin_mutation("user_primary", "clear_graph", None)
        .unwrap();
    j.finish_mutation(&op).unwrap();
    assert!(j.accept(other).is_err());
    drop(j);
    let db = rusqlite::Connection::open(path.join("jobs.sqlite")).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM jobs WHERE state='cleared'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 2);
    drop(db);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
#[ignore = "requires dedicated disposable FalkorDB on loopback port 16379"]
async fn isolated_upsert_cas_provenance_deletion_clear_and_recovery() {
    let url = std::env::var("COMPAT_TEST_DB_URL").expect("explicit disposable DB required");
    assert_eq!(url, "redis://127.0.0.1:16379");
    let group = "project_parity_00000001";
    let other = "project_parity_00000002";
    let store = LegacyStore::connect(&url, WriteMode::Synthetic)
        .await
        .unwrap();
    let mut conn = redis::aio::ConnectionManager::new(redis::Client::open(url.as_str()).unwrap())
        .await
        .unwrap();
    for g in [group, other] {
        let _: redis::Value = redis::cmd("GRAPH.DELETE")
            .arg(g)
            .query_async(&mut conn)
            .await
            .unwrap_or(redis::Value::Nil);
    }
    let first = store.plan(&fixture(group)).await.unwrap();
    assert!(store.commit(&first).await.unwrap());
    // The harness explicitly adds the legacy relationship index; the reader creates none.
    let _:redis::Value=redis::cmd("GRAPH.QUERY").arg(group).arg("CREATE INDEX FOR ()-[e:RELATES_TO]-() ON (e.uuid, e.group_id, e.name, e.created_at, e.expired_at, e.valid_at, e.invalid_at)").query_async(&mut conn).await.unwrap();
    assert_eq!(
        store
            .edge(group, &first.facts[0].uuid)
            .await
            .unwrap()
            .unwrap(),
        first.facts[0]
    );
    assert_eq!(store.facts(group, "가온", None, 20).await.unwrap().len(), 1);
    let other_p = store.plan(&fixture(other)).await.unwrap();
    store.commit(&other_p).await.unwrap();
    let mut second = first.clone();
    second.baseline = None;
    second.episode.uuid = uuid::Uuid::new_v4().to_string();
    second.episode.created_at = "2026-10-03T00:00:00.111111Z".into();
    second.episode.valid_at = second.episode.created_at.clone();
    second.entities[0].summary = "updated summary".into();
    second.entities[0].created_at = second.episode.created_at.clone();
    second.entities[0]
        .attributes
        .insert("new_attr".into(), json!("kept"));
    second.facts[0].episodes = vec![second.episode.uuid.clone()];
    second.facts[0].valid_at = Some("2026-10-03T01:00:00.987654+09:00".into());
    second.facts[0].invalid_at = None;
    second.facts[0].expired_at = None;
    let planned = store.plan(&second).await.unwrap();
    assert_eq!(planned.entities[0].created_at, first.entities[0].created_at);
    assert_eq!(
        planned.facts[0].episodes,
        vec![first.episode.uuid.clone(), second.episode.uuid.clone()]
    );
    // CAS detects concurrent changes, including properties outside the intended update.
    let _: redis::Value = redis::cmd("GRAPH.QUERY")
        .arg(group)
        .arg(format!(
            "MATCH(n:Entity {{uuid:'{}'}}) SET n.concurrent='changed'",
            first.entities[0].uuid
        ))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(store.commit(&planned).await.is_err());
    assert!(
        store
            .episode(group, &second.episode.uuid)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .edge(group, &first.facts[0].uuid)
            .await
            .unwrap()
            .unwrap(),
        first.facts[0]
    );
    let _: redis::Value = redis::cmd("GRAPH.QUERY")
        .arg(group)
        .arg(format!(
            "MATCH(n:Entity {{uuid:'{}'}}) REMOVE n.concurrent",
            first.entities[0].uuid
        ))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(store.commit(&planned).await.unwrap());
    assert!(!store.commit(&planned).await.unwrap());
    assert_eq!(
        store
            .edge(group, &first.facts[0].uuid)
            .await
            .unwrap()
            .unwrap(),
        planned.facts[0]
    );
    store.verify_receipt(&first).await.unwrap();
    // Existing UUID cannot be retargeted; no accidental second relationship is created.
    let mut retarget = second.clone();
    retarget.episode.uuid = uuid::Uuid::new_v4().to_string();
    retarget.facts[0].episodes = vec![retarget.episode.uuid.clone()];
    retarget.facts[0].source_node_uuid = second.entities[1].uuid.clone();
    assert!(store.plan(&retarget).await.is_err());
    // Exact normalized names/facts reuse UUIDs and append supporting provenance once.
    let mut third = fixture(group);
    third.episode.uuid = uuid::Uuid::new_v4().to_string();
    for n in &mut third.entities {
        n.uuid = uuid::Uuid::new_v4().to_string();
        n.name = format!("  {}  ", n.name);
    }
    third.facts[0].uuid = uuid::Uuid::new_v4().to_string();
    third.facts[0].source_node_uuid = third.entities[0].uuid.clone();
    third.facts[0].target_node_uuid = third.entities[1].uuid.clone();
    third.facts[0].episodes = vec![third.episode.uuid.clone()];
    third.episode.entity_edges = vec![third.facts[0].uuid.clone()];
    let resolved = store.resolve_exact(&third).await.unwrap();
    assert_eq!(resolved.entities[0].uuid, first.entities[0].uuid);
    assert_eq!(resolved.facts[0].uuid, first.facts[0].uuid);
    assert_eq!(resolved.facts[0].episodes.len(), 3);
    let resolved = store.plan(&resolved).await.unwrap();
    store.commit(&resolved).await.unwrap();
    // Recovery verifies stable MENTIONS identifiers, rather than only the episode row.
    let _: redis::Value = redis::cmd("GRAPH.QUERY")
        .arg(group)
        .arg(format!(
            "MATCH(e:Episodic {{uuid:'{}'}})-[m:MENTIONS]->() SET m.uuid=randomUUID()",
            third.episode.uuid
        ))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(store.commit(&resolved).await.is_err());
    let _:redis::Value=redis::cmd("GRAPH.QUERY").arg(group).arg(format!("MATCH(e:Episodic {{uuid:'{}'}}),(n:Entity {{uuid:'{}'}}) CREATE(s:Saga {{uuid:'00000000-0000-4000-8000-000000000901',group_id:'{group}',retain:true}})-[:HAS_EPISODE]->(e) CREATE(c:Community {{uuid:'00000000-0000-4000-8000-000000000902',group_id:'{group}'}})-[:HAS_MEMBER]->(n) CREATE(u:Unrecognized {{uuid:'00000000-0000-4000-8000-000000000903',group_id:'{group}',retain:true}})",first.episode.uuid,first.entities[0].uuid)).query_async(&mut conn).await.unwrap();
    let path = dir();
    let j = Journal::open(&path, 8).unwrap();
    let op = j
        .begin_mutation(group, "delete_episode", Some(&first.episode.uuid))
        .unwrap();
    store.apply_mutation(&op).await.unwrap();
    // Simulate DB success followed by process exit before ledger completion.
    drop(j);
    let j = Journal::open(&path, 8).unwrap();
    let op = j.mutations().unwrap().remove(0);
    store.apply_mutation(&op).await.unwrap();
    j.finish_mutation(&op).unwrap();
    assert!(
        store
            .episode(group, &first.episode.uuid)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .node(group, &first.entities[0].uuid)
            .await
            .unwrap()
            .is_some()
    );
    let remaining = store
        .edge(group, &first.facts[0].uuid)
        .await
        .unwrap()
        .unwrap();
    assert!(remaining.episodes.contains(&first.episode.uuid)); // observed MCP baseline leaves provenance arrays untouched
    assert!(
        store
            .episode(other, &first.episode.uuid)
            .await
            .unwrap()
            .is_some()
    );
    let op = j
        .begin_mutation(group, "delete_entity_edge", Some(&first.facts[0].uuid))
        .unwrap();
    store.apply_mutation(&op).await.unwrap();
    j.finish_mutation(&op).unwrap();
    assert!(
        store
            .edge(group, &first.facts[0].uuid)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .episode(group, &second.episode.uuid)
            .await
            .unwrap()
            .unwrap()
            .entity_edges
            .contains(&first.facts[0].uuid)
    );
    let op = j.begin_mutation(group, "clear_graph", None).unwrap();
    store.apply_mutation(&op).await.unwrap();
    j.finish_mutation(&op).unwrap();
    assert!(
        store
            .node(group, &first.entities[0].uuid)
            .await
            .unwrap()
            .is_none()
    );
    let keep: redis::Value = redis::cmd("GRAPH.RO_QUERY")
        .arg(group)
        .arg("MATCH(n) RETURN labels(n),n.retain ORDER BY labels(n)")
        .arg("--compact")
        .query_async(&mut conn)
        .await
        .unwrap();
    let rendered = format!("{keep:?}");
    assert!(rendered.contains("Saga"));
    assert!(rendered.contains("Unrecognized"));
    assert!(!rendered.contains("Community"));
    assert!(
        store
            .episode(other, &first.episode.uuid)
            .await
            .unwrap()
            .is_some()
    );
    drop(j);
    std::fs::remove_dir_all(path).unwrap();
    for g in [group, other] {
        let _: redis::Value = redis::cmd("GRAPH.DELETE")
            .arg(g)
            .query_async(&mut conn)
            .await
            .unwrap();
    }
}

#[test]
fn prepared_plan_rejects_changed_creation_times_and_mention_aliases() {
    let mut p = fixture("user_primary");
    let mut b = Baseline::default();
    for n in &p.entities {
        b.entities.insert(
            n.uuid.clone(),
            Some(NodeSnapshot {
                props: json!({"uuid":n.uuid,"group_id":n.group_id,"created_at":n.created_at})
                    .as_object()
                    .unwrap()
                    .clone(),
                labels: n.labels.clone(),
            }),
        );
        b.mentions
            .insert(n.uuid.clone(), uuid::Uuid::new_v4().to_string());
    }
    for f in &p.facts {
        b.facts.insert(
            f.uuid.clone(),
            Some(FactSnapshot {
                props: json!({"uuid":f.uuid,"group_id":f.group_id,"created_at":f.created_at})
                    .as_object()
                    .unwrap()
                    .clone(),
                source: f.source_node_uuid.clone(),
                target: f.target_node_uuid.clone(),
            }),
        );
    }
    p.baseline = Some(b);
    p.validate().unwrap();
    let mut altered = p.clone();
    altered.entities[0].created_at = "2026-10-02T00:00:00Z".into();
    assert!(altered.validate().is_err());
    let mut altered = p.clone();
    altered.facts[0].created_at = "2026-10-02T00:00:00Z".into();
    assert!(altered.validate().is_err());
    let mut altered = p.clone();
    altered.entities[0].labels.clear();
    assert!(altered.validate().is_err());
    let mut altered = p.clone();
    for id in altered.baseline.as_mut().unwrap().mentions.values_mut() {
        *id = p.entities[0].uuid.clone();
    }
    assert!(altered.validate().is_err());
    let mut altered = p;
    altered
        .episode
        .entity_edges
        .push(uuid::Uuid::new_v4().to_string());
    assert!(altered.validate().is_err());
}

#[tokio::test]
#[ignore = "requires dedicated disposable FalkorDB on loopback port 16379"]
async fn full_semantic_nodes_and_facts_precede_uuid_bound_and_keep_type_filter() {
    let url = std::env::var("COMPAT_TEST_DB_URL")
        .expect("explicit disposable COMPAT_TEST_DB_URL required");
    assert_eq!(url, "redis://127.0.0.1:16379");
    let group = format!(
        "project_semantic_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let store = LegacyStore::connect(&url, WriteMode::Synthetic)
        .await
        .unwrap();
    let mut c = redis::Client::open(url)
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let basis = vector();
    let mut outlier = vec![0.; 3072];
    outlier[1] = 1.;
    let id = |i: usize| format!("00000000-0000-4000-8000-{i:012x}");
    let rows = format!(
        "[{}]",
        (1..=201)
            .map(|i| format!("{{id:'{}'}}", id(i)))
            .collect::<Vec<_>>()
            .join(",")
    );
    let query = format!(
        "CYPHER v={} rows={} UNWIND $rows AS row CREATE(n:Entity:Person {{uuid:row.id,name:'Unrelated',summary:'Fixture',group_id:'{group}',created_at:'2026-10-01T00:00:00Z',name_embedding:vecf32($v)}})",
        serde_json::to_string(&basis).unwrap(),
        rows
    );
    let _: redis::Value = redis::cmd("GRAPH.QUERY")
        .arg(&group)
        .arg(query)
        .query_async(&mut c)
        .await
        .unwrap();
    let query = format!(
        "CYPHER v={} CREATE(n:Entity:Person {{uuid:'{}',name:'Cyclist',summary:'Fixture',group_id:'{group}',created_at:'2026-10-01T00:00:00Z',name_embedding:vecf32($v)}}), (t:Entity:Company {{uuid:'{}',name:'Hub',summary:'Fixture',group_id:'{group}',created_at:'2026-10-01T00:00:00Z'}}) CREATE(n)-[e:RELATES_TO {{uuid:'{}',name:'LINK',fact:'A cyclist on a road',group_id:'{group}',created_at:'2026-10-01T00:00:00Z',episodes:[],fact_embedding:vecf32($v)}}]->(t)",
        serde_json::to_string(&outlier).unwrap(),
        id(999),
        id(1000),
        id(99999)
    );
    let _: redis::Value = redis::cmd("GRAPH.QUERY")
        .arg(&group)
        .arg(query)
        .query_async(&mut c)
        .await
        .unwrap();
    // More than the semantic candidate budget of orthogonal facts precede it.
    let rows = format!(
        "[{}]",
        (1..=201)
            .map(|i| format!("{{id:'{}',source:'{}'}}", id(i + 10000), id(i + 200000)))
            .collect::<Vec<_>>()
            .join(",")
    );
    let query = format!(
        "CYPHER v={} rows={} MATCH(t:Entity {{uuid:'{}'}}) UNWIND $rows AS row CREATE(n:Entity {{uuid:row.source,name:'Unrelated',summary:'Fixture',group_id:'{group}',created_at:'2026-10-01T00:00:00Z'}}) CREATE(n)-[e:RELATES_TO {{uuid:row.id,name:'LINK',fact:'Unrelated',group_id:'{group}',created_at:'2026-10-01T00:00:00Z',episodes:[],fact_embedding:vecf32($v)}}]->(t)",
        serde_json::to_string(&basis).unwrap(),
        rows,
        id(1000)
    );
    let _: redis::Value = redis::cmd("GRAPH.QUERY")
        .arg(&group)
        .arg(query)
        .query_async(&mut c)
        .await
        .unwrap();
    let nodes = store
        .semantic_nodes(&group, &outlier, Some(&["Person".into()]), 20)
        .await
        .unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].uuid, id(999));
    assert!(
        store
            .semantic_nodes(&group, &outlier, Some(&["Company".into()]), 20)
            .await
            .unwrap()
            .is_empty()
    );
    let facts = store.semantic_facts(&group, &outlier, 20).await.unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].uuid, id(99999));
    // Python's normalized threshold .6 accepts raw cosine .3, unlike the old .6 raw cutoff.
    let mut query_vector = vec![0.; 3072];
    query_vector[0] = (1.0f32 - 0.09).sqrt();
    query_vector[1] = 0.3;
    let nodes = store
        .semantic_nodes(&group, &query_vector, Some(&["Person".into()]), 300)
        .await
        .unwrap();
    assert!(nodes.iter().any(|n| n.uuid == id(999)));
    store.clear(&group).await.unwrap();
}

#[tokio::test]
#[ignore = "requires dedicated disposable FalkorDB on loopback port 16379"]
async fn indexed_keyword_search_uses_fulltext_order_filters_and_no_read_side_writes() {
    let url = std::env::var("COMPAT_TEST_DB_URL").expect("explicit disposable DB required");
    assert_eq!(url, "redis://127.0.0.1:16379");
    let group = format!(
        "project_fulltext_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let id = |i: usize| format!("00000000-0000-4000-8000-{i:012x}");
    let mut c = redis::Client::open(url.clone())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let rows = format!(
        "[{}]",
        (1..=201)
            .map(|i| format!(
                "{{id:'{}',source:'{}',name:'{}'}}",
                id(i),
                id(i + 10000),
                if i == 201 { "cat" } else { "catapult" }
            ))
            .collect::<Vec<_>>()
            .join(",")
    );
    let query = format!(
        "CYPHER rows={rows} UNWIND $rows AS row MERGE(t:Entity:Company {{uuid:'{}'}}) SET t.name='Harbor',t.summary='Company',t.group_id='{group}',t.created_at='2026-10-01T00:00:00Z' CREATE(n:Entity:Person {{uuid:row.source,name:row.name,summary:'Fixture',group_id:'{group}',created_at:'2026-10-01T00:00:00Z'}}) CREATE(n)-[e:RELATES_TO {{uuid:row.id,name:'VISITS',fact:row.name,group_id:'{group}',created_at:'2026-10-01T00:00:00Z',episodes:[]}}]->(t)",
        id(999999)
    );
    let _: redis::Value = redis::cmd("GRAPH.QUERY")
        .arg(&group)
        .arg(query)
        .query_async(&mut c)
        .await
        .unwrap();
    let store = LegacyStore::connect(&url, WriteMode::ReadOnly)
        .await
        .unwrap();
    // No index means the documented fallback; the read must not create one.
    assert_eq!(
        store
            .keyword_facts(&group, "cat", None, 20)
            .await
            .unwrap()
            .len(),
        20
    );
    let counts = || redis::cmd("GRAPH.RO_QUERY");
    let before: redis::Value = counts()
        .arg(&group)
        .arg("CALL db.indexes() YIELD label RETURN count(*)")
        .query_async(&mut c)
        .await
        .unwrap();
    let _: redis::Value = redis::cmd("GRAPH.QUERY")
        .arg(&group)
        .arg("CALL db.idx.fulltext.createNodeIndex('Entity','name','summary','group_id')")
        .query_async(&mut c)
        .await
        .unwrap();
    let _: redis::Value = redis::cmd("GRAPH.QUERY")
        .arg(&group)
        .arg("CREATE FULLTEXT INDEX FOR ()-[e:RELATES_TO]-() ON (e.name,e.fact,e.group_id)")
        .query_async(&mut c)
        .await
        .unwrap();
    let indexed: redis::Value = counts()
        .arg(&group)
        .arg("CALL db.indexes() YIELD label RETURN count(*)")
        .query_async(&mut c)
        .await
        .unwrap();
    assert_ne!(before, indexed);
    // Fulltext population is asynchronous; readiness must precede retrieval.
    let ready = async {
        loop {
            let reply: redis::Value = redis::cmd("GRAPH.RO_QUERY")
                .arg(&group)
                .arg("CALL db.indexes() YIELD status RETURN status")
                .query_async(&mut c)
                .await
                .unwrap();
            if let redis::Value::Array(parts) = reply
                && let Some(redis::Value::Array(rows)) = parts.get(1)
                && rows.len() == 2
                && rows.iter().all(|row| matches!(row, redis::Value::Array(cells) if cells.first() == Some(&redis::Value::BulkString(b"OPERATIONAL".to_vec()))))
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), ready)
        .await
        .unwrap();
    let hits = store
        .keyword_facts(&group, "the cat", None, 20)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].uuid, id(201));
    let nodes = store
        .keyword_nodes(&group, "the cat", Some(&["Person".into()]), 20)
        .await
        .unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].uuid, id(10201));
    assert!(
        store
            .keyword_nodes(&group, "cat", Some(&["Company".into()]), 20)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .keyword_facts(&group, "VISITS", None, 20)
            .await
            .unwrap()
            .len(),
        20
    ); // relation name is indexed too
    assert!(
        store
            .keyword_facts(&group, "the a and", None, 20)
            .await
            .is_err()
    );
    assert!(
        store
            .keyword_facts(&group, "@group_id:other | absentword", None, 20)
            .await
            .unwrap()
            .is_empty()
    );
    // Engine must preserve the store's fulltext ranking as the RRF input.
    let mut config = config(dir(), "http://127.0.0.1:9");
    config.mode = WriteMode::ReadOnly;
    config.redis_url = url;
    config.journal_dir = None;
    let engine = Engine::new(config).await.unwrap();
    let hits = engine
        .execute(
            "search_memory_facts",
            json!({"query":"the cat","group_ids":[group],"max_facts":10}),
        )
        .await
        .unwrap();
    assert_eq!(hits["facts"][0]["uuid"], id(201));
    // Rare whole-token cat ranks above frequent catapult in the fulltext
    // score. Substring re-sorting would put the two-term catapult hit first.
    let hits = engine
        .execute(
            "search_memory_facts",
            json!({"query":"cat catapult","group_ids":[group],"max_facts":10}),
        )
        .await
        .unwrap();
    assert_eq!(hits["facts"][0]["uuid"], id(201));
    let after: redis::Value = counts()
        .arg(&group)
        .arg("CALL db.indexes() YIELD label RETURN count(*)")
        .query_async(&mut c)
        .await
        .unwrap();
    // Ignore per-query timing statistics when comparing metadata result rows.
    if let (redis::Value::Array(a), redis::Value::Array(b)) = (indexed, after) {
        assert_eq!(a[1], b[1]);
    } else {
        panic!("invalid index reply");
    }
}

#[tokio::test]
#[ignore = "requires dedicated disposable FalkorDB on loopback port 16379"]
async fn episodes_select_sdk_uuid_window_before_stable_global_time_sort() {
    let url = std::env::var("COMPAT_TEST_DB_URL").expect("explicit disposable DB required");
    assert_eq!(url, "redis://127.0.0.1:16379");
    let group = format!(
        "project_episodes_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let other = format!(
        "project_episodes_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let id = |i: usize| format!("00000000-0000-4000-8000-{i:012x}");
    let mut c = redis::Client::open(url.clone())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    for (g, count) in [(&group, 201), (&other, 1)] {
        let rows = format!(
            "[{}]",
            (1..=count)
                .map(|i| format!(
                    "{{id:'{}',created:'{}'}}",
                    id(i),
                    if i == 201 {
                        "2026-09-30T01:00:00.123456+00:00"
                    } else {
                        "2026-10-01T01:00:00+14:00"
                    }
                ))
                .collect::<Vec<_>>()
                .join(",")
        );
        let query = format!(
            "CYPHER rows={rows} UNWIND $rows AS row CREATE(e:Episodic {{uuid:row.id,name:'Fixture',content:'Synthetic',source:'text',source_description:'Fixture',group_id:'{g}',created_at:row.created,valid_at:row.created,entity_edges:[]}})"
        );
        let _: redis::Value = redis::cmd("GRAPH.QUERY")
            .arg(g)
            .arg(query)
            .query_async(&mut c)
            .await
            .unwrap();
    }
    let store = LegacyStore::connect(&url, WriteMode::ReadOnly)
        .await
        .unwrap();
    let hits = store.episodes(&group, 1).await.unwrap();
    assert_eq!(hits[0].uuid, id(201));
    assert_eq!(hits[0].created_at, "2026-09-30T01:00:00.123456+00:00");
    let mut config = config(dir(), "http://127.0.0.1:9");
    config.mode = WriteMode::ReadOnly;
    config.redis_url = url;
    config.journal_dir = None;
    let engine = Engine::new(config).await.unwrap();
    let hits = engine
        .execute(
            "get_episodes",
            json!({"group_ids":[other.clone(),group.clone()],"max_episodes":1}),
        )
        .await
        .unwrap();
    assert_eq!(hits["episodes"][0]["uuid"], id(1));
    assert_eq!(hits["episodes"][0]["group_id"], other);
    for groups in [
        [other.clone(), group.clone()],
        [group.clone(), other.clone()],
    ] {
        let hits = engine
            .execute("get_episodes", json!({"group_ids":groups,"max_episodes":2}))
            .await
            .unwrap();
        assert_eq!(hits["episodes"][0]["group_id"], groups[0]);
        assert_eq!(hits["episodes"][1]["group_id"], groups[1]);
        assert!(
            hits["episodes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["uuid"] != id(201))
        );
    }
}
