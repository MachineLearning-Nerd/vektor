use std::future;

use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo,
    },
    service::{RequestContext, RoleServer},
    transport::stdio,
};

use crate::{error::VektorError, mcp::handlers};

pub async fn start_stdio_server() -> crate::error::Result<()> {
    let service = VektorServer
        .serve(stdio())
        .await
        .map_err(|error| VektorError::Mcp(error.to_string()))?;

    service
        .waiting()
        .await
        .map_err(|error| VektorError::Mcp(error.to_string()))?;

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct VektorServer;

impl ServerHandler for VektorServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::default())
            .with_server_info(Implementation::new("vektor", env!("CARGO_PKG_VERSION")))
            .with_instructions("Vektor v0.1.0 exposes no-op skeleton tools only.")
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        future::ready(Ok(ListToolsResult::with_all_items(
            crate::mcp::schemas::tools(),
        )))
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        crate::mcp::schemas::get_tool(name)
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, ErrorData>> + Send + '_ {
        let result = match request.name.as_ref() {
            "index_codebase" => Ok(handlers::handle_index_codebase(request.arguments)),
            "search_code" => Ok(handlers::handle_search_code(request.arguments)),
            "get_context_for_prompt" => {
                Ok(handlers::handle_get_context_for_prompt(request.arguments))
            }
            other => Err(ErrorData::invalid_params(
                format!("unknown tool: {other}"),
                None,
            )),
        };

        future::ready(result.map(CallToolResult::structured))
    }
}
