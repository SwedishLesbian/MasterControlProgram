use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde_json::json;
use std::sync::Arc;
use tracing::info;

use crate::agent::{AgentManager, SpawnRequest, SteerRequest};
use crate::config::McpConfig;
use crate::workflow::WorkflowRunner;

#[derive(Clone)]
struct AppState {
    manager: Arc<AgentManager>,
    workflow_runner: Arc<WorkflowRunner>,
}

pub async fn run_server(
    config: &McpConfig,
    manager: Arc<AgentManager>,
    workflow_runner: Arc<WorkflowRunner>,
) -> anyhow::Result<()> {
    let bind = config.server.bind.clone();

    let state = AppState {
        manager,
        workflow_runner,
    };

    let app = Router::new()
        // Agent endpoints
        .route("/spawn", post(handle_spawn))
        .route("/agent/{id}", get(handle_agent_status))
        .route("/agent/{id}/steer", post(handle_steer))
        .route("/agent/{id}/kill", post(handle_kill))
        .route("/agent/{id}/pause", post(handle_pause))
        .route("/agent/{id}/resume", post(handle_resume))
        .route("/agents", get(handle_list_agents))
        // Provider endpoints
        .route("/providers", get(handle_list_providers))
        .route("/providers/{name}/check", get(handle_check_provider))
        // Tool registry endpoints
        .route("/tools", get(handle_list_tools))
        .route("/tools/{name}", get(handle_get_tool))
        // Workflow endpoints
        .route("/workflows", get(handle_list_workflows))
        .route("/workflows/{name}", get(handle_get_workflow))
        .route("/workflows/run", post(handle_run_workflow))
        .route("/workflow-runs/{id}", get(handle_workflow_run_status))
        .route("/workflow-runs/{id}/stop", post(handle_stop_workflow_run))
        // MCP discovery
        .route("/mcp-tools", get(handle_mcp_tools))
        .with_state(state);

    info!("MCP server listening on {bind}");
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── Agent Handlers ─────────────────────────────────────────────────

async fn handle_spawn(
    State(state): State<AppState>,
    Json(req): Json<SpawnRequest>,
) -> impl IntoResponse {
    match state.manager.spawn(req).await {
        Ok(resp) => (StatusCode::OK, Json(json!(resp))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_agent_status(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.manager.get_status(id).await {
        Ok(info) => (StatusCode::OK, Json(json!(info))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_steer(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<SteerRequest>,
) -> impl IntoResponse {
    match state.manager.steer(id, req).await {
        Ok(resp) => (StatusCode::OK, Json(json!(resp))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_kill(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.manager.kill(id).await {
        Ok(()) => (StatusCode::OK, Json(json!({"killed": id}))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_pause(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.manager.pause(id).await {
        Ok(()) => (StatusCode::OK, Json(json!({"paused": id}))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_resume(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.manager.resume(id).await {
        Ok(()) => (StatusCode::OK, Json(json!({"resumed": id}))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_list_agents(State(state): State<AppState>) -> impl IntoResponse {
    match state.manager.list_agents(None, None).await {
        Ok(agents) => (StatusCode::OK, Json(json!(agents))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

// ── Provider Handlers ──────────────────────────────────────────────

async fn handle_list_providers(State(state): State<AppState>) -> impl IntoResponse {
    let providers = state.manager.get_providers().await;
    (StatusCode::OK, Json(json!({"providers": providers})))
}

async fn handle_check_provider(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.manager.check_provider(&name).await {
        Ok(msg) => (StatusCode::OK, Json(json!({"status": msg}))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

// ── Tool Registry Handlers ─────────────────────────────────────────

async fn handle_list_tools() -> impl IntoResponse {
    match crate::tool::discovery_response() {
        Ok(tools) => (StatusCode::OK, Json(tools)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_get_tool(Path(name): Path<String>) -> impl IntoResponse {
    match crate::tool::get_tool(&name) {
        Ok(tool) => (StatusCode::OK, Json(json!(tool))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

// ── Workflow Handlers ──────────────────────────────────────────────

async fn handle_list_workflows() -> impl IntoResponse {
    match crate::workflow::list_workflows() {
        Ok(workflows) => {
            let entries: Vec<serde_json::Value> = workflows
                .iter()
                .map(|w| {
                    json!({
                        "name": w.name,
                        "version": w.version,
                        "description": w.description,
                        "steps": w.steps.len(),
                    })
                })
                .collect();
            (StatusCode::OK, Json(json!(entries)))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_get_workflow(Path(name): Path<String>) -> impl IntoResponse {
    match crate::workflow::get_workflow(&name) {
        Ok(wf) => (StatusCode::OK, Json(json!(wf))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

#[derive(serde::Deserialize)]
struct RunWorkflowRequest {
    name: String,
}

async fn handle_run_workflow(
    State(state): State<AppState>,
    Json(req): Json<RunWorkflowRequest>,
) -> impl IntoResponse {
    match crate::workflow::load_workflow(&req.name) {
        Ok(wf) => match state
            .workflow_runner
            .run(wf.clone(), state.manager.clone())
            .await
        {
            Ok(run_id) => (
                StatusCode::OK,
                Json(json!({"run_id": run_id, "workflow": wf.name, "status": "running"})),
            ),
            Err(e) => (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": e.to_string()})),
            ),
        },
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_workflow_run_status(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.workflow_runner.get_run(id).await {
        Ok(info) => (StatusCode::OK, Json(json!(info))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_stop_workflow_run(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.workflow_runner.stop(id).await {
        Ok(()) => (StatusCode::OK, Json(json!({"stopped": id}))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

// ── MCP Discovery ──────────────────────────────────────────────────

async fn handle_mcp_tools() -> impl IntoResponse {
    // Merge built-in tools with registered tools
    let mut all_tools = vec![
        json!({
            "name": "spawn_agent",
            "description": "Spawn a new AI subagent with a task, role, and model",
            "input_schema": {
                "type": "object",
                "properties": {
                    "task": {"type": "string", "description": "The task for the agent"},
                    "role": {"type": "string", "description": "Role name"},
                    "soul": {"type": "string", "description": "Soul/identity label"},
                    "model": {"type": "string", "description": "Model ID"},
                    "provider": {"type": "string", "description": "Provider name"}
                },
                "required": ["task"]
            }
        }),
        json!({
            "name": "kill_agent",
            "description": "Kill a running agent by ID",
            "input_schema": {
                "type": "object",
                "properties": {
                    "id": {"type": "integer", "description": "Agent ID"}
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "query_agent_status",
            "description": "Get the status of an agent by ID",
            "input_schema": {
                "type": "object",
                "properties": {
                    "id": {"type": "integer", "description": "Agent ID"}
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "get_logs",
            "description": "Get logs for an agent by ID",
            "input_schema": {
                "type": "object",
                "properties": {
                    "id": {"type": "integer", "description": "Agent ID"}
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "run_workflow",
            "description": "Execute a named workflow",
            "input_schema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Workflow name or path"}
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "list_tools",
            "description": "List all registered tools",
            "input_schema": {
                "type": "object",
                "properties": {}
            }
        }),
    ];

    // Add registered user tools
    if let Ok(registered) = crate::tool::list_tools() {
        for t in registered {
            all_tools.push(json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
                "output_schema": t.output_schema,
            }));
        }
    }

    (StatusCode::OK, Json(json!({"tools": all_tools})))
}
