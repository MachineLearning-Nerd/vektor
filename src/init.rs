use std::{
    fs,
    path::{Path, PathBuf},
};

use clap::ValueEnum;
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use toml::{Table as TomlTable, Value as TomlValue};

use crate::error::{Result, VektorError};

#[derive(clap::Args, Debug, Clone)]
pub(crate) struct InitArgs {
    /// Agent config to update. Defaults to all supported agents.
    #[arg(long, value_enum)]
    pub(crate) agent: Option<InitAgent>,

    /// Replace an existing `vektor` MCP entry.
    #[arg(long)]
    pub(crate) force: bool,

    /// Print planned changes without writing config files.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum InitAgent {
    Claude,
    Cursor,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InitReport {
    pub(crate) lines: Vec<String>,
}

impl InitReport {
    pub(crate) fn as_text(&self) -> String {
        self.lines.join("\n")
    }
}

pub(crate) fn run(args: InitArgs) -> Result<InitReport> {
    let home = dirs::home_dir()
        .ok_or_else(|| VektorError::Config("could not resolve home directory".to_string()))?;
    run_with_home(args, home)
}

pub(crate) fn run_with_home(args: InitArgs, home: PathBuf) -> Result<InitReport> {
    let changes = plan_changes(&args, &home)?;
    let mut lines = Vec::with_capacity(changes.len());

    for change in changes {
        if args.dry_run {
            lines.push(format!(
                "would update {} config at {}",
                change.agent.display_name(),
                change.path.display()
            ));
            continue;
        }

        if let Some(parent) = change.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&change.path, change.content)?;
        lines.push(format!(
            "updated {} config at {}",
            change.agent.display_name(),
            change.path.display()
        ));
    }

    Ok(InitReport { lines })
}

#[derive(Debug)]
struct PlannedChange {
    agent: InitAgent,
    path: PathBuf,
    content: String,
}

fn plan_changes(args: &InitArgs, home: &Path) -> Result<Vec<PlannedChange>> {
    selected_agents(args)
        .into_iter()
        .map(|agent| {
            let path = agent.config_path(home);
            let content = match agent {
                InitAgent::Claude | InitAgent::Cursor => json_config_content(&path, args.force)?,
                InitAgent::Codex => codex_config_content(&path, args.force)?,
            };
            Ok(PlannedChange {
                agent,
                path,
                content,
            })
        })
        .collect()
}

fn selected_agents(args: &InitArgs) -> Vec<InitAgent> {
    match args.agent {
        Some(agent) => vec![agent],
        None => vec![InitAgent::Claude, InitAgent::Cursor, InitAgent::Codex],
    }
}

impl InitAgent {
    fn config_path(self, home: &Path) -> PathBuf {
        match self {
            Self::Claude => home.join(".claude.json"),
            Self::Cursor => home.join(".cursor/mcp.json"),
            Self::Codex => home.join(".codex/config.toml"),
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Cursor => "Cursor",
            Self::Codex => "Codex CLI",
        }
    }
}

fn json_config_content(path: &Path, force: bool) -> Result<String> {
    let mut root = read_json_config(path)?;
    let root_object = root.as_object_mut().ok_or_else(|| {
        VektorError::Config(format!("{} must contain a JSON object", path.display()))
    })?;

    let servers = root_object
        .entry("mcpServers")
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    let servers_object = servers.as_object_mut().ok_or_else(|| {
        VektorError::Config(format!(
            "{} field `mcpServers` must contain a JSON object",
            path.display()
        ))
    })?;

    if servers_object.contains_key("vektor") && !force {
        return Err(existing_entry_error(path));
    }
    servers_object.insert("vektor".to_string(), mcp_server_json());

    serde_json::to_string_pretty(&root)
        .map(|mut content| {
            content.push('\n');
            content
        })
        .map_err(|error| VektorError::Config(format!("failed to render JSON config: {error}")))
}

fn read_json_config(path: &Path) -> Result<JsonValue> {
    if !path.exists() {
        return Ok(JsonValue::Object(JsonMap::new()));
    }
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|error| {
        VektorError::Config(format!("failed to parse {}: {error}", path.display()))
    })
}

fn mcp_server_json() -> JsonValue {
    json!({
        "command": "vektor",
        "args": ["serve", "--transport", "stdio"]
    })
}

fn codex_config_content(path: &Path, force: bool) -> Result<String> {
    let mut root_table = read_toml_config(path)?;
    let servers = root_table
        .entry("mcp_servers".to_string())
        .or_insert_with(|| TomlValue::Table(TomlTable::new()));
    let servers_table = servers.as_table_mut().ok_or_else(|| {
        VektorError::Config(format!(
            "{} table `mcp_servers` must contain subtables",
            path.display()
        ))
    })?;

    if servers_table.contains_key("vektor") && !force {
        return Err(existing_entry_error(path));
    }
    servers_table.insert("vektor".to_string(), codex_server_toml());

    toml::to_string_pretty(&root_table)
        .map(|mut content| {
            content.push('\n');
            content
        })
        .map_err(|error| VektorError::Config(format!("failed to render TOML config: {error}")))
}

fn read_toml_config(path: &Path) -> Result<TomlTable> {
    if !path.exists() {
        return Ok(TomlTable::new());
    }
    let content = fs::read_to_string(path)?;
    content.parse::<TomlTable>().map_err(|error| {
        VektorError::Config(format!("failed to parse {}: {error}", path.display()))
    })
}

fn codex_server_toml() -> TomlValue {
    let mut table = TomlTable::new();
    table.insert(
        "command".to_string(),
        TomlValue::String("vektor".to_string()),
    );
    table.insert(
        "args".to_string(),
        TomlValue::Array(
            ["serve", "--transport", "stdio"]
                .into_iter()
                .map(|value| TomlValue::String(value.to_string()))
                .collect(),
        ),
    );
    TomlValue::Table(table)
}

fn existing_entry_error(path: &Path) -> VektorError {
    VektorError::Config(format!(
        "{} already has an MCP server named `vektor`; rerun with --force to replace it",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_agents_args() -> InitArgs {
        InitArgs {
            agent: None,
            force: false,
            dry_run: false,
        }
    }

    fn args_for(agent: InitAgent) -> InitArgs {
        InitArgs {
            agent: Some(agent),
            force: false,
            dry_run: false,
        }
    }

    #[test]
    fn init_mcp_config_writes_all_supported_agent_entries() {
        let home = tempfile::tempdir().expect("home");

        run_with_home(all_agents_args(), home.path().to_path_buf()).expect("init all agents");

        let claude = std::fs::read_to_string(home.path().join(".claude.json")).expect("claude");
        assert!(claude.contains("\"mcpServers\""));
        assert!(claude.contains("\"vektor\""));
        assert!(claude.contains("\"command\": \"vektor\""));
        assert!(claude.contains("\"serve\""));

        let cursor = std::fs::read_to_string(home.path().join(".cursor/mcp.json")).expect("cursor");
        assert!(cursor.contains("\"mcpServers\""));
        assert!(cursor.contains("\"vektor\""));

        let codex = std::fs::read_to_string(home.path().join(".codex/config.toml")).expect("codex");
        let codex = codex.parse::<TomlTable>().expect("parse codex toml");
        let server = codex
            .get("mcp_servers")
            .and_then(TomlValue::as_table)
            .and_then(|servers| servers.get("vektor"))
            .and_then(TomlValue::as_table)
            .expect("vektor server");
        assert_eq!(
            server.get("command").and_then(TomlValue::as_str),
            Some("vektor")
        );
        let args = server
            .get("args")
            .and_then(TomlValue::as_array)
            .expect("args");
        assert_eq!(
            args.iter()
                .filter_map(TomlValue::as_str)
                .collect::<Vec<_>>(),
            ["serve", "--transport", "stdio"]
        );
    }

    #[test]
    fn init_mcp_config_dry_run_reports_without_writing() {
        let home = tempfile::tempdir().expect("home");
        let args = InitArgs {
            dry_run: true,
            ..args_for(InitAgent::Claude)
        };

        let report = run_with_home(args, home.path().to_path_buf()).expect("dry run");

        assert!(report.as_text().contains("would update Claude Code"));
        assert!(!home.path().join(".claude.json").exists());
    }

    #[test]
    fn init_mcp_config_preserves_unrelated_entries() {
        let home = tempfile::tempdir().expect("home");
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"mcpServers":{"other":{"command":"other","args":["serve"]}},"theme":"dark"}"#,
        )
        .expect("seed claude");

        run_with_home(args_for(InitAgent::Claude), home.path().to_path_buf()).expect("init claude");

        let claude = std::fs::read_to_string(home.path().join(".claude.json")).expect("claude");
        assert!(claude.contains("\"other\""));
        assert!(claude.contains("\"theme\": \"dark\""));
        assert!(claude.contains("\"vektor\""));
    }

    #[test]
    fn init_mcp_config_refuses_existing_vektor_without_force() {
        let home = tempfile::tempdir().expect("home");
        std::fs::create_dir_all(home.path().join(".cursor")).expect("mkdir cursor");
        std::fs::write(
            home.path().join(".cursor/mcp.json"),
            r#"{"mcpServers":{"vektor":{"command":"old-vektor"}}}"#,
        )
        .expect("seed cursor");

        let error = run_with_home(args_for(InitAgent::Cursor), home.path().to_path_buf())
            .expect_err("existing vektor should be protected");

        assert!(error.to_string().contains("--force"));
        let cursor = std::fs::read_to_string(home.path().join(".cursor/mcp.json")).expect("cursor");
        assert!(cursor.contains("old-vektor"));
    }

    #[test]
    fn init_mcp_config_force_replaces_existing_vektor() {
        let home = tempfile::tempdir().expect("home");
        std::fs::create_dir_all(home.path().join(".codex")).expect("mkdir codex");
        std::fs::write(
            home.path().join(".codex/config.toml"),
            "[mcp_servers.vektor]\ncommand = \"old-vektor\"\n",
        )
        .expect("seed codex");
        let args = InitArgs {
            force: true,
            ..args_for(InitAgent::Codex)
        };

        run_with_home(args, home.path().to_path_buf()).expect("force codex");

        let codex = std::fs::read_to_string(home.path().join(".codex/config.toml")).expect("codex");
        assert!(codex.contains("command = \"vektor\""));
        assert!(!codex.contains("old-vektor"));
    }
}
