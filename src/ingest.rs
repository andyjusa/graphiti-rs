use crate::{
    config::Config, domain::*, embeddings::Embedder, models::ModelApi, search::Search, store::Store,
};
use anyhow::Result;
use std::sync::Arc;
#[derive(Clone)]
pub struct Memory {
    pub store: Store,
    pub embedder: Embedder,
    pub search: Search,
    api: ModelApi,
    jobs: Arc<tokio::sync::Semaphore>,
}
impl Memory {
    pub async fn new(config: &Config) -> Result<Self> {
        let store = Store::connect(&config.redis_url, &config.graph).await?;
        let embedder = Embedder::new(config).await?;
        Ok(Self {
            search: Search {
                store: store.clone(),
                embedder: embedder.clone(),
                candidates: config.candidates,
                admission: Arc::new(tokio::sync::Semaphore::new(config.search_jobs)),
            },
            store,
            embedder,
            api: ModelApi::new(config)?,
            jobs: Arc::new(tokio::sync::Semaphore::new(config.jobs)),
        })
    }
    pub async fn add(&self, request: AddEpisode) -> Result<IngestResult> {
        request.validate()?;
        let _permit = self
            .jobs
            .try_acquire()
            .map_err(|_| anyhow::anyhow!("ingestion busy; retry later"))?;
        let id = request
            .episode_id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let episode = Episode {
            id,
            group_id: request.group_id,
            content: request.content,
            source: request.source,
            reference_time: request.reference_time,
            created_at: chrono::Utc::now(),
        };
        if let Some(result) = self.store.retry_result(&episode).await? {
            return Ok(result);
        }
        let extraction = match request.extraction {
            Some(x) => x,
            None => {
                let existing = self
                    .store
                    .facts(
                        &episode.group_id,
                        episode.reference_time.timestamp_millis(),
                        30,
                    )
                    .await?;
                self.api.extract(&episode, &existing).await?
            }
        };
        let texts: Vec<_> = extraction.facts.iter().map(|f| f.fact.clone()).collect();
        let vectors = self.embedder.embed(&texts).await?;
        self.store
            .ingest(
                &episode,
                &extraction,
                vectors.as_deref(),
                self.embedder.space().as_deref(),
            )
            .await
    }
}
