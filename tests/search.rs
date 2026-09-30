use graphiti_rs::search::*;
#[test]
fn fusion_rewards_agreement_and_deduplicates() {
    let r = rrf(&[
        vec!["a".into(), "b".into(), "b".into()],
        vec!["b".into(), "c".into()],
    ]);
    assert_eq!(r[0].0, "b");
    assert_eq!(r.len(), 3);
    assert!((r[0].1 - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-10);
}
#[test]
fn keyword_matching_is_case_insensitive() {
    assert_eq!(keyword_score("ALICE company", "Alice works at Company"), 2);
    assert_eq!(keyword_score("Bob", "Alice"), 0);
}
