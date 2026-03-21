use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub default: DefaultConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub cli: CliConfig,
    #[serde(default)]
    pub provider: HashMap<String, ProviderEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultConfig {
    pub provider: String,
    pub model: String,
}

impl Default for DefaultConfig {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            model: "gpt-4o".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default)]
    pub tls: bool,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
}

fn default_true() -> bool {
    true
}
fn default_bind() -> String {
    "127.0.0.1:29999".into()
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: default_bind(),
            tls: false,
            tls_cert: None,
            tls_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsConfig {
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_agents: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default = "default_max_children")]
    pub max_children_per_parent: u32,
    #[serde(default = "default_timeout")]
    pub agent_timeout_sec: u64,
}

fn default_max_concurrent() -> u32 {
    8
}
fn default_max_depth() -> u32 {
    2
}
fn default_max_children() -> u32 {
    5
}
fn default_timeout() -> u64 {
    600
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_concurrent_agents: default_max_concurrent(),
            max_depth: default_max_depth(),
            max_children_per_parent: default_max_children(),
            agent_timeout_sec: default_timeout(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CliConfig {
    #[serde(default)]
    pub json_output: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    #[serde(rename = "type")]
    pub provider_type: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_provider_timeout")]
    pub timeout: u32,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default)]
    pub region: Option<String>,
}

fn default_provider_timeout() -> u32 {
    300
}
fn default_max_retries() -> u32 {
    3
}

/// Resolve `<env:VAR_NAME>` patterns to actual env var values.
fn resolve_env_value(val: &str) -> String {
    if let Some(var_name) = val.strip_prefix("<env:").and_then(|s| s.strip_suffix('>')) {
        std::env::var(var_name).unwrap_or_default()
    } else {
        val.to_string()
    }
}

impl ProviderEntry {
    /// Resolve env references in api_key.
    pub fn resolved_api_key(&self) -> Option<String> {
        self.api_key.as_deref().map(resolve_env_value)
    }

    pub fn base_url(&self) -> Option<String> {
        self.url.clone()
    }
}

/// Returns the MCP home directory (~/.mcp).
pub fn mcp_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mcp")
}

/// Load config by merging global (~/.mcp/config.toml) and local (./.mcp/config.toml).
pub fn load_config() -> Result<McpConfig> {
    let global_path = mcp_home().join("config.toml");
    let local_path = PathBuf::from(".mcp").join("config.toml");

    let mut config = McpConfig::default();

    if global_path.exists() {
        let text =
            std::fs::read_to_string(&global_path).context("Failed to read global config")?;
        config = toml::from_str(&text).context("Failed to parse global config")?;
    }

    if local_path.exists() {
        let text = std::fs::read_to_string(&local_path).context("Failed to read local config")?;
        let local: McpConfig = toml::from_str(&text).context("Failed to parse local config")?;
        merge_config(&mut config, local);
    }

    Ok(config)
}

fn merge_config(base: &mut McpConfig, overlay: McpConfig) {
    // Overlay providers
    for (k, v) in overlay.provider {
        base.provider.insert(k, v);
    }
    // If overlay specifies non-default values, override
    if overlay.default.provider != "openai" || overlay.default.model != "gpt-4o" {
        base.default = overlay.default;
    }
    if overlay.server.bind != default_bind() {
        base.server.bind = overlay.server.bind;
    }
    if !overlay.server.enabled {
        base.server.enabled = false;
    }
}

/// Save a config to the global config file (~/.mcp/config.toml).
pub fn save_config(config: &McpConfig) -> Result<()> {
    let path = mcp_home().join("config.toml");
    std::fs::create_dir_all(mcp_home())?;
    let text = toml::to_string_pretty(config)?;
    std::fs::write(&path, text).context("Failed to write config")?;
    Ok(())
}

/// Ensure the MCP home directory and subdirectories exist.
pub fn ensure_dirs() -> Result<()> {
    let home = mcp_home();
    for sub in &["roles", "logs", "tools", "workflows"] {
        std::fs::create_dir_all(home.join(sub))?;
    }
    Ok(())
}
