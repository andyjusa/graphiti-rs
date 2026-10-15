//! Future assertion bounds adapted from getzep/graphiti temporal_edge_utils.py,
//! aca369364226a666ca541b061ff2a1a54b80f2ff (Apache-2.0).
//! Stored historical timestamp spellings are retained unless a bound changes.
use super::dto::timestamp;
use anyhow::Result;
use regex::Regex;
use std::sync::OnceLock;

fn pattern(slot: &'static OnceLock<Regex>, expression: &str) -> &'static Regex {
    slot.get_or_init(|| Regex::new(expression).expect("fixed temporal pattern"))
}

pub fn normalize_future_bounds(
    fact: &str,
    reference_time: Option<&str>,
    mut valid_at: Option<String>,
    mut invalid_at: Option<String>,
) -> Result<(Option<String>, Option<String>)> {
    static FUTURE: OnceLock<Regex> = OnceLock::new();
    static DEADLINE: OnceLock<Regex> = OnceLock::new();
    static END: OnceLock<Regex> = OnceLock::new();
    let Some(reference_time) = reference_time else {
        return Ok((valid_at, invalid_at));
    };
    let reference = timestamp(reference_time)?;
    let valid = valid_at.as_deref().map(timestamp).transpose()?;
    let invalid = invalid_at.as_deref().map(timestamp).transpose()?;
    if !pattern(&FUTURE, r"(?i)\b(will|shall|is going to|are going to|going to|plans? to|planned to|planning to|intends? to|intended to|expects? to|expected to|promises? to|promised to|scheduled to|is scheduled to|are scheduled to|set to|aims? to|aimed to|committed to)\b|[a-z]+'ll\b").is_match(fact) {
        return Ok((valid_at, invalid_at));
    }
    if valid.is_none_or(|value| value > reference) {
        valid_at = Some(reference_time.to_owned());
    }
    if invalid.is_some_and(|value| value > reference)
        && pattern(&DEADLINE, r"(?i)\b(by|before|no later than|deadline|due by|due on)\b").is_match(fact)
        && !pattern(&END, r"(?i)\b(until|through|thru|expires?|expired|ends?|ended|ending|stops?|stopped|ceases?|ceased|no longer)\b").is_match(fact)
    {
        invalid_at = None;
    }
    Ok((valid_at, invalid_at))
}
