use chrono::{TimeZone, Utc};
use graphiti_rs::{domain::*, store::Store};
async fn setup() -> Store {
    Store::connect(
        &std::env::var("TEST_FALKORDB_URL").unwrap_or_else(|_| "redis://127.0.0.1:16379".into()),
        &format!("test_{}", uuid::Uuid::new_v4().simple()),
    )
    .await
    .unwrap()
}
fn episode(id: &str, group: &str, t: i64) -> Episode {
    Episode {
        id: id.into(),
        group_id: group.into(),
        content: "Alice works at Acme".into(),
        source: "test".into(),
        reference_time: Utc.timestamp_opt(t, 0).unwrap(),
        created_at: Utc::now(),
    }
}
fn extraction() -> Extraction {
    Extraction {
        entities: vec![
            ExtractedEntity {
                name: "Alice".into(),
                summary: "Person".into(),
            },
            ExtractedEntity {
                name: "Acme".into(),
                summary: "Company".into(),
            },
        ],
        facts: vec![ExtractedFact {
            source: "Alice".into(),
            target: "Acme".into(),
            relation: "works_at".into(),
            fact: "Alice works at Acme".into(),
            supersedes: vec![],
        }],
    }
}
#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn persistence_idempotency_and_isolation() {
    let s = setup().await;
    let e = episode("one", "g", 10);
    let x = extraction();
    let (a, b) = tokio::join!(s.ingest(&e, &x, None, None), s.ingest(&e, &x, None, None));
    assert_ne!(a.unwrap().inserted, b.unwrap().inserted);
    assert_eq!(
        s.episode("g", "one").await.unwrap().unwrap().content,
        e.content
    );
    assert!(s.episode("other", "one").await.unwrap().is_none());
    assert_eq!(s.facts("g", 10000, 50).await.unwrap().len(), 1);
    assert!(s.facts("other", 10000, 50).await.unwrap().is_empty());
}
#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn reconnect_reuses_existing_range_indexes() {
    let url =
        std::env::var("TEST_FALKORDB_URL").unwrap_or_else(|_| "redis://127.0.0.1:16379".into());
    let graph = format!("test_reconnect_{}", uuid::Uuid::new_v4().simple());
    let first = Store::connect(&url, &graph).await.unwrap();
    let e = episode("persistent", "g", 10);
    first.ingest(&e, &extraction(), None, None).await.unwrap();
    drop(first);
    let reopened = Store::connect(&url, &graph).await.unwrap();
    assert_eq!(
        reopened
            .episode("g", "persistent")
            .await
            .unwrap()
            .unwrap()
            .content,
        e.content
    );
    assert_eq!(reopened.facts("g", 20000, 50).await.unwrap().len(), 1);
}
#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn shared_provenance_survives_deletion() {
    let s = setup().await;
    let x = extraction();
    s.ingest(&episode("one", "g", 10), &x, None, None)
        .await
        .unwrap();
    s.ingest(&episode("two", "g", 10), &x, None, None)
        .await
        .unwrap();
    assert!(s.delete_episode("g", "one").await.unwrap());
    let facts = s.facts("g", 20000, 50).await.unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].episodes, vec!["two"]);
    assert!(s.delete_episode("g", "two").await.unwrap());
    assert!(s.facts("g", 20000, 50).await.unwrap().is_empty());
    assert!(!s.delete_episode("g", "two").await.unwrap());
}
#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn explicit_replacement_preserves_history() {
    let s = setup().await;
    let x = extraction();
    let result = s
        .ingest(&episode("one", "g", 10), &x, None, None)
        .await
        .unwrap();
    let mut y = x.clone();
    y.facts[0].fact = "Alice no longer works at Acme".into();
    y.facts[0].supersedes = result.fact_ids;
    s.ingest(&episode("two", "g", 20), &y, None, None)
        .await
        .unwrap();
    assert_eq!(
        s.facts("g", 15000, 50).await.unwrap()[0].fact,
        "Alice works at Acme"
    );
    assert_eq!(
        s.facts("g", 20000, 50).await.unwrap()[0].fact,
        "Alice no longer works at Acme"
    );
}
#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn rejects_wrong_embedding_space() {
    let s = setup().await;
    let x = extraction();
    s.ingest(
        &episode("one", "g", 10),
        &x,
        Some(&[vec![1.0, 0.0]]),
        Some("model-a:2"),
    )
    .await
    .unwrap();
    assert!(
        s.ingest(
            &episode("two", "g", 20),
            &x,
            Some(&[vec![1.0, 0.0]]),
            Some("model-b:2")
        )
        .await
        .is_err()
    );
    assert!(s.episode("g", "two").await.unwrap().is_none());
}
#[tokio::test]
#[ignore = "requires disposable FalkorDB"]
async fn failed_episode_conflict_does_not_pin_embedding_space() {
    let s = setup().await;
    let x = extraction();
    let e = episode("one", "g", 10);
    s.ingest(&e, &x, None, None).await.unwrap();
    let mut conflict = e;
    conflict.content = "Different".into();
    assert!(
        s.ingest(&conflict, &x, Some(&[vec![1.0, 0.0]]), Some("model:2"))
            .await
            .is_err()
    );
    assert_eq!(s.embedding_space("g").await.unwrap().as_deref(), Some(""));
}
