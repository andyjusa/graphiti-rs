#![cfg(feature = "graphiti-compat")]
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
fn config(path: PathBuf, base: &str) -> CompatConfig {
    CompatConfig {
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
    bad.facts[0].episodes.push(uuid::Uuid::new_v4().to_string());
    assert!(bad.validate().is_err());
    let mut bad = p;
    bad.entities[0]
        .attributes
        .insert("uuid".into(), json!("evil"));
    assert!(bad.validate().is_err());
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
    // Existing entity IDs must fail atomically, without creating an episode.
    let mut collision = p.clone();
    collision.episode.uuid = uuid::Uuid::new_v4().to_string();
    for fact in &mut collision.facts {
        fact.uuid = uuid::Uuid::new_v4().to_string();
        fact.episodes = vec![collision.episode.uuid.clone()];
    }
    collision.episode.entity_edges = collision.facts.iter().map(|f| f.uuid.clone()).collect();
    assert!(store.commit(&collision).await.is_err());
    assert!(
        store
            .episode("user_primary", &collision.episode.uuid)
            .await
            .unwrap()
            .is_none()
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
    assert_eq!(facts["facts"].as_array().unwrap().len(), 1);
    assert!(!facts.to_string().contains("embedding"));
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
        engine
            .execute(
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
    provider.embed(&["line1\nline2".into()]).await.unwrap();
    state.bad.store(true, Ordering::SeqCst);
    assert!(provider.embed(&["bad".into()]).await.is_err());
    let bodies = state.bodies.lock().await;
    let extraction = bodies.iter().find(|v| v.get("messages").is_some()).unwrap();
    assert_eq!(extraction["model"], "gpt-6-luna");
    assert_eq!(extraction["max_tokens"], 512);
    assert!(
        bodies
            .iter()
            .any(|b| b["requests"][0]["content"]["parts"][0]["text"] == "line1 line2")
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
