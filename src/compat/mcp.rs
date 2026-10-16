use super::{dto::*, engine::Engine};
use rmcp::{
    ErrorData, ServerHandler,
    model::*,
    service::{RequestContext, RoleServer},
};
use serde_json::Value;
use std::sync::Arc;
#[derive(Clone)]
pub struct CompatService {
    pub engine: Arc<Engine>,
}
impl CompatService {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self { engine }
    }
}
pub fn tools() -> Vec<Tool> {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/production-mcp-contract.json"
    ))
    .expect("checked-in compatibility fixture");
    serde_json::from_value(fixture["tools"]["tools"].clone())
        .expect("checked-in compatibility schemas")
}
pub fn shape(name: &str, args: &Value) -> Result<(), ErrorData> {
    fn check<T: serde::de::DeserializeOwned>(args: &Value) -> Result<(), ErrorData> {
        serde_json::from_value::<T>(args.clone())
            .map(|_| ())
            .map_err(|_| ErrorData::invalid_params("arguments do not match the tool schema", None))
    }
    match name {
        "add_memory" => check::<AddMemory>(args),
        "get_episode" | "get_entity_edge" | "delete_episode" | "delete_entity_edge" => {
            check::<Key>(args)
        }
        "search_nodes" => check::<NodeSearch>(args),
        "search_memory_facts" => check::<FactSearch>(args),
        "get_episodes" => check::<Episodes>(args),
        "clear_graph" => check::<Clear>(args),
        "get_status" => Ok(()),
        _ => Err(ErrorData::invalid_params("unknown tool", None)),
    }
}
pub fn response(name: &str, value: anyhow::Result<Value>) -> Result<CallToolResult, ErrorData> {
    let value = value.unwrap_or_else(|e| serde_json::json!({"error":e.to_string()}));
    let text = if name == "get_status" {
        format!(
            "{{\n  \"status\": {},\n  \"message\": {}\n}}",
            value["status"], value["message"]
        )
    } else {
        serde_json::to_string_pretty(&value)
            .map_err(|_| ErrorData::internal_error("serialization failed", None))?
    };
    let mut r = CallToolResult::success(vec![ContentBlock::text(text)]);
    r.structured_content = Some(if name == "get_status" {
        value
    } else {
        serde_json::json!({"result":value})
    });
    r.is_error = Some(false);
    Ok(r)
}
impl ServerHandler for CompatService {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        let operational = self.engine.write_mode() == super::config::WriteMode::Operational;
        info.server_info = Implementation::new(
            if operational {
                "graphiti-rs"
            } else {
                "graphiti-rs-compat-preview"
            },
            env!("CARGO_PKG_VERSION"),
        );
        info.instructions = Some(if operational {
            "Graphiti memory server. add_memory acceptance is not graph completion; confirm completion using get_episode with its exact UUID and group. Scoped read and write permissions apply."
        } else {
            "Graphiti 0.28.2 contract preview. Operational writes are disabled. add_memory acceptance is not graph completion; use exact get_episode."
        }.into());
        info
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|t| t.name == name)
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools()))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = Value::Object(request.arguments.unwrap_or_default());
        shape(&request.name, &args)?;
        Ok(response(
            &request.name,
            self.engine.execute(&request.name, args).await,
        )?
        .into())
    }
}
