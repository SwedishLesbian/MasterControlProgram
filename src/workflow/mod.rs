use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use crate::agent::{AgentManager, AgentStatus, SpawnRequest, SteerRequest};
use crate::config::mcp_home;

// ── Workflow Definition (YAML) ─────────────────────────────────────

/// A complete workflow definition loaded from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub globals: WorkflowGlobals,
    pub steps: Vec<WorkflowStep>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkflowGlobals {
    #[serde(default)]
    pub max_depth: Option<u32>,
    #[serde(default)]
    pub default_role: Option<String>,
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
}

/// A single step in a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    pub action: StepAction,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub soul: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub prompt_patch: Option<String>,
    /// For summarize: list of step IDs whose outputs to gather.
    #[serde(default)]
    pub source: Option<Vec<String>>,
    #[serde(default)]
    pub timeout_sec: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StepAction {
    Spawn,
    Wait,
    Steer,
    Kill,
    Pause,
    Resume,
    Inspect,
    Summarize,
}

impl std::fmt::Display for StepAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn => write!(f, "spawn"),
            Self::Wait => write!(f, "wait"),
            Self::Steer => write!(f, "steer"),
            Self::Kill => write!(f, "kill"),
            Self::Pause => write!(f, "pause"),
            Self::Resume => write!(f, "resume"),
            Self::Inspect => write!(f, "inspect"),
            Self::Summarize => write!(f, "summarize"),
        }
    }
}

// ── Workflow Run State ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowRunStatus {
    Running,
    Completed,
    Failed,
    Stopped,
}

impl std::fmt::Display for WorkflowRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_id: String,
    pub action: String,
    pub status: String,
    pub agent_id: Option<u64>,
    pub output: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunInfo {
    pub run_id: u64,
    pub workflow_name: String,
    pub status: WorkflowRunStatus,
    pub current_step: Option<String>,
    pub step_results: Vec<StepResult>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

// ── Workflow Runner ────────────────────────────────────────────────

pub struct WorkflowRunner {
    runs: RwLock<HashMap<u64, Arc<Mutex<WorkflowRunInfo>>>>,
    next_run_id: Mutex<u64>,
}

impl WorkflowRunner {
    pub fn new() -> Self {
        Self {
            runs: RwLock::new(HashMap::new()),
            next_run_id: Mutex::new(1),
        }
    }

    /// Start executing a workflow. Returns the run ID.
    pub async fn run(
        &self,
        workflow: WorkflowDefinition,
        manager: Arc<AgentManager>,
    ) -> Result<u64> {
        validate_workflow(&workflow)?;

        let run_id = {
            let mut next = self.next_run_id.lock().await;
            let id = *next;
            *next += 1;
            id
        };

        let run_info = WorkflowRunInfo {
            run_id,
            workflow_name: workflow.name.clone(),
            status: WorkflowRunStatus::Running,
            current_step: None,
            step_results: vec![],
            started_at: Utc::now(),
            finished_at: None,
        };

        let run_handle = Arc::new(Mutex::new(run_info));
        {
            let mut runs = self.runs.write().await;
            runs.insert(run_id, run_handle.clone());
        }

        // Execute in background
        tokio::spawn(async move {
            execute_workflow(run_handle, workflow, manager).await;
        });

        Ok(run_id)
    }

    /// Get the status of a workflow run.
    pub async fn get_run(&self, run_id: u64) -> Result<WorkflowRunInfo> {
        let runs = self.runs.read().await;
        let handle = runs
            .get(&run_id)
            .ok_or_else(|| anyhow::anyhow!("Workflow run {run_id} not found"))?;
        let info = handle.lock().await;
        Ok(info.clone())
    }

    /// List all workflow runs.
    #[allow(dead_code)]
    pub async fn list_runs(&self) -> Vec<WorkflowRunInfo> {
        let runs = self.runs.read().await;
        let mut result = Vec::new();
        for handle in runs.values() {
            let info = handle.lock().await;
            result.push(info.clone());
        }
        result.sort_by_key(|r| r.run_id);
        result
    }

    /// Stop a running workflow.
    pub async fn stop(&self, run_id: u64) -> Result<()> {
        let runs = self.runs.read().await;
        let handle = runs
            .get(&run_id)
            .ok_or_else(|| anyhow::anyhow!("Workflow run {run_id} not found"))?;
        let mut info = handle.lock().await;
        if info.status == WorkflowRunStatus::Running {
            info.status = WorkflowRunStatus::Stopped;
            info.finished_at = Some(Utc::now());
            info!("Stopped workflow run {run_id}");
        }
        Ok(())
    }
}

/// Execute a workflow step-by-step.
async fn execute_workflow(
    run_handle: Arc<Mutex<WorkflowRunInfo>>,
    workflow: WorkflowDefinition,
    manager: Arc<AgentManager>,
) {
    // Map from step_id → agent_id (for steps that spawn agents)
    let mut step_agents: HashMap<String, u64> = HashMap::new();

    for step in &workflow.steps {
        // Check if stopped
        {
            let info = run_handle.lock().await;
            if info.status != WorkflowRunStatus::Running {
                return;
            }
        }

        // Update current step
        {
            let mut info = run_handle.lock().await;
            info.current_step = Some(step.id.clone());
        }

        let result = execute_step(step, &workflow.globals, &manager, &step_agents).await;

        match result {
            Ok(step_result) => {
                // Track agent IDs
                if let Some(agent_id) = step_result.agent_id {
                    step_agents.insert(step.id.clone(), agent_id);
                }

                let failed = step_result.status == "failed";
                {
                    let mut info = run_handle.lock().await;
                    info.step_results.push(step_result);
                    if failed {
                        info.status = WorkflowRunStatus::Failed;
                        info.finished_at = Some(Utc::now());
                        return;
                    }
                }
            }
            Err(e) => {
                let step_result = StepResult {
                    step_id: step.id.clone(),
                    action: step.action.to_string(),
                    status: "failed".into(),
                    agent_id: None,
                    output: None,
                    error: Some(e.to_string()),
                };
                let mut info = run_handle.lock().await;
                info.step_results.push(step_result);
                info.status = WorkflowRunStatus::Failed;
                info.finished_at = Some(Utc::now());
                return;
            }
        }
    }

    // All steps completed
    let mut info = run_handle.lock().await;
    if info.status == WorkflowRunStatus::Running {
        info.status = WorkflowRunStatus::Completed;
        info.finished_at = Some(Utc::now());
    }
}

/// Execute a single workflow step.
async fn execute_step(
    step: &WorkflowStep,
    globals: &WorkflowGlobals,
    manager: &AgentManager,
    step_agents: &HashMap<String, u64>,
) -> Result<StepResult> {
    match step.action {
        StepAction::Spawn => {
            let task = step
                .task
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Spawn step '{}' missing task", step.id))?;

            let role = step
                .role
                .as_ref()
                .or(globals.default_role.as_ref())
                .cloned();
            let provider = step
                .provider
                .as_ref()
                .or(globals.default_provider.as_ref())
                .cloned();
            let model = step
                .model
                .as_ref()
                .or(globals.default_model.as_ref())
                .cloned();

            let req = SpawnRequest {
                task: task.clone(),
                role,
                soul: step.soul.clone(),
                model,
                provider,
                depth: None,
                max_depth: globals.max_depth,
                max_children: None,
                timeout_sec: step.timeout_sec,
                system_prompt: None,
                parent_id: None,
            };

            let resp = manager.spawn(req).await?;
            info!(
                "Workflow step '{}': spawned agent {} (model: {})",
                step.id, resp.id, resp.model
            );

            Ok(StepResult {
                step_id: step.id.clone(),
                action: "spawn".into(),
                status: "ok".into(),
                agent_id: Some(resp.id),
                output: None,
                error: None,
            })
        }

        StepAction::Wait => {
            let agent_ref = step
                .agent
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Wait step '{}' missing agent ref", step.id))?;

            let agent_id = step_agents
                .get(agent_ref)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Wait step '{}': agent ref '{}' not found in earlier steps",
                        step.id,
                        agent_ref
                    )
                })?;

            let timeout = step.timeout_sec.unwrap_or(600);
            let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout);

            loop {
                let status = manager.get_status(*agent_id).await?;
                match status.status {
                    AgentStatus::Completed => {
                        return Ok(StepResult {
                            step_id: step.id.clone(),
                            action: "wait".into(),
                            status: "ok".into(),
                            agent_id: Some(*agent_id),
                            output: status.last_output,
                            error: None,
                        });
                    }
                    AgentStatus::Failed | AgentStatus::Killed => {
                        return Ok(StepResult {
                            step_id: step.id.clone(),
                            action: "wait".into(),
                            status: "failed".into(),
                            agent_id: Some(*agent_id),
                            output: status.last_output,
                            error: Some(format!("Agent {} ended with status: {}", agent_id, status.status)),
                        });
                    }
                    _ => {}
                }

                if tokio::time::Instant::now() >= deadline {
                    return Ok(StepResult {
                        step_id: step.id.clone(),
                        action: "wait".into(),
                        status: "failed".into(),
                        agent_id: Some(*agent_id),
                        output: None,
                        error: Some(format!("Timeout waiting for agent {agent_id}")),
                    });
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }

        StepAction::Steer => {
            let agent_ref = step
                .agent
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Steer step '{}' missing agent ref", step.id))?;

            let agent_id = step_agents
                .get(agent_ref)
                .ok_or_else(|| {
                    anyhow::anyhow!("Steer step '{}': agent '{}' not found", step.id, agent_ref)
                })?;

            let req = SteerRequest {
                instruction: step.instruction.clone(),
                prompt_patch: step.prompt_patch.clone(),
            };

            let resp = manager.steer(*agent_id, req).await?;
            info!("Workflow step '{}': steered agent {}", step.id, agent_id);

            Ok(StepResult {
                step_id: step.id.clone(),
                action: "steer".into(),
                status: "ok".into(),
                agent_id: Some(*agent_id),
                output: Some(format!(
                    "instruction_appended={}, prompt_patched={}",
                    resp.instruction_appended, resp.system_prompt_patched
                )),
                error: None,
            })
        }

        StepAction::Kill => {
            let agent_ref = step
                .agent
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Kill step '{}' missing agent ref", step.id))?;

            let agent_id = step_agents
                .get(agent_ref)
                .ok_or_else(|| {
                    anyhow::anyhow!("Kill step '{}': agent '{}' not found", step.id, agent_ref)
                })?;

            manager.kill(*agent_id).await?;

            Ok(StepResult {
                step_id: step.id.clone(),
                action: "kill".into(),
                status: "ok".into(),
                agent_id: Some(*agent_id),
                output: None,
                error: None,
            })
        }

        StepAction::Pause => {
            let agent_ref = step
                .agent
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Pause step '{}' missing agent ref", step.id))?;

            let agent_id = step_agents.get(agent_ref).ok_or_else(|| {
                anyhow::anyhow!("Pause step '{}': agent '{}' not found", step.id, agent_ref)
            })?;

            manager.pause(*agent_id).await?;

            Ok(StepResult {
                step_id: step.id.clone(),
                action: "pause".into(),
                status: "ok".into(),
                agent_id: Some(*agent_id),
                output: None,
                error: None,
            })
        }

        StepAction::Resume => {
            let agent_ref = step
                .agent
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Resume step '{}' missing agent ref", step.id))?;

            let agent_id = step_agents.get(agent_ref).ok_or_else(|| {
                anyhow::anyhow!(
                    "Resume step '{}': agent '{}' not found",
                    step.id,
                    agent_ref
                )
            })?;

            manager.resume(*agent_id).await?;

            Ok(StepResult {
                step_id: step.id.clone(),
                action: "resume".into(),
                status: "ok".into(),
                agent_id: Some(*agent_id),
                output: None,
                error: None,
            })
        }

        StepAction::Inspect => {
            let agent_ref = step
                .agent
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Inspect step '{}' missing agent ref", step.id))?;

            let agent_id = step_agents.get(agent_ref).ok_or_else(|| {
                anyhow::anyhow!(
                    "Inspect step '{}': agent '{}' not found",
                    step.id,
                    agent_ref
                )
            })?;

            let status = manager.get_status(*agent_id).await?;

            Ok(StepResult {
                step_id: step.id.clone(),
                action: "inspect".into(),
                status: "ok".into(),
                agent_id: Some(*agent_id),
                output: Some(serde_json::to_string(&status)?),
                error: None,
            })
        }

        StepAction::Summarize => {
            let sources = step.source.as_ref().ok_or_else(|| {
                anyhow::anyhow!("Summarize step '{}' missing source list", step.id)
            })?;

            let mut summaries = Vec::new();
            for src in sources {
                if let Some(agent_id) = step_agents.get(src) {
                    match manager.get_status(*agent_id).await {
                        Ok(status) => {
                            summaries.push(serde_json::json!({
                                "step": src,
                                "agent_id": agent_id,
                                "status": status.status.to_string(),
                                "output": status.last_output,
                            }));
                        }
                        Err(e) => {
                            summaries.push(serde_json::json!({
                                "step": src,
                                "error": e.to_string(),
                            }));
                        }
                    }
                } else {
                    summaries.push(serde_json::json!({
                        "step": src,
                        "error": "agent not found for step",
                    }));
                }
            }

            let summary = serde_json::to_string_pretty(&summaries)?;

            Ok(StepResult {
                step_id: step.id.clone(),
                action: "summarize".into(),
                status: "ok".into(),
                agent_id: None,
                output: Some(summary),
                error: None,
            })
        }
    }
}

// ── Workflow File I/O ──────────────────────────────────────────────

fn workflows_dir() -> PathBuf {
    mcp_home().join("workflows")
}

/// Load a workflow from a YAML file path or name.
pub fn load_workflow(name_or_path: &str) -> Result<WorkflowDefinition> {
    // Try as literal path first
    let path = std::path::Path::new(name_or_path);
    if path.exists() {
        let text = std::fs::read_to_string(path)?;
        return parse_workflow_yaml(&text);
    }

    // Try in ~/.mastercontrolprogram/workflows/
    let wf_path = workflows_dir().join(name_or_path);
    if wf_path.exists() {
        let text = std::fs::read_to_string(&wf_path)?;
        return parse_workflow_yaml(&text);
    }

    // Try with .yaml extension
    let wf_yaml = workflows_dir().join(format!("{name_or_path}.yaml"));
    if wf_yaml.exists() {
        let text = std::fs::read_to_string(&wf_yaml)?;
        return parse_workflow_yaml(&text);
    }

    // Try with .yml extension
    let wf_yml = workflows_dir().join(format!("{name_or_path}.yml"));
    if wf_yml.exists() {
        let text = std::fs::read_to_string(&wf_yml)?;
        return parse_workflow_yaml(&text);
    }

    bail!("Workflow '{name_or_path}' not found");
}

/// Parse a workflow from YAML text.
pub fn parse_workflow_yaml(yaml_text: &str) -> Result<WorkflowDefinition> {
    serde_yaml::from_str(yaml_text).context("Failed to parse workflow YAML")
}

/// Parse a workflow from YAML text (temporary, not saved to disk).
#[allow(dead_code)]
pub fn parse_temporary_workflow(yaml_text: &str) -> Result<WorkflowDefinition> {
    let wf = parse_workflow_yaml(yaml_text)?;
    validate_workflow(&wf)?;
    Ok(wf)
}

/// List saved workflow files.
pub fn list_workflows() -> Result<Vec<WorkflowDefinition>> {
    let dir = workflows_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut workflows = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str());
        if matches!(ext, Some("yaml") | Some("yml")) {
            match std::fs::read_to_string(&path) {
                Ok(text) => match parse_workflow_yaml(&text) {
                    Ok(wf) => workflows.push(wf),
                    Err(e) => warn!("Skipping invalid workflow {}: {e}", path.display()),
                },
                Err(e) => warn!("Failed to read {}: {e}", path.display()),
            }
        }
    }
    workflows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(workflows)
}

/// Get a specific workflow by name.
pub fn get_workflow(name: &str) -> Result<WorkflowDefinition> {
    load_workflow(name)
}

// ── Validation ─────────────────────────────────────────────────────

/// Validate a workflow definition.
pub fn validate_workflow(wf: &WorkflowDefinition) -> Result<()> {
    if wf.name.is_empty() {
        bail!("Workflow name cannot be empty");
    }
    if wf.steps.is_empty() {
        bail!("Workflow must have at least one step");
    }

    // Check unique step IDs
    let mut ids = std::collections::HashSet::new();
    for step in &wf.steps {
        if step.id.is_empty() {
            bail!("Step ID cannot be empty");
        }
        if !ids.insert(&step.id) {
            bail!("Duplicate step ID: '{}'", step.id);
        }
    }

    // Validate step references
    for (i, step) in wf.steps.iter().enumerate() {
        match step.action {
            StepAction::Spawn => {
                if step.task.is_none() {
                    bail!("Spawn step '{}' must have a task", step.id);
                }
            }
            StepAction::Wait | StepAction::Steer | StepAction::Kill | StepAction::Pause
            | StepAction::Resume | StepAction::Inspect => {
                if let Some(ref agent_ref) = step.agent {
                    // Agent reference must point to a prior spawn step
                    let valid = wf.steps[..i].iter().any(|s| {
                        s.id == *agent_ref && s.action == StepAction::Spawn
                    });
                    if !valid {
                        bail!(
                            "Step '{}' references agent '{}' which is not a prior spawn step",
                            step.id,
                            agent_ref
                        );
                    }
                } else {
                    bail!(
                        "Step '{}' (action: {}) must specify an agent reference",
                        step.id,
                        step.action
                    );
                }
            }
            StepAction::Summarize => {
                if step.source.is_none() || step.source.as_ref().unwrap().is_empty() {
                    bail!("Summarize step '{}' must have non-empty source list", step.id);
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_YAML: &str = r#"
name: test_workflow
version: 1
description: A test workflow
globals:
  max_depth: 2
  default_role: coder
steps:
  - id: code
    action: spawn
    role: coder
    task: "Write hello world"
  - id: wait_code
    action: wait
    agent: code
  - id: finish
    action: summarize
    source: [code]
"#;

    #[test]
    fn test_parse_workflow_yaml() {
        let wf = parse_workflow_yaml(SAMPLE_YAML).unwrap();
        assert_eq!(wf.name, "test_workflow");
        assert_eq!(wf.version, 1);
        assert_eq!(wf.steps.len(), 3);
        assert_eq!(wf.steps[0].action, StepAction::Spawn);
        assert_eq!(wf.steps[1].action, StepAction::Wait);
        assert_eq!(wf.steps[2].action, StepAction::Summarize);
    }

    #[test]
    fn test_validate_valid_workflow() {
        let wf = parse_workflow_yaml(SAMPLE_YAML).unwrap();
        assert!(validate_workflow(&wf).is_ok());
    }

    #[test]
    fn test_validate_empty_name() {
        let wf = WorkflowDefinition {
            name: "".into(),
            version: 1,
            description: "bad".into(),
            globals: Default::default(),
            steps: vec![WorkflowStep {
                id: "s1".into(),
                action: StepAction::Spawn,
                task: Some("x".into()),
                role: None,
                soul: None,
                agent: None,
                instruction: None,
                prompt_patch: None,
                source: None,
                timeout_sec: None,
                model: None,
                provider: None,
            }],
        };
        assert!(validate_workflow(&wf).is_err());
    }

    #[test]
    fn test_validate_empty_steps() {
        let wf = WorkflowDefinition {
            name: "empty".into(),
            version: 1,
            description: "".into(),
            globals: Default::default(),
            steps: vec![],
        };
        assert!(validate_workflow(&wf).is_err());
    }

    #[test]
    fn test_validate_duplicate_step_ids() {
        let yaml = r#"
name: dup
steps:
  - id: s1
    action: spawn
    task: "x"
  - id: s1
    action: spawn
    task: "y"
"#;
        let wf = parse_workflow_yaml(yaml).unwrap();
        assert!(validate_workflow(&wf).is_err());
    }

    #[test]
    fn test_validate_spawn_missing_task() {
        let yaml = r#"
name: no_task
steps:
  - id: s1
    action: spawn
"#;
        let wf = parse_workflow_yaml(yaml).unwrap();
        assert!(validate_workflow(&wf).is_err());
    }

    #[test]
    fn test_validate_wait_missing_agent() {
        let yaml = r#"
name: no_agent
steps:
  - id: s1
    action: spawn
    task: "x"
  - id: s2
    action: wait
"#;
        let wf = parse_workflow_yaml(yaml).unwrap();
        assert!(validate_workflow(&wf).is_err());
    }

    #[test]
    fn test_validate_bad_agent_ref() {
        let yaml = r#"
name: bad_ref
steps:
  - id: s1
    action: spawn
    task: "x"
  - id: s2
    action: wait
    agent: nonexistent
"#;
        let wf = parse_workflow_yaml(yaml).unwrap();
        assert!(validate_workflow(&wf).is_err());
    }

    #[test]
    fn test_validate_summarize_empty_source() {
        let yaml = r#"
name: empty_summary
steps:
  - id: s1
    action: spawn
    task: "x"
  - id: s2
    action: summarize
    source: []
"#;
        let wf = parse_workflow_yaml(yaml).unwrap();
        assert!(validate_workflow(&wf).is_err());
    }

    #[test]
    fn test_step_action_display() {
        assert_eq!(StepAction::Spawn.to_string(), "spawn");
        assert_eq!(StepAction::Wait.to_string(), "wait");
        assert_eq!(StepAction::Steer.to_string(), "steer");
        assert_eq!(StepAction::Kill.to_string(), "kill");
        assert_eq!(StepAction::Summarize.to_string(), "summarize");
    }

    #[test]
    fn test_workflow_run_status_display() {
        assert_eq!(WorkflowRunStatus::Running.to_string(), "running");
        assert_eq!(WorkflowRunStatus::Completed.to_string(), "completed");
        assert_eq!(WorkflowRunStatus::Failed.to_string(), "failed");
        assert_eq!(WorkflowRunStatus::Stopped.to_string(), "stopped");
    }

    #[test]
    fn test_globals_default() {
        let yaml = r#"
name: minimal
steps:
  - id: s1
    action: spawn
    task: "hello"
"#;
        let wf = parse_workflow_yaml(yaml).unwrap();
        assert!(wf.globals.max_depth.is_none());
        assert!(wf.globals.default_role.is_none());
    }

    #[test]
    fn test_complex_workflow() {
        let yaml = r#"
name: complex
version: 2
description: Multi-step workflow
globals:
  max_depth: 3
  default_role: coder
  default_provider: nvidia-nim
  default_model: llama-3
steps:
  - id: design
    action: spawn
    role: architect
    soul: system-designer
    task: "Design the system"
    timeout_sec: 300
  - id: wait_design
    action: wait
    agent: design
  - id: steer_design
    action: steer
    agent: design
    instruction: "Add error handling"
  - id: implement
    action: spawn
    task: "Implement the design"
    model: gpt-4
    provider: openai
  - id: wait_impl
    action: wait
    agent: implement
  - id: summary
    action: summarize
    source: [design, implement]
"#;
        let wf = parse_workflow_yaml(yaml).unwrap();
        assert_eq!(wf.name, "complex");
        assert_eq!(wf.version, 2);
        assert_eq!(wf.steps.len(), 6);
        assert_eq!(wf.globals.default_provider, Some("nvidia-nim".into()));
        assert!(validate_workflow(&wf).is_ok());
    }
}
