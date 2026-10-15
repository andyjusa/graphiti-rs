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
fn observed_python_prompts_and_entity_similarity_match_staged_pipeline() {
    use graphiti_rs::compat::{dto::LegacyEntity, node_resolution as names, prompts};
    let stages = [
        "extract_nodes.extract_message",
        "extract_nodes.extract_json",
        "extract_nodes.extract_text",
        "extract_nodes.extract_summaries_batch",
        "extract_edges.edge",
        "dedupe_nodes.nodes",
        "dedupe_edges.resolve_edge",
    ];
    let mut cases = vec![];
    let mut expected = vec![];
    for text in [
        "Gaon: I work at Saebit.",
        "가온은 새빛에서 일한다.\n나리: '안녕'",
        "Mixed \"quotes\", apostrophe's, tab\t and emoji 🦀",
    ] {
        let context = json!({"episode_content":text,"episode_timestamp":"2026-10-02T00:00:00Z","previous_episodes":["가온: 이전 대화.","A\nB"],"source_description":"Synthetic JSON","custom_extraction_instructions":"","entity_types":[{"entity_type_id":0,"entity_type_name":"Entity","entity_type_description":"default"}],"reference_time":"2026-10-02T00:00:00Z","nodes":[{"name":"Gaon","entity_types":["Entity","Person"]}],"edge_types":[],"extracted_nodes":[{"id":0,"name":"Gaon","entity_type":["Entity"],"entity_type_description":"default"}],"existing_nodes":[{"name":"Gaon","entity_types":["Entity"],"summary":"Original \"summary\""}],"entities":[{"name":"가온","summary":"Existing\nsummary","attributes":{"active":true,"optional":null}}],"existing_edges":[{"idx":0,"fact":"Gaon works at Saebit."}],"edge_invalidation_candidates":[{"idx":1,"fact":"Gaon works elsewhere."}],"new_edge":"Gaon works at Hanul."});
        for stage in stages {
            cases.push(json!({"kind":"prompt","stage":stage,"context":context}));
            expected.push(serde_json::to_value(prompts::render(stage, &context).unwrap()).unwrap());
        }
    }
    for name in [
        "",
        "a",
        "ab",
        "aaa aaa",
        "Gaon",
        "SAEBIT",
        "  John\tDoe ",
        "Acme—Research's",
        "가온 새빛",
        "İstanbul",
        "O'Reilly",
        "Long_company_name",
        "John\u{001c}Doe",
    ] {
        let fuzzy = names::fuzzy_name(name);
        let shingles = names::shingles(&fuzzy);
        cases.push(json!({"kind":"name","name":name}));
        expected.push(json!({"fuzzy":fuzzy,"high_entropy":names::high_entropy(&fuzzy),"shingles":shingles,"signature":names::signature(&shingles)}));
    }
    for (name, candidates) in [
        ("Saebit", vec!["Saebit", "Gaon"]),
        // Short exact names intentionally follow latest main; see core regressions.
        ("Saebit", vec!["Saebit", " SAEBIT "]),
        ("OpenAI Research", vec!["OpenAI_Research", "Other company"]),
        ("Unrelated Company", vec!["Other Company"]),
        ("John Doe", vec![]),
    ] {
        let nodes:Vec<LegacyEntity>=candidates.iter().enumerate().map(|(i,name)|serde_json::from_value(json!({"uuid":i.to_string(),"group_id":"user_primary","name":name,"summary":"","labels":["Entity"],"created_at":"2026-10-02T00:00:00Z","attributes":{}})).unwrap()).collect();
        cases.push(json!({"kind":"resolve","name":name,"candidates":candidates}));
        expected.push(json!(names::CandidateIndex::new(&nodes).resolve(name)));
    }
    for text in [
        "가온\u{200b} 새빛\u{feff}",
        "A\u{0001}B\nC\rD\tE\u{200d}🦀",
        "",
    ] {
        cases.push(json!({"kind":"clean","text":text}));
        expected.push(json!(graphiti_rs::compat::providers::clean_input(text)));
    }
    for text in [
        "Short summary.".to_owned(),
        "가온은 새빛에서 일한다. ".repeat(80),
        "No boundaries ".repeat(80),
        format!("{}! trailing sentence", "a".repeat(499)),
        format!("One sentence!\t{}", "b".repeat(600)),
    ] {
        cases.push(json!({"kind":"summary","text":text}));
        expected.push(json!(graphiti_rs::compat::pipeline::truncate_summary(
            &text
        )));
    }
    eprintln!(
        "independent pipeline baseline: {} synthetic cases",
        cases.len()
    );
    let mut child = Command::new("python3")
        .args(["-S", "scripts/python-0282-pipeline-baseline.py"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&cases).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(
            a, b,
            "independent observed baseline case {}: {}",
            i, cases[i]
        );
    }
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
        for algorithm in ["fallback", "operations"] {
            cases.push(json!({"kind":"center","algorithm":algorithm,"ids":ids,"center":center,"neighbors":neighbors}));
            expected.push(json!(resolution::center_order(&ids, center, &neighbors)));
        }
        cases.push(json!({"kind":"center","algorithm":"deployed-falkor","ids":ids,"center":center,"neighbors":neighbors}));
        expected.push(json!(resolution::deployed_falkor_center_order(
            &ids, center
        )));
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
    for (query, expected_query) in [
        (
            "MATCH (n:Entity)-[e:RELATES_TO {uuid:$id}]->(m:Entity) RETURN properties(e),n.uuid,m.uuid",
            graphiti_rs::compat::store::EDGE_LOOKUP_QUERY,
        ),
        (
            "MATCH (n:Entity)-[e:RELATES_TO]->(m:Entity) WHERE e.group_id=$group AND n.group_id=$group AND m.group_id=$group AND (size($terms)=0 OR any(t IN $terms WHERE toLower(e.fact) CONTAINS t)) RETURN properties(e),n.uuid,m.uuid ORDER BY e.uuid LIMIT $limit",
            graphiti_rs::compat::store::EDGE_FACTS_QUERY,
        ),
        (
            "MATCH (n:Entity)-[e:RELATES_TO]->(m:Entity) WHERE e.group_id=$group AND n.group_id=$group AND m.group_id=$group AND n.uuid=$source AND m.uuid=$target RETURN properties(e),n.uuid,m.uuid LIMIT 1001",
            graphiti_rs::compat::store::EDGE_RELATED_QUERY,
        ),
    ] {
        cases.push(json!({"kind":"edge_read","query":query}));
        expected.push(json!(expected_query));
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

#[test]
fn observed_falkor_fulltext_query_matches_rust_on_punctuation_stopwords_and_bounds() {
    let queries = [
        "Someone rides a bike",
        "quantum:state | works_at",
        "한글 검색어",
        "A\tTHE and Quantum",
        "path/to\\file + name@example.com",
        "x\u{1c}y",
        "won't",
        "red-green_blue",
        "C++ Graphiti?",
        "\"quote\"[literal]",
        "|@group_id:other",
        "_reserved",
        "nan",
        "42",
        "café",
        "Straße",
        "a near the river",
        "foo bar baz",
        "𝒙 unicode",
        "aquantum anthe",
        "ab*cd",
        "url=https://example.com",
        "punctuation?!$",
        "~prefix~",
        "X Y Z",
    ];
    let group = "project_fulltext_1234abcd";
    let mut cases: Vec<_> = queries
        .iter()
        .map(|q| json!({"kind":"fulltext","query":q,"group":group}))
        .collect();
    cases.push(json!({"kind":"fulltext","query":vec!["term";63].join(" "),"group":group}));
    cases.push(json!({"kind":"fulltext","query":vec!["term";64].join(" "),"group":group}));
    let mut child = Command::new("python3")
        .args(["-S", "scripts/python-0282-baseline.py"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(&cases).unwrap().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let values: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
    for (case, expected) in cases.iter().zip(values) {
        assert_eq!(
            graphiti_rs::compat::fulltext::query(case["query"].as_str().unwrap(), group)
                .unwrap()
                .unwrap_or_default(),
            expected,
            "{case}"
        );
    }
    // Latest upstream skips the lexical branch instead of refusing these queries.
    assert!(
        graphiti_rs::compat::fulltext::query("the a and", group)
            .unwrap()
            .is_none()
    );
    assert!(
        graphiti_rs::compat::fulltext::query("***", group)
            .unwrap()
            .is_none()
    );
    assert!(graphiti_rs::compat::fulltext::query("term", "unknown").is_err());
}

#[test]
fn future_temporal_bounds_match_pinned_latest_upstream() {
    let reference = "2026-10-15T09:00:00+09:00";
    let before = "2026-10-14T00:00:00Z";
    let after = "2026-10-23T00:00:00Z";
    let cases = [
        (
            "Nova will complete by Friday.",
            Some(reference),
            Some(after),
            Some(after),
        ),
        (
            "Nova plans to complete before Friday.",
            Some(reference),
            None,
            Some(after),
        ),
        (
            "Nova SHALL complete no later than Friday.",
            Some(reference),
            Some(before),
            Some(after),
        ),
        (
            "Nova is scheduled to run until Friday.",
            Some(reference),
            Some(after),
            Some(after),
        ),
        (
            "Nova will run through Friday, due by then.",
            Some(reference),
            None,
            Some(after),
        ),
        (
            "Nova'll complete by Friday.",
            Some(reference),
            None,
            Some(after),
        ),
        (
            "Nova is complete by Friday.",
            Some(reference),
            Some(after),
            Some(after),
        ),
        (
            "Nova will complete by Friday.",
            Some(reference),
            Some(before),
            Some(before),
        ),
        (
            "Nova will complete by Friday.",
            Some(reference),
            Some(reference),
            Some(reference),
        ),
        (
            "Nova will complete by Friday.",
            None,
            Some(after),
            Some(after),
        ),
        ("Nova will launch Friday.", Some(reference), None, None),
        (
            "Nova willing to complete by Friday.",
            Some(reference),
            Some(after),
            Some(after),
        ),
        (
            "노바는 금요일까지 완료할 예정이다.",
            Some(reference),
            Some(after),
            Some(after),
        ),
    ];
    let input: Vec<_> = cases.iter().map(|(fact,reference,valid,invalid)|
        json!({"fact":fact,"reference":reference,"valid":valid,"invalid":invalid})).collect();
    let mut child = Command::new("python3")
        .args(["-S", "scripts/python-upstream-temporal.py"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&input).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected: Vec<[Option<String>; 2]> = serde_json::from_slice(&output.stdout).unwrap();
    let instant = |value: Option<String>| {
        value.map(|v| {
            graphiti_rs::compat::dto::timestamp(&v)
                .unwrap()
                .with_timezone(&chrono::Utc)
        })
    };
    for (case, expected) in cases.into_iter().zip(expected) {
        let actual = graphiti_rs::compat::temporal::normalize_future_bounds(
            case.0,
            case.1,
            case.2.map(str::to_owned),
            case.3.map(str::to_owned),
        )
        .unwrap();
        assert_eq!(
            [instant(actual.0), instant(actual.1)],
            expected.map(instant),
            "{case:?}"
        );
    }
}
