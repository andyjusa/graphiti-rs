//! Deterministic parts of the observed Python 0.28.2 pipeline.
//! A supplied decision is evidence from a caller/mock, never inferred contradiction.
use super::dto::{LegacyFact, timestamp};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
pub const SEMANTIC_MIN_SCORE: f64 = 0.6;

#[derive(Default, Deserialize, Serialize)]
pub struct Decision {
    pub duplicate_facts: Vec<i64>,
    pub contradicted_facts: Vec<i64>,
}
pub fn resolve_fact(
    extracted: &LegacyFact,
    related: &[LegacyFact],
    existing: &[LegacyFact],
    episode: &str,
    decision: &Decision,
    now: &str,
) -> Result<(LegacyFact, Vec<LegacyFact>, Vec<LegacyFact>)> {
    timestamp(now)?;
    if related.is_empty() && existing.is_empty() {
        return Ok((extracted.clone(), vec![], vec![]));
    }
    for edge in related {
        if edge.source_node_uuid == extracted.source_node_uuid
            && edge.target_node_uuid == extracted.target_node_uuid
            && crate::domain::normalize(&edge.fact) == crate::domain::normalize(&extracted.fact)
        {
            let mut result = edge.clone();
            if !result.episodes.iter().any(|id| id == episode) {
                result.episodes.push(episode.into());
            }
            return Ok((result, vec![], vec![]));
        }
    }
    let duplicates: Vec<_> = decision
        .duplicate_facts
        .iter()
        .filter_map(|i| {
            usize::try_from(*i)
                .ok()
                .and_then(|i| related.get(i))
                .cloned()
        })
        .collect();
    let mut result = duplicates.first().unwrap_or(extracted).clone();
    if !duplicates.is_empty() {
        // The baseline's model branch appends even an already present episode.
        result.episodes.push(episode.into());
    }
    result.attributes.clear(); // No custom relation models in this compatibility profile.
    let mut candidates: Vec<_> = decision
        .contradicted_facts
        .iter()
        .filter_map(|i| {
            usize::try_from(*i)
                .ok()
                .and_then(|i| related.iter().chain(existing).nth(i))
                .cloned()
        })
        .collect();
    if result.invalid_at.is_some() && result.expired_at.is_none() {
        result.expired_at = Some(now.into());
    }
    if result.expired_at.is_none() {
        candidates.sort_by_key(|e| {
            e.valid_at
                .as_deref()
                .map(timestamp)
                .transpose()
                .ok()
                .flatten()
        });
        for candidate in &candidates {
            if let (Some(c), Some(r)) = (&candidate.valid_at, &result.valid_at)
                && timestamp(c)? > timestamp(r)?
            {
                result.invalid_at = Some(c.clone());
                result.expired_at = Some(now.into());
                break;
            }
        }
    }
    for candidate in &mut candidates {
        if candidate.uuid == result.uuid {
            *candidate = result.clone();
        }
    }
    let invalidated = invalidate(&result, &candidates, now)?;
    let duplicates = duplicates
        .into_iter()
        .map(|edge| {
            if edge.uuid == result.uuid {
                result.clone()
            } else if let Some(updated) =
                invalidated.iter().find(|updated| updated.uuid == edge.uuid)
            {
                updated.clone()
            } else {
                edge
            }
        })
        .collect();
    Ok((result, invalidated, duplicates))
}
pub fn invalidate(
    resolved: &LegacyFact,
    candidates: &[LegacyFact],
    now: &str,
) -> Result<Vec<LegacyFact>> {
    let new_valid = resolved.valid_at.as_deref().map(timestamp).transpose()?;
    let new_invalid = resolved.invalid_at.as_deref().map(timestamp).transpose()?;
    timestamp(now)?;
    let mut out = vec![];
    for old in candidates {
        let old_valid = old.valid_at.as_deref().map(timestamp).transpose()?;
        let old_invalid = old.invalid_at.as_deref().map(timestamp).transpose()?;
        if matches!((old_invalid, new_valid), (Some(a), Some(b)) if a <= b)
            || matches!((new_invalid, old_valid), (Some(a), Some(b)) if a <= b)
        {
            continue;
        }
        if matches!((old_valid, new_valid), (Some(a), Some(b)) if a < b) {
            let mut old = old.clone();
            old.invalid_at = resolved.valid_at.clone();
            if old.expired_at.is_none() {
                old.expired_at = Some(now.into());
            }
            out.push(old);
        }
    }
    Ok(out)
}
/// Python uses rank_const=1 and stable insertion order for ties.
pub fn rrf(lists: &[Vec<String>]) -> Vec<String> {
    let mut order = vec![];
    let mut scores = HashMap::<String, f64>::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            if !scores.contains_key(id) {
                order.push(id.clone());
            }
            *scores.entry(id.clone()).or_default() += 1. / (rank + 1) as f64;
        }
    }
    order.sort_by(|a, b| scores[b].total_cmp(&scores[a]));
    order
}
/// Baseline Falkor distance is center / direct undirected neighbor / disconnected.
/// At min_score=0 disconnected candidates remain, in their prior order.
pub fn center_order(ids: &[String], center: &str, neighbors: &HashSet<String>) -> Vec<String> {
    let mut out = ids.to_vec();
    out.sort_by_key(|id| {
        if id == center {
            0
        } else if neighbors.contains(id) {
            1
        } else {
            2
        }
    });
    out
}
/// The deployed 0.28.2 fallback double-decodes FalkorDriver's dictionary rows.
/// Neighbor scores are therefore absent; it promotes the center and otherwise
/// retains RRF source order. Preserve that observed behavior at this boundary.
pub fn deployed_falkor_center_order(ids: &[String], center: &str) -> Vec<String> {
    let mut out = ids.to_vec();
    out.sort_by_key(|id| usize::from(id != center));
    out
}
