use graphiti_rs::{config::validate_bind, domain::*};
#[test]
fn rejects_dangling_reference() {
    let extraction = Extraction {
        entities: vec![ExtractedEntity {
            name: "Alice".into(),
            summary: "".into(),
        }],
        facts: vec![ExtractedFact {
            source: "Alice".into(),
            target: "Missing".into(),
            relation: "knows".into(),
            fact: "Alice knows Missing".into(),
            supersedes: vec![],
        }],
    };
    assert!(extraction.validate().is_err());
}
#[test]
fn rejects_empty_entity() {
    assert!(
        Extraction {
            entities: vec![ExtractedEntity {
                name: "  ".into(),
                summary: "".into()
            }],
            facts: vec![]
        }
        .validate()
        .is_err()
    );
}
#[test]
fn temporal_intervals_are_half_open() {
    assert!(valid_at(10, Some(20), 10));
    assert!(!valid_at(10, Some(20), 20));
    assert!(!valid_at(10, None, 9));
    assert!(valid_at(10, None, 100));
}
#[test]
fn public_http_requires_token() {
    assert!(validate_bind("0.0.0.0:8080".parse().unwrap(), None).is_err());
    assert!(validate_bind("127.0.0.1:8080".parse().unwrap(), None).is_ok());
    assert!(validate_bind("0.0.0.0:8080".parse().unwrap(), Some("secret")).is_ok());
}
