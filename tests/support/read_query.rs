//! Missing graph reads must stay non-creating without hiding other Redis errors.
use super::*;

#[tokio::test]
#[ignore = "requires exclusively owned disposable FalkorDB on loopback port 16379"]
async fn absent_graph_reads_stay_empty_and_wrong_type_keys_fail_without_mutation() {
    let url = std::env::var("COMPAT_TEST_DB_URL").expect("explicit disposable DB required");
    assert_eq!(url, "redis://127.0.0.1:16379");
    let group = format!(
        "project_read_query_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let id = uuid::Uuid::new_v4().to_string();
    let mut connection =
        redis::aio::ConnectionManager::new(redis::Client::open(url.as_str()).unwrap())
            .await
            .unwrap();
    let store = LegacyStore::connect(&url, WriteMode::ReadOnly)
        .await
        .unwrap();
    let exists: bool = redis::cmd("EXISTS")
        .arg(&group)
        .query_async(&mut connection)
        .await
        .unwrap();
    assert!(!exists);
    assert!(store.episode(&group, &id).await.unwrap().is_none());
    assert!(
        store
            .keyword_nodes(&group, "absent", None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .keyword_facts(&group, "absent", None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let exists: bool = redis::cmd("EXISTS")
        .arg(&group)
        .query_async(&mut connection)
        .await
        .unwrap();
    assert!(!exists, "GRAPH.RO_QUERY must not create a missing graph");

    let _: () = redis::cmd("SET")
        .arg(&group)
        .arg("synthetic-string-key")
        .query_async(&mut connection)
        .await
        .unwrap();
    assert!(
        store.episode(&group, &id).await.is_err(),
        "WRONGTYPE must not be treated as a missing graph"
    );
    assert!(
        store
            .keyword_nodes(&group, "absent", None, 10)
            .await
            .is_err()
    );
    let retained: String = redis::cmd("GET")
        .arg(&group)
        .query_async(&mut connection)
        .await
        .unwrap();
    assert_eq!(retained, "synthetic-string-key");
    let _: usize = redis::cmd("DEL")
        .arg(&group)
        .query_async(&mut connection)
        .await
        .unwrap();
}
