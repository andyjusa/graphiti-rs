//! A durable completion fault must stop acceptance until restart/reconciliation.
use super::*;
use graphiti_rs::compat::config::{EntityType, IngestionPipeline};

#[tokio::test]
#[ignore = "requires exclusively owned disposable FalkorDB on loopback port 16379"]
async fn stopped_worker_refuses_new_jobs_and_reconciles_prepared_commit_on_restart() {
    assert_eq!(
        std::env::var("COMPAT_TEST_DB_URL").unwrap(),
        "redis://127.0.0.1:16379"
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state = StagedMock::default();
    let embedding_calls = Arc::new(AtomicUsize::new(0));
    let counter = embedding_calls.clone();
    let app = axum::Router::new()
        .route("/chat/completions", axum::routing::post(staged_llm))
        .route(
            "/models/gemini-embedding-2:batchEmbedContents",
            axum::routing::post(move |v: axum::Json<Value>| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    staged_embeddings(v).await
                }
            }),
        )
        .with_state(state.clone());
    let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let path = dir();
    let mut config = config(path.clone(), &base);
    config.pipeline = IngestionPipeline::Graphiti;
    config.entity_types = vec![
        EntityType {
            name: "Person".into(),
            description: "A person".into(),
        },
        EntityType {
            name: "Organization".into(),
            description: "A company".into(),
        },
    ];
    let engine = Engine::new(config.clone()).await.unwrap();
    let db = rusqlite::Connection::open(path.join("jobs.sqlite")).unwrap();
    // Real SQLite abort after the graph commit, without any runtime test hook.
    db.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE OF state ON jobs WHEN NEW.state IN ('committed','failed') BEGIN SELECT RAISE(ABORT,'synthetic completion fault'); END;").unwrap();
    let group = format!(
        "project_worker_failure_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let id = uuid::Uuid::new_v4().to_string();
    let args = json!({"uuid":id,"name":"completion fault","episode_body":"Gaon works at Saebit.","source":"message","group_id":group});
    engine.execute("add_memory", args.clone()).await.unwrap();
    poll(&engine, &group, &id).await;
    assert_eq!(
        db.query_row("SELECT state FROM jobs WHERE uuid=?", [&id], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "prepared"
    );
    let mut stopped = false;
    for _ in 0..100 {
        let status = engine.execute("get_status", json!({})).await.unwrap();
        if status["status"] == "error" {
            assert!(
                status["message"]
                    .as_str()
                    .unwrap()
                    .contains("worker stopped")
            );
            stopped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(stopped, "dead ingestion worker still reports healthy");
    let calls = state.trace.lock().await.len();
    assert_eq!(calls, 3);
    let embeds = embedding_calls.load(Ordering::SeqCst);
    assert!(embeds > 0);
    // Healthy graph reads remain possible, but no further accepted receipt is created.
    assert_eq!(
        engine
            .execute("get_episode", json!({"uuid":id,"group_id":group}))
            .await
            .unwrap()["found"],
        true
    );
    let mut second = args.clone();
    let second_id = uuid::Uuid::new_v4().to_string();
    second["uuid"] = json!(second_id);
    assert!(
        engine
            .execute("add_memory", second)
            .await
            .unwrap_err()
            .to_string()
            .contains("worker stopped")
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(state.trace.lock().await.len(), calls);
    assert_eq!(embedding_calls.load(Ordering::SeqCst), embeds);
    assert!(
        engine
            .execute("clear_graph", json!({"group_ids":[group]}))
            .await
            .unwrap_err()
            .to_string()
            .contains("worker stopped")
    );
    for tool in ["delete_episode", "delete_entity_edge"] {
        assert!(
            engine
                .execute(tool, json!({"uuid":id,"group_id":group}))
                .await
                .unwrap_err()
                .to_string()
                .contains("worker stopped")
        );
    }
    db.execute_batch("DROP TRIGGER reject_completion").unwrap();
    drop(engine);
    let reopened = Engine::new(config).await.unwrap();
    let mut reconciled = false;
    for _ in 0..100 {
        if db
            .query_row("SELECT state FROM jobs WHERE uuid=?", [&id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
            == "committed"
        {
            reconciled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        reconciled,
        "prepared graph commit not reconciled after restart"
    );
    assert_eq!(
        state.trace.lock().await.len(),
        calls,
        "restart called the model again"
    );
    assert_eq!(embedding_calls.load(Ordering::SeqCst), embeds);
    assert_eq!(
        reopened.execute("get_status", json!({})).await.unwrap()["status"],
        "ok"
    );
    reopened.execute("add_memory", args).await.unwrap();
    assert_eq!(state.trace.lock().await.len(), calls);
    assert_eq!(embedding_calls.load(Ordering::SeqCst), embeds);
    println!(
        "WORKER_FAILURE_REPORT {}",
        json!({"journal_before_restart":"prepared","journal_after_restart":"committed","health_after_failure":"error","health_after_restart":"ok","mutations_refused":4,"accepted_jobs":1,"mock_llm_requests":calls,"mock_embedding_requests":embeds,"llm_requests_after_restart":0,"embedding_requests_after_restart":0,"real_provider_calls":0})
    );
    reopened.store.clear(&group).await.unwrap();
    drop(reopened);
    mock.abort();
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(db);
    std::fs::remove_dir_all(path).unwrap();
}
