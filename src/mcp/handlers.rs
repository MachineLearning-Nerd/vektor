use serde_json::{Value, json};

pub fn handle_index_codebase(_args: Option<rmcp::model::JsonObject>) -> Value {
    json!({
        "status": "not implemented yet",
        "phase": "Phase 2 — Discovery + Chunking (v0.2.0)",
        "see": "docs/plans/initial/phase-2-discovery-chunking/README.md"
    })
}

pub fn handle_search_code(_args: Option<rmcp::model::JsonObject>) -> Value {
    json!({
        "status": "not implemented yet",
        "phase": "Phase 4 — Search + MCP",
        "see": "docs/plans/initial/phase-4-search-mcp/README.md"
    })
}

pub fn handle_get_context_for_prompt(_args: Option<rmcp::model::JsonObject>) -> Value {
    json!({
        "status": "not implemented yet",
        "phase": "Phase 5 — Context Assembly (v0.4.0)",
        "see": "docs/plans/initial/phase-5-context-assembly/README.md"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_handlers_return_status_and_phase() {
        for response in [
            handle_index_codebase(None),
            handle_search_code(None),
            handle_get_context_for_prompt(None),
        ] {
            assert_eq!(response["status"], "not implemented yet");
            assert!(response["phase"].is_string());
        }
    }
}
