use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::config::mcp_home;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleDefinition {
    pub name: String,
    #[serde(default)]
    pub soul: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub prompt_file: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default = "default_depth")]
    pub max_depth: u32,
    #[serde(default = "default_children")]
    pub max_children: u32,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

fn default_depth() -> u32 {
    2
}
fn default_children() -> u32 {
    5
}

fn roles_dir() -> PathBuf {
    mcp_home().join("roles")
}

pub fn list_roles() -> Result<Vec<RoleDefinition>> {
    let dir = roles_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut roles = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml") {
            let text = std::fs::read_to_string(&path)?;
            if let Ok(role) = toml::from_str::<RoleDefinition>(&text) {
                roles.push(role);
            }
        }
    }
    Ok(roles)
}

pub fn get_role(name: &str) -> Result<RoleDefinition> {
    let path = roles_dir().join(format!("{name}.toml"));
    if !path.exists() {
        bail!("Role '{name}' not found");
    }
    let text = std::fs::read_to_string(&path)?;
    toml::from_str(&text).context("Failed to parse role file")
}

pub fn create_role(role: &RoleDefinition) -> Result<()> {
    let path = roles_dir().join(format!("{}.toml", role.name));
    let text = toml::to_string_pretty(role)?;
    std::fs::write(&path, text)?;
    Ok(())
}

pub fn delete_role(name: &str) -> Result<()> {
    let path = roles_dir().join(format!("{name}.toml"));
    if !path.exists() {
        bail!("Role '{name}' not found");
    }
    std::fs::remove_file(&path)?;
    Ok(())
}

pub fn patch_role(name: &str, prompt_patch: Option<&str>, model: Option<&str>, provider: Option<&str>) -> Result<RoleDefinition> {
    let mut role = get_role(name)?;
    if let Some(patch) = prompt_patch {
        let current = role.system_prompt.unwrap_or_default();
        role.system_prompt = Some(format!("{current}\n{patch}"));
    }
    if let Some(m) = model {
        role.default_model = Some(m.to_string());
    }
    if let Some(p) = provider {
        role.default_provider = Some(p.to_string());
    }
    create_role(&role)?;
    Ok(role)
}

/// Load the system prompt for a role, resolving prompt_file if needed.
pub fn resolve_system_prompt(role: &RoleDefinition) -> Result<String> {
    if let Some(ref prompt) = role.system_prompt {
        return Ok(prompt.clone());
    }
    if let Some(ref file) = role.prompt_file {
        let path = roles_dir().join(file);
        if path.exists() {
            return std::fs::read_to_string(&path).context("Failed to read soul file");
        }
        // Try relative to CWD
        let cwd_path = PathBuf::from(file);
        if cwd_path.exists() {
            return std::fs::read_to_string(&cwd_path).context("Failed to read soul file");
        }
        bail!("Prompt file '{file}' not found");
    }
    Ok(String::new())
}
