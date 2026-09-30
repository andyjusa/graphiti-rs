use crate::{
    domain::{AddEpisode, EpisodeKey, SearchRequest},
    ingest::Memory,
};
use rmcp::{
    ErrorData, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};
use std::sync::Arc;
#[derive(Clone)]
pub struct MemoryService {
    pub memory: Arc<Memory>,
    tool_router: ToolRouter<Self>,
}
impl MemoryService {
    pub fn new(memory: Arc<Memory>) -> Self {
        Self {
            memory,
            tool_router: Self::tool_router(),
        }
    }
}
fn result<T: serde::Serialize>(value: anyhow::Result<T>) -> Result<CallToolResult, ErrorData> {
    match value {
        Ok(value) => Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string(&value)
                .map_err(|_| ErrorData::internal_error("serialization failed", None))?,
        )])),
        Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(
            error.to_string(),
        )])),
    }
}
#[tool_router]
impl MemoryService {
    #[tool(
        description = "Store an episode and its temporal graph. Supply extraction to avoid an LLM call. Reuse episode_id for idempotent retries."
    )]
    async fn add_episode(
        &self,
        Parameters(request): Parameters<AddEpisode>,
    ) -> Result<CallToolResult, ErrorData> {
        result(self.memory.add(request).await)
    }
    #[tool(
        description = "Search current or historical facts using keywords, optional semantic ranking, and bounded graph expansion. Returns provenance."
    )]
    async fn search_facts(
        &self,
        Parameters(request): Parameters<SearchRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        result(self.memory.search.facts(request).await)
    }
    #[tool(
        description = "Search entities by normalized name within one group. as_of is not applied to entity summaries."
    )]
    async fn search_nodes(
        &self,
        Parameters(request): Parameters<SearchRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        result(self.memory.search.entities(request).await)
    }
    #[tool(description = "Retrieve an original episode by group and episode ID.")]
    async fn get_episode(
        &self,
        Parameters(request): Parameters<EpisodeKey>,
    ) -> Result<CallToolResult, ErrorData> {
        result(
            async {
                request.validate()?;
                self.memory
                    .store
                    .episode(&request.group_id, &request.episode_id)
                    .await
            }
            .await,
        )
    }
    #[tool(
        description = "Delete an episode and its provenance. Facts shared with other episodes survive; temporal invalidation is not undone."
    )]
    async fn delete_episode(
        &self,
        Parameters(request): Parameters<EpisodeKey>,
    ) -> Result<CallToolResult, ErrorData> {
        result(
            async {
                request.validate()?;
                self.memory
                    .store
                    .delete_episode(&request.group_id, &request.episode_id)
                    .await
            }
            .await,
        )
    }
    #[tool(
        description = "Check server availability, FalkorDB connectivity, and the configured embedding provider."
    )]
    async fn get_status(&self) -> Result<CallToolResult, ErrorData> {
        let connected = self.memory.store.ping().await.is_ok();
        result(Ok(
            serde_json::json!({"status":if connected{"ok"}else{"degraded"},"database_connected":connected,"embedding_space":self.memory.embedder.space(),"version":env!("CARGO_PKG_VERSION")}),
        ))
    }
}
#[tool_handler(router = self.tool_router)]
impl ServerHandler for MemoryService {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info = Implementation::new("graphiti-rs", env!("CARGO_PKG_VERSION"));
        info.instructions=Some("Lightweight temporal memory. Every tool except get_status requires group_id; use a stable episode_id for retries. This is not a complete Python Graphiti API replacement.".into());
        info
    }
}
