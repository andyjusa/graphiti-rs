//! Query construction follows the observed Apache-2.0 Graphiti0.28.2
//! FalkorDriver sanitize/build_fulltext_query and its STOPWORDS list.
use super::policy;
use anyhow::{Result, ensure};
const STOPWORDS: &[&str] = &[
    "a", "is", "the", "an", "and", "are", "as", "at", "be", "but", "by", "for", "if", "in", "into",
    "it", "no", "not", "of", "on", "or", "such", "that", "their", "then", "there", "these", "they",
    "this", "to", "was", "will", "with",
];

pub fn require_operational(status: &str) -> Result<()> {
    ensure!(
        status == "OPERATIONAL",
        "fulltext index not ready; retry after indexing completes"
    );
    Ok(())
}

pub fn query(text: &str, group: &str) -> Result<Option<String>> {
    policy::read(group)?;
    let sanitized: String = text
        .chars()
        .map(|c| {
            if ",.<>{}[]\"':;!@#$%^&*()-+=~?|/\\".contains(c) || matches!(c, '\u{1c}'..='\u{1f}') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let words: Vec<_> = sanitized
        .split_whitespace()
        .filter(|w| !STOPWORDS.contains(&w.to_lowercase().as_str()))
        .collect();
    // Python measures the space-separated OR expression plus group count.
    // The SDK's empty fulltext expression fails; preserve that refusal without
    // sending malformed Cypher. Blank public search exits earlier in Engine.
    ensure!(!words.is_empty(), "query has no searchable fulltext terms");
    if words.len() * 2 >= 128 {
        return Ok(None);
    }
    Ok(Some(format!(
        "(@group_id:\"{group}\") ({})",
        words.join(" | ")
    )))
}

pub(crate) fn substring_rank(query: &str, text: &str) -> usize {
    let text = crate::domain::normalize(text);
    crate::domain::normalize(query)
        .split_whitespace()
        .filter(|t| text.contains(t))
        .count()
}
