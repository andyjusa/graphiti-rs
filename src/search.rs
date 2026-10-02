use crate::{
    domain::*,
    embeddings::{Embedder, cosine},
    store::Store,
};
use anyhow::{Result, ensure};
use chrono::Utc;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

pub fn rrf(lists: &[Vec<String>]) -> Vec<(String, f64)> {
    let mut scores = HashMap::<String, f64>::new();
    for list in lists {
        let mut seen = HashSet::new();
        for (rank, id) in list.iter().enumerate() {
            if seen.insert(id) {
                *scores.entry(id.clone()).or_default() += 1.0 / (61 + rank) as f64;
            }
        }
    }
    let mut result: Vec<_> = scores.into_iter().collect();
    result.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    result
}
pub fn keyword_score(query: &str, text: &str) -> usize {
    let text = normalize(text);
    let tokens: HashSet<_> = normalize(query)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    tokens.iter().filter(|t| text.contains(t.as_str())).count()
}
#[derive(Serialize)]
pub struct FactHit {
    pub score: f64,
    #[serde(flatten)]
    pub fact: Fact,
}
#[derive(Clone)]
pub struct Search {
    pub store: Store,
    pub embedder: Embedder,
    pub candidates: usize,
    pub admission: std::sync::Arc<tokio::sync::Semaphore>,
}
impl Search {
    pub async fn facts(&self, request: SearchRequest) -> Result<Vec<FactHit>> {
        request.validate()?;
        let _permit = self
            .admission
            .try_acquire()
            .map_err(|_| anyhow::anyhow!("search busy; retry later"))?;
        ensure!(
            (50..=2000).contains(&self.candidates),
            "invalid candidate bound"
        );
        let at = request.as_of.unwrap_or_else(Utc::now).timestamp_millis();
        let mut candidates: HashMap<String, Fact> = self
            .store
            .facts(&request.group_id, at, self.candidates)
            .await?
            .into_iter()
            .map(|f| (f.id.clone(), f))
            .collect();
        // Keyword candidates are fetched independently of the recent window.
        let tokens: Vec<String> = normalize(&request.query)
            .split_whitespace()
            .take(4)
            .map(str::to_owned)
            .collect();
        for token in &tokens {
            for fact in self
                .store
                .fact_candidates(&request.group_id, at, self.candidates, Some(token))
                .await?
            {
                candidates.entry(fact.id.clone()).or_insert(fact);
            }
        }
        let mut keywords: Vec<_> = candidates
            .values()
            .filter_map(|f| {
                let score = keyword_score(&request.query, &f.fact);
                (score > 0).then_some((f.id.clone(), score))
            })
            .collect();
        keywords.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        keywords.truncate(self.candidates);
        let mut lists = vec![
            keywords
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
        ];
        if let Some(space) = self.embedder.space() {
            if let Some(stored) = self.store.embedding_space(&request.group_id).await? {
                ensure!(
                    stored.is_empty() || stored == space,
                    "embedding model differs from stored group; use original model or a new group"
                );
            }
            if let Some(vectors) = self
                .embedder
                .embed(std::slice::from_ref(&request.query))
                .await?
            {
                let vector = &vectors[0];
                let mut semantic = vec![];
                for fact in candidates.values() {
                    if let Some(v) = &fact.embedding {
                        let score = cosine(vector, v)?;
                        if score > 0.0 {
                            semantic.push((fact.id.clone(), score));
                        }
                    }
                }
                semantic.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                semantic.truncate(self.candidates);
                lists.push(semantic.into_iter().map(|(id, _)| id).collect());
            }
        }
        // One-hop relationship expansion within the bounded fetched graph.
        let direct = rrf(&lists);
        let direct_order: HashMap<_, _> = direct
            .iter()
            .enumerate()
            .map(|(rank, (id, _))| (id.as_str(), rank))
            .collect();
        let seeds: HashSet<_> = direct
            .iter()
            .take(5)
            .filter_map(|(id, _)| candidates.get(id))
            .flat_map(|f| [f.source_id.clone(), f.target_id.clone()])
            .collect();
        let mut neighbors: Vec<_> = candidates
            .values()
            .filter(|f| seeds.contains(&f.source_id) || seeds.contains(&f.target_id))
            .map(|f| f.id.clone())
            .collect();
        // Retain the relevance order before applying the expansion bound. A
        // common hub must not promote an unrelated low-UUID neighbor over a seed.
        neighbors.sort_by(|a, b| {
            direct_order
                .get(a.as_str())
                .unwrap_or(&usize::MAX)
                .cmp(direct_order.get(b.as_str()).unwrap_or(&usize::MAX))
                .then(a.cmp(b))
        });
        neighbors.truncate(self.candidates);
        lists.push(neighbors);
        Ok(rrf(&lists)
            .into_iter()
            .take(request.limit)
            .filter_map(|(id, score)| candidates.remove(&id).map(|fact| FactHit { score, fact }))
            .collect())
    }
    pub async fn entities(&self, request: SearchRequest) -> Result<Vec<Entity>> {
        request.validate()?;
        let _permit = self
            .admission
            .try_acquire()
            .map_err(|_| anyhow::anyhow!("search busy; retry later"))?;
        self.store
            .entities(&request.group_id, &request.query, request.limit)
            .await
    }
}
