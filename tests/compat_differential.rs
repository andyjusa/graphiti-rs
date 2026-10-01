#![cfg(feature = "graphiti-compat")]
use graphiti_rs::compat::{
    dto::LegacyFact,
    policy,
    resolution::{self, Decision},
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    io::Write,
    process::{Command, Stdio},
};

fn edge(id: &str, valid: Option<&str>) -> LegacyFact {
    serde_json::from_value(json!({"uuid":id,"group_id":"user_primary","source_node_uuid":"source","target_node_uuid":"target","name":"WORKS_AT","fact":id,"created_at":"2026-01-01T00:00:00.000001Z","valid_at":valid,"invalid_at":null,"expired_at":null,"episodes":["old-episode"],"attributes":{"retained":"original"}})).unwrap()
}
fn canonical(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if ["created_at", "valid_at", "invalid_at", "expired_at"].contains(&key.as_str())
                    && v.is_string()
                {
                    *v = json!(
                        graphiti_rs::compat::dto::timestamp(v.as_str().unwrap())
                            .unwrap()
                            .to_rfc3339()
                    );
                } else {
                    *v = canonical(v.take());
                }
            }
        }
        Value::Array(list) => {
            for v in list {
                *v = canonical(v.take());
            }
        }
        _ => {}
    }
    value
}
#[test]
fn observed_python_functions_match_rust_on_synthetic_decisions() {
    let now = "2026-10-01T10:00:00.123456Z";
    let mut cases = vec![];
    let mut expected = vec![];
    for lists in [
        vec![vec!["a", "b"], vec!["b", "c"]],
        vec![vec!["b", "a"], vec!["a", "b"]],
        vec![],
        vec![vec!["a", "a"]],
    ] {
        let lists: Vec<Vec<String>> = lists
            .into_iter()
            .map(|l| l.into_iter().map(str::to_owned).collect())
            .collect();
        cases.push(json!({"kind":"rrf","lists":lists}));
        expected.push(json!(resolution::rrf(&lists)));
    }
    for (ids, center, neighbors) in [
        (
            vec!["far", "neighbor", "center", "other"],
            "center",
            vec!["neighbor"],
        ),
        (vec!["far", "other"], "absent", vec![]),
        (vec!["first", "second"], "center", vec!["first", "second"]),
    ] {
        let ids: Vec<String> = ids.into_iter().map(str::to_owned).collect();
        let neighbors: HashSet<String> = neighbors.into_iter().map(str::to_owned).collect();
        cases.push(json!({"kind":"center","ids":ids,"center":center,"neighbors":neighbors}));
        expected.push(json!(resolution::center_order(&ids, center, &neighbors)));
    }
    for groups in [
        None,
        Some(vec![]),
        Some(vec![
            "general_chat".to_string(),
            "user_primary".to_string(),
            "general_chat".to_string(),
        ]),
    ] {
        cases.push(json!({"kind":"groups","groups":groups}));
        expected.push(json!(policy::groups(groups.as_deref()).unwrap()));
    }
    let texts = vec!["  Gaon\tWORKS\n at Saebit  ", "가온\u{2003}새빛", "A\r\nB"];
    cases.push(json!({"kind":"normalize","texts":texts}));
    expected.push(json!(
        texts
            .iter()
            .map(|t| graphiti_rs::domain::normalize(t))
            .collect::<Vec<_>>()
    ));
    let old = edge("old", Some("2026-01-01T09:00:00.123456+09:00"));
    let new = edge("new", Some("2026-02-01T00:00:00.654321Z"));
    let newer = edge("newer", Some("2026-03-01T00:00:00Z"));
    let mut exact = old.clone();
    exact.fact = "  OLD  ".into();
    let mut already_invalid = old.clone();
    already_invalid.invalid_at = Some("2026-01-02T00:00:00Z".into());
    let mut expired = old.clone();
    expired.expired_at = Some("2026-01-03T00:00:00.222222Z".into());
    let combinations = vec![
        (new.clone(), vec![], vec![], Decision::default()),
        (
            exact,
            vec![old.clone()],
            vec![newer.clone()],
            Decision {
                duplicate_facts: vec![],
                contradicted_facts: vec![0, 1],
            },
        ),
        (
            new.clone(),
            vec![old.clone()],
            vec![],
            Decision {
                duplicate_facts: vec![-1, 0, 99],
                contradicted_facts: vec![],
            },
        ),
        (
            new.clone(),
            vec![old.clone()],
            vec![],
            Decision {
                duplicate_facts: vec![],
                contradicted_facts: vec![-1, 0, 99],
            },
        ),
        (
            new.clone(),
            vec![],
            vec![newer],
            Decision {
                duplicate_facts: vec![],
                contradicted_facts: vec![0],
            },
        ),
        (
            new.clone(),
            vec![],
            vec![already_invalid],
            Decision {
                duplicate_facts: vec![],
                contradicted_facts: vec![0],
            },
        ),
        (
            new,
            vec![],
            vec![expired],
            Decision {
                duplicate_facts: vec![],
                contradicted_facts: vec![0],
            },
        ),
        (
            edge("no-time", None),
            vec![],
            vec![old.clone()],
            Decision {
                duplicate_facts: vec![],
                contradicted_facts: vec![0],
            },
        ),
        (old.clone(), vec![old], vec![], Decision::default()),
    ];
    for (extracted, related, existing, decision) in combinations {
        cases.push(json!({"kind":"resolve","extracted":extracted,"related":related,"existing":existing,"episode":"new-episode","now":now,"decision":decision}));
        let (resolved, invalidated, duplicates) = resolution::resolve_fact(
            &extracted,
            &related,
            &existing,
            "new-episode",
            &decision,
            now,
        )
        .unwrap();
        expected.push(json!([resolved, invalidated, duplicates]));
    }
    for content in [
        "{\"entities\":[],\"facts\":[]}",
        "prefix ```json\n{\"entities\":[],\"facts\":[]}\n``` suffix",
        "```json\n{\"entities\":[],\"facts\":[]}\n```",
        "{\"entities\":[],\"facts\":[]} {\"ignored\":true}",
        "{\"type\":\"object\",\"properties\":{\"entities\":{\"value\":[]},\"facts\":{\"value\":[]}}}",
        "[]",
        "incomplete {",
        "null",
    ] {
        cases.push(json!({"kind":"decode","content":content}));
        expected.push(
            graphiti_rs::compat::providers::decode_extraction(content)
                .unwrap_or_else(|_| json!({"error":true})),
        );
    }
    let mut child = Command::new("python3")
        .args(["-S", "scripts/python-0282-baseline.py"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Python 3 required for observed-source differential tests");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_vec(&cases).unwrap().as_slice())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let actual: Vec<Value> = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual.into_iter().zip(expected).enumerate() {
        assert_eq!(
            canonical(a),
            canonical(b),
            "differential case {i}: {}",
            cases[i]
        );
    }
}
#[test]
fn baseline_uuid_and_destructive_scope_are_observed_not_assumed() {
    let cases = json!([
        {"kind":"stable_uuid","uuid":"synthetic","exists":true,"now":"2026-10-01T00:00:00Z"},
        {"kind":"stable_uuid","uuid":"synthetic","exists":false,"now":"2026-10-01T00:00:00Z"},
        {"kind":"delete_episode","uuid":"synthetic"},
        {"kind":"delete_edge","uuid":"synthetic"},
        {"kind":"clear"},
        {"kind":"recipes"}
    ]);
    let mut child = Command::new("python3")
        .args(["-S", "scripts/python-0282-baseline.py"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_vec(&cases).unwrap().as_slice())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let values: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        values[5]["recipes"],
        json!({
            "EDGE_HYBRID_SEARCH_RRF":{"search_methods":["bm25","cosine_similarity"],"reranker":"rrf"},
            "EDGE_HYBRID_SEARCH_NODE_DISTANCE":{"search_methods":["bm25","cosine_similarity"],"reranker":"node_distance"},
            "NODE_HYBRID_SEARCH_RRF":{"search_methods":["bm25","cosine_similarity"],"reranker":"rrf"}
        })
    );
    assert_eq!(
        values[5]["semantic_min_score"],
        json!(resolution::SEMANTIC_MIN_SCORE)
    );
    assert_eq!(values[0]["content"], "old content");
    assert_eq!(values[1]["content"], "new content");
    let episode = values[2].as_array().unwrap();
    assert_eq!(episode.len(), 3);
    for (op, label) in episode.iter().zip(["Entity", "Episodic", "Community"]) {
        assert_eq!(
            op["query"],
            format!("MATCH (n:{label} {{uuid: $uuid}}) DETACH DELETE n")
        );
    }
    assert_eq!(
        values[3][0]["query"],
        "MATCH (n)-[e:MENTIONS|RELATES_TO|HAS_MEMBER {uuid: $uuid}]->(m) DELETE e"
    );
    for (op, label) in values[4]
        .as_array()
        .unwrap()
        .iter()
        .zip(["Entity", "Episodic", "Community"])
    {
        assert_eq!(
            op["query"],
            format!("MATCH (n:{label}) WHERE n.group_id IN $group_ids DETACH DELETE n")
        );
    }
}
