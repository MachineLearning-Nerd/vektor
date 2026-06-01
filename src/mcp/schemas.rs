use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "index_codebase",
            "Build or refresh the local codebase index.",
            index_codebase_schema(),
        )
        .with_title("Index codebase"),
        Tool::new(
            "search_code",
            "Search indexed code with hybrid semantic and keyword ranking.",
            search_code_schema(),
        )
        .with_title("Search code"),
        Tool::new(
            "get_context_for_prompt",
            "Assemble token-budgeted context for a prompt.",
            get_context_for_prompt_schema(),
        )
        .with_title("Get context for prompt"),
    ]
}

pub fn get_tool(name: &str) -> Option<Tool> {
    tools().into_iter().find(|tool| tool.name == name)
}

fn index_codebase_schema() -> JsonObject {
    object_schema(
        [
            ("path", json!({ "type": "string" })),
            ("force_full", json!({ "type": "boolean", "default": false })),
            (
                "extensions",
                json!({ "type": "array", "items": { "type": "string" } }),
            ),
            ("embedding_backend", json!({ "type": "string" })),
        ],
        ["path"],
    )
}

fn search_code_schema() -> JsonObject {
    object_schema(
        [
            ("query", json!({ "type": "string" })),
            ("path", json!({ "type": "string" })),
            (
                "top_k",
                json!({ "type": "integer", "minimum": 1, "default": 8 }),
            ),
            (
                "mode",
                json!({
                    "type": "string",
                    "enum": ["hybrid", "semantic", "keyword"],
                    "default": "hybrid"
                }),
            ),
            (
                "filter_ext",
                json!({ "type": "array", "items": { "type": "string" } }),
            ),
            (
                "bypass_cache",
                json!({ "type": "boolean", "default": false }),
            ),
        ],
        ["query", "path"],
    )
}

fn get_context_for_prompt_schema() -> JsonObject {
    object_schema(
        [
            ("query", json!({ "type": "string" })),
            ("path", json!({ "type": "string" })),
            (
                "token_budget",
                json!({ "type": "integer", "minimum": 1, "default": 8000 }),
            ),
            (
                "max_files",
                json!({ "type": "integer", "minimum": 1, "default": 10 }),
            ),
            (
                "include_related",
                json!({ "type": "boolean", "default": true }),
            ),
            (
                "min_relevance",
                json!({ "type": "number", "minimum": 0.0, "maximum": 1.0, "default": 0.5 }),
            ),
            (
                "include_docs",
                json!({ "type": "boolean", "default": true }),
            ),
            (
                "bypass_cache",
                json!({ "type": "boolean", "default": false }),
            ),
            ("scope", json!({ "type": "string" })),
        ],
        ["query", "path"],
    )
}

fn object_schema(
    properties: impl IntoIterator<Item = (&'static str, Value)>,
    required: impl IntoIterator<Item = &'static str>,
) -> JsonObject {
    let mut schema = JsonObject::new();
    let mut property_map = JsonObject::new();

    for (name, value) in properties {
        property_map.insert(name.into(), value);
    }

    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(property_map));
    schema.insert(
        "required".into(),
        Value::Array(required.into_iter().map(|name| json!(name)).collect()),
    );
    schema.insert("additionalProperties".into(), json!(false));
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_list_has_expected_names() {
        let names = tools()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            ["index_codebase", "search_code", "get_context_for_prompt"]
        );
    }

    #[test]
    fn schemas_are_input_only_at_v0_1() {
        for tool in tools() {
            assert_eq!(tool.input_schema["type"], "object");
            assert_eq!(tool.input_schema["additionalProperties"], false);
            assert!(tool.output_schema.is_none());
        }
    }

    #[test]
    fn schemas_have_expected_required_fields() {
        assert_eq!(required_fields("index_codebase"), expected(["path"]));
        assert_eq!(required_fields("search_code"), expected(["query", "path"]));
        assert_eq!(
            required_fields("get_context_for_prompt"),
            expected(["query", "path"])
        );
    }

    #[test]
    fn index_codebase_schema_matches_prd_request_shape() {
        let tool = get_tool("index_codebase").expect("tool exists");
        let properties = tool.input_schema["properties"]
            .as_object()
            .expect("properties object");

        for field in ["path", "force_full", "extensions", "embedding_backend"] {
            assert!(properties.contains_key(field), "missing {field}");
        }
        assert!(
            !properties.contains_key("force"),
            "force is a CLI concept; MCP advertises force_full"
        );
        assert_eq!(properties["force_full"]["default"], false);
        assert_eq!(properties["extensions"]["type"], "array");
        assert_eq!(properties["extensions"]["items"]["type"], "string");
    }

    #[test]
    fn search_code_schema_matches_prd_request_shape() {
        let tool = get_tool("search_code").expect("tool exists");
        let properties = tool.input_schema["properties"]
            .as_object()
            .expect("properties object");

        for field in [
            "path",
            "query",
            "top_k",
            "mode",
            "filter_ext",
            "bypass_cache",
        ] {
            assert!(properties.contains_key(field), "missing {field}");
        }
        assert_eq!(properties["top_k"]["minimum"], 1);
        assert_eq!(properties["top_k"]["default"], 8);
        assert_eq!(
            properties["mode"]["enum"],
            json!(["hybrid", "semantic", "keyword"])
        );
        assert_eq!(properties["mode"]["default"], "hybrid");
        assert_eq!(properties["filter_ext"]["type"], "array");
        assert_eq!(properties["filter_ext"]["items"]["type"], "string");
        assert_eq!(properties["bypass_cache"]["default"], false);
    }

    #[test]
    fn get_context_for_prompt_schema_matches_prd_request_shape() {
        let tool = get_tool("get_context_for_prompt").expect("tool exists");
        let properties = tool.input_schema["properties"]
            .as_object()
            .expect("properties object");

        for field in [
            "path",
            "query",
            "token_budget",
            "max_files",
            "include_related",
            "min_relevance",
            "include_docs",
            "bypass_cache",
            "scope",
        ] {
            assert!(properties.contains_key(field), "missing {field}");
        }
        assert_eq!(properties["token_budget"]["minimum"], 1);
        assert_eq!(properties["token_budget"]["default"], 8000);
        assert_eq!(properties["max_files"]["minimum"], 1);
        assert_eq!(properties["max_files"]["default"], 10);
        assert_eq!(properties["include_related"]["default"], true);
        assert_eq!(properties["min_relevance"]["minimum"], 0.0);
        assert_eq!(properties["min_relevance"]["maximum"], 1.0);
        assert_eq!(properties["min_relevance"]["default"], 0.5);
        assert_eq!(properties["include_docs"]["default"], true);
        assert_eq!(properties["bypass_cache"]["default"], false);
        assert_eq!(properties["scope"]["type"], "string");
    }

    fn required_fields(tool_name: &str) -> Vec<String> {
        let tool = get_tool(tool_name).expect("tool exists");
        tool.input_schema["required"]
            .as_array()
            .expect("required array")
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect()
    }

    fn expected<const N: usize>(fields: [&str; N]) -> Vec<String> {
        fields.into_iter().map(str::to_owned).collect()
    }
}
