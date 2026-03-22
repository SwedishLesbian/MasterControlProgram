use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify, RwLock};
use tracing::{info, warn};

use crate::config::{LimitsConfig, McpConfig, DefaultConfig};
use crate::persistence::Database;
use crate::provider::{self, ChatMessage, Provider};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Queued,
    Running,
    #[serde(rename = "waiting-on-user")]
    WaitingOnUser,
    Completed,
    Failed,
    Killed,
    Paused,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Running => write!(f, "running"),
            Self::WaitingOnUser => write!(f, "waiting-on-user"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Killed => write!(f, "killed"),
            Self::Paused => write!(f, "paused"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: u64,
    pub task: String,
    pub soul: Option<String>,
    pub role: Option<String>,
    pub model: String,
    pub provider: String,
    pub status: AgentStatus,
    pub phase: Option<String>,
    pub progress: f64,
    pub max_depth: u32,
    pub max_children: u32,
    pub depth: u32,
    pub parent_id: Option<u64>,
    pub children: Vec<u64>,
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub last_output: Option<String>,
    pub last_output_tokens: Option<u64>,
    pub timeout_sec: u64,
    pub created_at: DateTime<Utc>,
    pub timeout_remaining_sec: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnRequest {
    pub task: String,
    pub role: Option<String>,
    pub soul: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub depth: Option<u32>,
    pub max_children: Option<u32>,
    pub max_depth: Option<u32>,
    pub timeout_sec: Option<u64>,
    pub system_prompt: Option<String>,
    pub parent_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnResponse {
    pub id: u64,
    pub soul: Option<String>,
    pub role: Option<String>,
    pub model: String,
    pub provider: String,
    pub max_depth: u32,
    pub max_children: u32,
    pub timeout_remaining_sec: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteerRequest {
    pub instruction: Option<String>,
    pub prompt_patch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteerResponse {
    pub id: u64,
    pub action: String,
    pub instruction_appended: bool,
    pub system_prompt_patched: bool,
    pub patch_size_delta_tokens: i64,
}

struct AgentHandle {
    info: AgentInfo,
    pause_notify: Arc<Notify>,
}

pub struct AgentManager {
    agents: RwLock<HashMap<u64, Arc<Mutex<AgentHandle>>>>,
    next_id: Mutex<u64>,
    limits: LimitsConfig,
    defaults: DefaultConfig,
    providers: RwLock<HashMap<String, Arc<dyn Provider>>>,
    db: Arc<Database>,
}

impl AgentManager {
    pub fn new(config: &McpConfig, db: Arc<Database>) -> Result<Self> {
        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        for (name, entry) in &config.provider {
            match provider::build_provider(name, entry, &config.default.model) {
                Ok(client) => {
                    providers.insert(name.clone(), Arc::from(client));
                }
                Err(e) => {
                    warn!("Failed to initialize provider '{name}': {e}");
                }
            }
        }

        let next_id = db.next_agent_id()?;

        Ok(Self {
            agents: RwLock::new(HashMap::new()),
            next_id: Mutex::new(next_id),
            limits: config.limits.clone(),
            defaults: config.default.clone(),
            providers: RwLock::new(providers),
            db,
        })
    }

    pub async fn spawn(&self, req: SpawnRequest) -> Result<SpawnResponse> {
        // Enforce max concurrent agents
        {
            let agents = self.agents.read().await;
            let mut active = 0u32;
            for handle in agents.values() {
                let h = handle.lock().await;
                if matches!(
                    h.info.status,
                    AgentStatus::Running | AgentStatus::Queued | AgentStatus::Paused
                ) {
                    active += 1;
                }
            }
            if active >= self.limits.max_concurrent_agents {
                bail!(
                    "Max concurrent agents ({}) reached",
                    self.limits.max_concurrent_agents
                );
            }
        }

        // Determine provider and model
        let provider_name = req
            .provider
            .clone()
            .unwrap_or_else(|| self.defaults.provider.clone());

        let providers = self.providers.read().await;
        let provider_arc = providers
            .get(&provider_name)
            .ok_or_else(|| anyhow::anyhow!("Provider '{provider_name}' not configured"))?
            .clone();

        // Build fallback provider chain (secondary, tertiary)
        let mut fallback_providers: Vec<Arc<dyn Provider>> = Vec::new();
        if let Some(ref sec) = self.defaults.secondary_provider {
            if let Some(p) = providers.get(sec) {
                fallback_providers.push(p.clone());
            }
        }
        if let Some(ref ter) = self.defaults.tertiary_provider {
            if let Some(p) = providers.get(ter) {
                fallback_providers.push(p.clone());
            }
        }

        let model = req
            .model
            .clone()
            .unwrap_or_else(|| provider_arc.model().to_string());
        let max_depth = req.max_depth.unwrap_or(self.limits.max_depth);
        let max_children = req
            .max_children
            .unwrap_or(self.limits.max_children_per_parent);
        let timeout_sec = req.timeout_sec.unwrap_or(self.limits.agent_timeout_sec);
        let depth = req.depth.unwrap_or(0);

        // Enforce depth
        if depth >= max_depth {
            bail!("Max depth ({max_depth}) exceeded at depth {depth}");
        }

        let id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            *next += 1;
            id
        };

        let system_prompt = req.system_prompt.clone().unwrap_or_default();

        let info = AgentInfo {
            id,
            task: req.task.clone(),
            soul: req.soul.clone(),
            role: req.role.clone(),
            model: model.clone(),
            provider: provider_name.clone(),
            status: AgentStatus::Running,
            phase: Some("initializing".into()),
            progress: 0.0,
            max_depth,
            max_children,
            depth,
            parent_id: req.parent_id,
            children: vec![],
            system_prompt: system_prompt.clone(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: req.task.clone(),
            }],
            last_output: None,
            last_output_tokens: None,
            timeout_sec,
            created_at: Utc::now(),
            timeout_remaining_sec: Some(timeout_sec),
        };

        // Persist to SQLite
        if let Err(e) = self.db.insert_agent(&info) {
            warn!("Failed to persist agent to database: {e}");
        }

        let response = SpawnResponse {
            id,
            soul: req.soul.clone(),
            role: req.role.clone(),
            model: model.clone(),
            provider: provider_name.clone(),
            max_depth,
            max_children,
            timeout_remaining_sec: timeout_sec,
        };

        let handle = Arc::new(Mutex::new(AgentHandle {
            info,
            pause_notify: Arc::new(Notify::new()),
        }));

        // Release providers lock before write-locking agents
        drop(providers);

        // Register the agent
        {
            let mut agents = self.agents.write().await;
            agents.insert(id, handle.clone());
        }

        // If there's a parent, register this as a child
        if let Some(parent_id) = req.parent_id {
            let agents = self.agents.read().await;
            if let Some(parent_handle) = agents.get(&parent_id) {
                let mut parent = parent_handle.lock().await;
                if parent.info.children.len() as u32 >= parent.info.max_children {
                    drop(parent);
                    drop(agents);
                    let mut agents = self.agents.write().await;
                    agents.remove(&id);
                    bail!("Parent agent {parent_id} max_children exceeded");
                }
                parent.info.children.push(id);
            }
        }

        // Spawn the agent task
        let agent_handle = handle.clone();
        let db = self.db.clone();
        tokio::spawn(async move {
            run_agent_loop(agent_handle, provider_arc, fallback_providers, db).await;
        });

        info!("Spawned agent {id} with task: {}", req.task);
        Ok(response)
    }

    pub async fn get_status(&self, id: u64) -> Result<AgentInfo> {
        // Try in-memory first (live agents in this process)
        {
            let agents = self.agents.read().await;
            if let Some(handle) = agents.get(&id) {
                let h = handle.lock().await;
                let mut info = h.info.clone();
                let elapsed = (Utc::now() - info.created_at).num_seconds().max(0) as u64;
                info.timeout_remaining_sec = Some(info.timeout_sec.saturating_sub(elapsed));
                return Ok(info);
            }
        }

        // Fall back to database
        self.db
            .get_agent(id)?
            .ok_or_else(|| anyhow::anyhow!("Agent {id} not found"))
    }

    pub async fn list_agents(
        &self,
        soul_filter: Option<&str>,
        role_filter: Option<&str>,
    ) -> Result<Vec<AgentInfo>> {
        // Read all agents from database
        let mut agents = self.db.list_agents(soul_filter, role_filter)?;

        // Overlay live in-memory state for running agents in this process
        let live = self.agents.read().await;
        for info in agents.iter_mut() {
            if let Some(handle) = live.get(&info.id) {
                let h = handle.lock().await;
                *info = h.info.clone();
            }
            let elapsed = (Utc::now() - info.created_at).num_seconds().max(0) as u64;
            info.timeout_remaining_sec = Some(info.timeout_sec.saturating_sub(elapsed));
        }
        agents.sort_by_key(|a| a.id);
        Ok(agents)
    }

    pub async fn steer(&self, id: u64, req: SteerRequest) -> Result<SteerResponse> {
        let agents = self.agents.read().await;
        let handle = agents
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {id} not found"))?;
        let mut h = handle.lock().await;

        let mut instruction_appended = false;
        let mut system_prompt_patched = false;
        let mut delta: i64 = 0;

        if let Some(ref instruction) = req.instruction {
            h.info.messages.push(ChatMessage {
                role: "user".into(),
                content: instruction.clone(),
            });
            instruction_appended = true;
            let seq = h.info.messages.len() - 1;
            let _ = self
                .db
                .insert_message(id, seq, h.info.messages.last().unwrap());
        }

        if let Some(ref patch) = req.prompt_patch {
            let old_len = h.info.system_prompt.len() as i64;
            h.info.system_prompt.push('\n');
            h.info.system_prompt.push_str(patch);
            let new_len = h.info.system_prompt.len() as i64;
            delta = (new_len - old_len) / 4;
            system_prompt_patched = true;
            let _ = self.db.update_system_prompt(id, &h.info.system_prompt);
        }

        Ok(SteerResponse {
            id,
            action: "steer".into(),
            instruction_appended,
            system_prompt_patched,
            patch_size_delta_tokens: delta,
        })
    }

    pub async fn kill(&self, id: u64) -> Result<()> {
        let agents = self.agents.read().await;
        let handle = agents
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {id} not found"))?;
        let mut h = handle.lock().await;
        h.info.status = AgentStatus::Killed;
        h.pause_notify.notify_waiters();
        let _ = self.db.update_agent_status(
            id,
            &AgentStatus::Killed,
            Some("killed"),
            h.info.progress,
            None,
            None,
            None,
        );
        info!("Killed agent {id}");
        Ok(())
    }

    pub async fn kill_all(&self) -> Result<u32> {
        let agents = self.agents.read().await;
        let mut count = 0;
        for handle in agents.values() {
            let mut h = handle.lock().await;
            if matches!(
                h.info.status,
                AgentStatus::Running | AgentStatus::Queued | AgentStatus::Paused
            ) {
                h.info.status = AgentStatus::Killed;
                h.pause_notify.notify_waiters();
                let _ = self.db.update_agent_status(
                    h.info.id,
                    &AgentStatus::Killed,
                    Some("killed"),
                    h.info.progress,
                    None,
                    None,
                    None,
                );
                count += 1;
            }
        }
        info!("Killed {count} agents");
        Ok(count)
    }

    pub async fn pause(&self, id: u64) -> Result<()> {
        let agents = self.agents.read().await;
        let handle = agents
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {id} not found"))?;
        let mut h = handle.lock().await;
        if h.info.status != AgentStatus::Running {
            bail!("Agent {id} is not running (status: {})", h.info.status);
        }
        h.info.status = AgentStatus::Paused;
        let _ = self.db.update_agent_status(
            id,
            &AgentStatus::Paused,
            Some("paused"),
            h.info.progress,
            None,
            None,
            None,
        );
        info!("Paused agent {id}");
        Ok(())
    }

    pub async fn resume(&self, id: u64) -> Result<()> {
        let agents = self.agents.read().await;
        let handle = agents
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {id} not found"))?;
        let mut h = handle.lock().await;
        if h.info.status != AgentStatus::Paused {
            bail!("Agent {id} is not paused (status: {})", h.info.status);
        }
        h.info.status = AgentStatus::Running;
        h.pause_notify.notify_waiters();
        let _ = self.db.update_agent_status(
            id,
            &AgentStatus::Running,
            Some("executing"),
            h.info.progress,
            None,
            None,
            None,
        );
        info!("Resumed agent {id}");
        Ok(())
    }

    /// Wait for an agent to reach a terminal state, polling at the configured interval.
    pub async fn wait_for_completion(&self, id: u64) -> Result<AgentInfo> {
        let poll_ms = self.limits.agent_poll_interval_ms;
        loop {
            let info = self.get_status(id).await?;
            match info.status {
                AgentStatus::Completed | AgentStatus::Failed | AgentStatus::Killed => {
                    return Ok(info);
                }
                _ => {
                    tokio::time::sleep(std::time::Duration::from_millis(poll_ms)).await;
                }
            }
        }
    }

    pub async fn get_providers(&self) -> Vec<String> {
        let providers = self.providers.read().await;
        providers.keys().cloned().collect()
    }

    pub async fn check_provider(&self, name: &str) -> Result<String> {
        let providers = self.providers.read().await;
        let provider = providers
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Provider '{name}' not configured"))?;
        provider.health_check().await
    }

    pub async fn list_models(&self, provider_name: &str) -> Result<Vec<String>> {
        let providers = self.providers.read().await;
        let provider = providers
            .get(provider_name)
            .ok_or_else(|| anyhow::anyhow!("Provider '{provider_name}' not configured"))?;
        provider.list_models().await
    }
}

/// The main agent execution loop.
/// Tries the primary provider first; on failure, attempts fallback providers in order.
async fn run_agent_loop(
    handle: Arc<Mutex<AgentHandle>>,
    provider: Arc<dyn Provider>,
    fallback_providers: Vec<Arc<dyn Provider>>,
    db: Arc<Database>,
) {
    let (messages, system_prompt, agent_id) = {
        let h = handle.lock().await;
        (
            h.info.messages.clone(),
            h.info.system_prompt.clone(),
            h.info.id,
        )
    };

    let sys_owned = if system_prompt.is_empty() {
        None
    } else {
        Some(system_prompt.clone())
    };
    let sys = sys_owned.as_deref();

    {
        let mut h = handle.lock().await;
        h.info.phase = Some("executing".into());
        h.info.progress = 0.1;
    }
    let _ = db.update_agent_status(
        agent_id,
        &AgentStatus::Running,
        Some("executing"),
        0.1,
        None,
        None,
        None,
    );

    // Build the ordered list of providers to try: primary first, then fallbacks
    let mut providers_to_try: Vec<Arc<dyn Provider>> = vec![provider];
    providers_to_try.extend(fallback_providers);

    let mut last_error: Option<String> = None;
    let mut success = false;

    for (idx, prov) in providers_to_try.iter().enumerate() {
        if idx > 0 {
            // Update phase to indicate fallback attempt
            let phase = format!("retrying-provider-{idx}");
            let mut h = handle.lock().await;
            if h.info.status == AgentStatus::Killed {
                return;
            }
            h.info.phase = Some(phase.clone());
            let _ = db.update_agent_status(
                agent_id,
                &AgentStatus::Running,
                Some(&phase),
                0.1,
                None,
                None,
                None,
            );
            info!(
                "Agent {agent_id}: primary provider failed, trying fallback provider '{}'",
                prov.name()
            );
        }

        match prov.chat(&messages, sys).await {
            Ok(resp) => {
                let mut h = handle.lock().await;
                if h.info.status == AgentStatus::Killed {
                    return;
                }
                h.info.last_output = Some(resp.content.clone());
                h.info.last_output_tokens = resp.tokens_used;
                h.info.provider = prov.name().to_string();
                h.info.messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: resp.content.clone(),
                });
                h.info.progress = 1.0;
                h.info.phase = Some("completed".into());
                h.info.status = AgentStatus::Completed;

                let _ = db.update_agent_status(
                    agent_id,
                    &AgentStatus::Completed,
                    Some("completed"),
                    1.0,
                    Some(&resp.content),
                    resp.tokens_used,
                    None,
                );
                let seq = h.info.messages.len() - 1;
                let _ = db.insert_message(agent_id, seq, h.info.messages.last().unwrap());

                if let Err(e) = write_agent_log(&h.info) {
                    warn!("Failed to write agent log: {e}");
                }
                success = true;
                break;
            }
            Err(e) => {
                warn!(
                    "Agent {agent_id}: provider '{}' failed: {e}",
                    prov.name()
                );
                last_error = Some(e.to_string());
            }
        }
    }

    if !success {
        let err_msg = last_error.unwrap_or_else(|| "Unknown error".into());
        let mut h = handle.lock().await;
        if h.info.status != AgentStatus::Killed {
            h.info.status = AgentStatus::Failed;
            h.info.phase = Some("failed".into());
            h.info.last_output = Some(format!("Error: {err_msg}"));

            let _ = db.update_agent_status(
                agent_id,
                &AgentStatus::Failed,
                Some("failed"),
                h.info.progress,
                Some(&format!("Error: {err_msg}")),
                None,
                Some(&err_msg),
            );

            if let Err(e) = write_agent_log(&h.info) {
                warn!("Failed to write agent log: {e}");
            }
        }
    }
}

fn write_agent_log(info: &AgentInfo) -> Result<()> {
    let log_dir = crate::config::mcp_home().join("logs");
    std::fs::create_dir_all(&log_dir)?;
    let timestamp = info.created_at.format("%Y%m%d_%H%M%S");
    let path = log_dir.join(format!("agent.{}-{timestamp}.log", info.id));
    let content = serde_json::to_string_pretty(info)?;
    std::fs::write(path, content)?;
    Ok(())
}
