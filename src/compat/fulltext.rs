//! Query construction follows the observed Apache-2.0 Graphiti0.28.2
//! FalkorDriver and STOPWORDS, with upstream empty-query/backtick fixes.
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
    Ok(match build_query(text, group)? {
        Query::Search(query) => Some(query),
        Query::Empty | Query::TooLong => None,
    })
}

pub(crate) enum Query {
    Empty,
    TooLong,
    Search(String),
}

pub(crate) fn build_query(text: &str, group: &str) -> Result<Query> {
    policy::read(group)?;
    let sanitized: String = text
        .chars()
        .map(|c| {
            if ",.<>{}[]\"':;!@#$%^&*()-+=~?|/\\`".contains(c) || matches!(c, '\u{1c}'..='\u{1f}') {
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
    // No lexical terms must not abort the independent semantic search. Keep
    // this separate from the length bound: unindexed graphs retain their
    // existing bounded substring fallback for long queries.
    if words.is_empty() {
        return Ok(Query::Empty);
    }
    if words.len() * 2 >= 128 {
        return Ok(Query::TooLong);
    }
    Ok(Query::Search(format!(
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
