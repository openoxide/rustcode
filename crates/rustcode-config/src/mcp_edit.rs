use std::fs;
use std::path::{Path, PathBuf};

use rustcode_core::config::McpServerConfig;
use rustcode_core::error::ConfigError;
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::paths::{resolve_project_config_path, resolve_user_config_path};
use crate::ConfigSources;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigEditScope {
    User,
    Project,
}

pub fn edit_mcp_server(
    scope: ConfigEditScope,
    cwd: &Path,
    name: &str,
    server: &McpServerConfig,
) -> Result<PathBuf, ConfigError> {
    if name.trim().is_empty() {
        return Err(ConfigError::Validation(
            "mcp server name must not be empty".to_string(),
        ));
    }

    let sources = ConfigSources::new(cwd.to_path_buf());
    let path = match scope {
        ConfigEditScope::User => resolve_user_config_path(&sources),
        ConfigEditScope::Project => resolve_project_config_path(&sources),
    };

    let mut doc = read_or_init_document(&path)?;
    let mcp = doc
        .entry("mcp")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| ConfigError::Parse(format!("{}: mcp must be a table", path.display())))?;
    let servers = mcp
        .entry("servers")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            ConfigError::Parse(format!("{}: mcp.servers must be a table", path.display()))
        })?;

    let server_item = servers.entry(name).or_insert(Item::Table(Table::new()));
    let server_table = server_item.as_table_mut().ok_or_else(|| {
        ConfigError::Parse(format!(
            "{}: mcp.servers.{name} must be a table",
            path.display()
        ))
    })?;

    if let Some(url) = server.url.as_ref() {
        server_table.insert("url", Item::Value(Value::from(url.clone())));
    } else {
        server_table.remove("url");
    }

    if let Some(command) = server.command.as_ref() {
        server_table.insert("command", Item::Value(Value::from(command.clone())));
    } else {
        server_table.remove("command");
    }

    if server.args.is_empty() {
        server_table.remove("args");
    } else {
        let mut args_array = toml_edit::Array::default();
        for arg in &server.args {
            args_array.push(arg.as_str());
        }
        server_table.insert("args", Item::Value(Value::Array(args_array)));
    }

    if server.env.is_empty() {
        server_table.remove("env");
    } else {
        let mut env_table = toml_edit::InlineTable::new();
        for (key, value) in &server.env {
            env_table.get_or_insert(key.as_str(), value.clone());
        }
        server_table.insert("env", Item::Value(Value::InlineTable(env_table)));
    }

    let oauth = &server.oauth;
    let has_settings = oauth.client_id.is_some() || oauth.client_secret_env.is_some();
    if has_settings {
        let mut inline = toml_edit::InlineTable::new();
        inline.get_or_insert("enabled", oauth.enabled);
        if let Some(client_id) = oauth.client_id.as_ref() {
            inline.get_or_insert("client_id", client_id.clone());
        }
        if let Some(secret_env) = oauth.client_secret_env.as_ref() {
            inline.get_or_insert("client_secret_env", secret_env.clone());
        }
        server_table.insert("oauth", Item::Value(Value::InlineTable(inline)));
    } else if oauth.enabled {
        server_table.remove("oauth");
    } else {
        server_table.insert("oauth", Item::Value(Value::from(false)));
    }

    write_document_atomically(&path, &doc)?;
    Ok(path)
}

pub fn remove_mcp_server(
    scope: ConfigEditScope,
    cwd: &Path,
    name: &str,
) -> Result<(PathBuf, bool), ConfigError> {
    if name.trim().is_empty() {
        return Err(ConfigError::Validation(
            "mcp server name must not be empty".to_string(),
        ));
    }

    let sources = ConfigSources::new(cwd.to_path_buf());
    let path = match scope {
        ConfigEditScope::User => resolve_user_config_path(&sources),
        ConfigEditScope::Project => resolve_project_config_path(&sources),
    };
    let mut doc = read_or_init_document(&path)?;

    let Some(mcp_item) = doc.get_mut("mcp") else {
        return Ok((path, false));
    };
    let Some(mcp) = mcp_item.as_table_mut() else {
        return Err(ConfigError::Parse(format!(
            "{}: mcp must be a table",
            path.display()
        )));
    };
    let Some(servers_item) = mcp.get_mut("servers") else {
        return Ok((path, false));
    };
    let Some(servers) = servers_item.as_table_mut() else {
        return Err(ConfigError::Parse(format!(
            "{}: mcp.servers must be a table",
            path.display()
        )));
    };
    let removed = servers.remove(name).is_some();
    if removed {
        write_document_atomically(&path, &doc)?;
    }
    Ok((path, removed))
}

fn read_or_init_document(path: &Path) -> Result<DocumentMut, ConfigError> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let raw = fs::read_to_string(path)
        .map_err(|err| ConfigError::Read(format!("{}: {err}", path.display())))?;
    raw.parse::<DocumentMut>()
        .map_err(|err| ConfigError::Parse(format!("{}: {err}", path.display())))
}

fn write_document_atomically(path: &Path, doc: &DocumentMut) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| ConfigError::Read(format!("{}: {err}", parent.display())))?;
    }
    let serialized = doc.to_string();
    let temp_path = path.with_extension("toml.tmp");
    fs::write(&temp_path, serialized)
        .map_err(|err| ConfigError::Read(format!("{}: {err}", temp_path.display())))?;
    fs::rename(&temp_path, path)
        .map_err(|err| ConfigError::Read(format!("{}: {err}", path.display())))?;
    Ok(())
}
