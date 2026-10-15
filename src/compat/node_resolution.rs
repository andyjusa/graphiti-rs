//! Observed exact/entropy/32-permutation MinHash resolution before model escalation.
use super::dto::LegacyEntity;
use blake2::{Blake2b, Digest, digest::consts::U8};
use std::collections::{BTreeMap, BTreeSet};

pub fn fuzzy_name(name: &str) -> String {
    let lower = crate::domain::normalize(name);
    lower
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '\'' || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn high_entropy(name: &str) -> bool {
    if name.len() < 6 && name.split_whitespace().count() < 2 {
        return false;
    }
    let mut counts = BTreeMap::<char, usize>::new();
    for c in name.chars().filter(|c| *c != ' ') {
        *counts.entry(c).or_default() += 1;
    }
    let total: usize = counts.values().sum();
    total > 0
        && counts
            .values()
            .map(|n| {
                let p = *n as f64 / total as f64;
                -p * p.log2()
            })
            .sum::<f64>()
            >= 1.5
}
pub fn shingles(name: &str) -> BTreeSet<String> {
    let chars: Vec<_> = name.chars().filter(|c| *c != ' ').collect();
    if chars.len() < 2 {
        return if chars.is_empty() {
            BTreeSet::new()
        } else {
            BTreeSet::from([chars.iter().collect()])
        };
    }
    chars
        .windows(3)
        .map(|chunk| chunk.iter().collect())
        .collect()
}
pub fn signature(set: &BTreeSet<String>) -> Vec<u64> {
    if set.is_empty() {
        return vec![];
    }
    (0..32)
        .map(|seed| {
            set.iter()
                .map(|shingle| {
                    let digest = Blake2b::<U8>::digest(format!("{seed}:{shingle}").as_bytes());
                    u64::from_be_bytes(digest.as_slice().try_into().unwrap())
                })
                .min()
                .unwrap()
        })
        .collect()
}
/// Build exact-name and LSH buckets once per bounded ingestion context.
pub struct CandidateIndex {
    exact: BTreeMap<String, Vec<usize>>,
    shingles: Vec<BTreeSet<String>>,
    buckets: BTreeMap<(usize, Vec<u64>), Vec<usize>>,
}
impl CandidateIndex {
    pub fn new(candidates: &[LegacyEntity]) -> Self {
        let mut index = Self {
            exact: BTreeMap::new(),
            shingles: vec![],
            buckets: BTreeMap::new(),
        };
        for (i, node) in candidates.iter().enumerate() {
            index
                .exact
                .entry(crate::domain::normalize(&node.name))
                .or_default()
                .push(i);
            let set = shingles(&fuzzy_name(&node.name));
            for (band, values) in signature(&set).chunks(4).enumerate() {
                index
                    .buckets
                    .entry((band, values.to_vec()))
                    .or_default()
                    .push(i);
            }
            index.shingles.push(set);
        }
        index
    }
    pub fn resolve(&self, name: &str) -> Option<usize> {
        // Short and non-ASCII names are reliable exact identities too. Entropy
        // only guards the fuzzy path, as in upstream _resolve_with_similarity.
        if let Some(matches) = self.exact.get(&crate::domain::normalize(name)) {
            return if matches.len() == 1 {
                matches.first().copied()
            } else {
                None
            };
        }
        let fuzzy = fuzzy_name(name);
        if !high_entropy(&fuzzy) {
            return None;
        }
        let set = shingles(&fuzzy);
        let mut eligible = BTreeSet::new();
        for (band, values) in signature(&set).chunks(4).enumerate() {
            if let Some(ids) = self.buckets.get(&(band, values.to_vec())) {
                eligible.extend(ids.iter().copied());
            }
        }
        let mut best = None;
        let mut best_score = 0.;
        for i in eligible {
            let other = &self.shingles[i];
            let union = set.union(other).count();
            let score = if union == 0 {
                1.
            } else {
                set.intersection(other).count() as f64 / union as f64
            };
            // Stable candidate order resolves Python set traversal ties reproducibly.
            if score > best_score {
                best_score = score;
                best = Some(i);
            }
        }
        if best_score >= 0.9 { best } else { None }
    }
}
pub fn deterministic(name: &str, candidates: &[LegacyEntity]) -> Option<usize> {
    CandidateIndex::new(candidates).resolve(name)
}

#[cfg(test)]
mod upstream_tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, name: &str) -> LegacyEntity {
        serde_json::from_value(json!({"uuid":id,"name":name,"group_id":"user_primary","summary":"","created_at":"2026-01-01T00:00:00Z","labels":["Entity"]})).unwrap()
    }

    #[test]
    fn short_and_non_ascii_exact_names_resolve_before_entropy_gate() {
        for name in ["Nate", "AB", "가온", "王明", "aaaaaa"] {
            let candidates = vec![node("canonical", name)];
            assert_eq!(CandidateIndex::new(&candidates).resolve(name), Some(0));
            let mut ambiguous = candidates;
            ambiguous.push(node("different", &format!(" {name} ")));
            assert_eq!(CandidateIndex::new(&ambiguous).resolve(name), None);
        }
        assert_eq!(
            CandidateIndex::new(&[node("other", "Nate")]).resolve("Nat"),
            None
        );
    }
}
