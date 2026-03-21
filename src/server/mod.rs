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

type AppState = Arc<AgentManager>;

pub async fn run_server(config: &McpConfig, manager: Arc<AgentManager>) -> anyhow::Result<()> {
    let bind = config.server.bind.clone();

    let app = Router::new()
        .route("/spawn", post(handle_spawn))
        .route("/agent/{id}", get(handle_agent_status))
        .route("/agent/{id}/steer", post(handle_steer))
        .route("/agent/{id}/kill", post(handle_kill))
        .route("/agent/{id}/pause", post(handle_pause))
        .route("/agent/{id}/resume", post(handle_resume))
        .route("/agents", get(handle_list_agents))
        .route("/providers", get(handle_list_providers))
        .route("/providers/{name}/check", get(handle_check_provider))
        .route("/mcp-tools", get(handle_mcp_tools))
        .with_state(manager);

    info!("MCP server listening on {bind}");
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_spawn(
    State(mgr): State<AppState>,
    Json(req): Json<SpawnRequest>,
) -> impl IntoResponse {
    match mgr.spawn(req).await {
        Ok(resp) => (StatusCode::OK, Json(json!(resp))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_agent_status(
    State(mgr): State<AppState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match mgr.get_status(id).await {
        Ok(info) => (StatusCode::OK, Json(json!(info))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_steer(
    State(mgr): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<SteerRequest>,
) -> impl IntoResponse {
    match mgr.steer(id, req).await {
        Ok(resp) => (StatusCode::OK, Json(json!(resp))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_kill(
    State(mgr): State<AppState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match mgr.kill(id).await {
        Ok(()) => (StatusCode::OK, Json(json!({"killed": id}))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_pause(
    State(mgr): State<AppState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match mgr.pause(id).await {
        Ok(()) => (StatusCode::OK, Json(json!({"paused": id}))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_resume(
    State(mgr): State<AppState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match mgr.resume(id).await {
        Ok(()) => (StatusCode::OK, Json(json!({"resumed": id}))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_list_agents(State(mgr): State<AppState>) -> impl IntoResponse {
    match mgr.list_agents(None, None).await {
        Ok(agents) => (StatusCode::OK, Json(json!(agents))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_list_providers(State(mgr): State<AppState>) -> impl IntoResponse {
    let providers = mgr.get_providers().await;
    (StatusCode::OK, Json(json!({"providers": providers})))
}

async fn handle_check_provider(
    State(mgr): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match mgr.check_provider(&name).await {
        Ok(msg) => (StatusCode::OK, Json(json!({"status": msg}))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        ),
    }
}

async fn handle_mcp_tools() -> impl IntoResponse {
    let tools = json!({
        "tools": [
            {
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
            },
            {
                "name": "kill_agent",
                "description": "Kill a running agent by ID",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer", "description": "Agent ID"}
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "query_agent_status",
                "description": "Get the status of an agent by ID",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer", "description": "Agent ID"}
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "get_logs",
                "description": "Get logs for an agent by ID",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer", "description": "Agent ID"}
                    },
                    "required": ["id"]
                }
            }
        ]
    });
    (StatusCode::OK, Json(tools))
}
