mod agent;
mod cli;
mod config;
mod logging;
mod persistence;
mod provider;
mod role;
mod server;
mod tool;
mod workflow;

use anyhow::Result;
use clap::Parser;
use serde_json::json;
use std::sync::Arc;

use crate::agent::{AgentManager, SpawnRequest, SteerRequest};
use crate::cli::*;
use crate::config::{ensure_dirs, load_config};
use crate::role::RoleDefinition;

/// Known subcommands — used to detect direct-prompt mode.
const KNOWN_SUBCOMMANDS: &[&str] = &[
    "spawn", "status", "agent", "agents", "role", "config",
    "provider", "tool", "workflow", "server", "logs", "diagnose", "alias",
];

/// Returns true if the process args indicate direct-prompt mode:
/// i.e. the first non-flag argument is not a known subcommand.
fn is_direct_mode() -> bool {
    let args: Vec<String> = std::env::args().collect();
    let first = args.iter().skip(1).find(|a| !a.starts_with('-'));
    match first {
        Some(f) => !KNOWN_SUBCOMMANDS.contains(&f.as_str()),
        None => false,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    logging::init_logging()?;
    ensure_dirs()?;

    // Direct-prompt mode: mastercontrolprogram "some task" [--role X] [--model X] [--provider X]
    if is_direct_mode() {
        let config = load_config()?;
        let db = Arc::new(persistence::Database::open()?);
        let manager = Arc::new(AgentManager::new(&config, db)?);
        let exit_code = run_direct_prompt(&manager, &config).await;
        std::process::exit(exit_code);
    }

    let cli = Cli::parse();
    let config = load_config()?;
    let json_output = cli.json || config.cli.json_output;

    let db = Arc::new(persistence::Database::open()?);
    let manager = Arc::new(AgentManager::new(&config, db)?);
    let workflow_runner = Arc::new(workflow::WorkflowRunner::new());

    let exit_code = match cli.command {
        Commands::Spawn {
            task,
            detach,
            role: role_name,
            model,
            provider,
            depth,
            max_depth,
            max_children,
            soul,
            timeout,
            system_prompt,
            parent,
        } => {
            // Resolve role: use --role flag, or fall back to config default
            let effective_role_name = role_name.or_else(|| config.default.role.clone());
            let mut sys_prompt = system_prompt;
            let mut resolved_model = model;
            let mut resolved_provider = provider;
            let mut resolved_soul = soul;
            let mut resolved_max_depth = max_depth;
            let mut resolved_max_children = max_children;
            let mut resolved_role_name = effective_role_name.clone();

            if let Some(ref rn) = effective_role_name {
                if let Ok(role_def) = role::get_role(rn) {
                    if sys_prompt.is_none() {
                        sys_prompt =
                            Some(role::resolve_system_prompt(&role_def).unwrap_or_default());
                    }
                    if resolved_model.is_none() {
                        resolved_model = role_def.default_model;
                    }
                    if resolved_provider.is_none() {
                        resolved_provider = role_def.default_provider;
                    }
                    if resolved_soul.is_none() {
                        resolved_soul = role_def.soul;
                    }
                    if resolved_max_depth.is_none() {
                        resolved_max_depth = Some(role_def.max_depth);
                    }
                    if resolved_max_children.is_none() {
                        resolved_max_children = Some(role_def.max_children);
                    }
                    resolved_role_name = Some(role_def.role.unwrap_or(rn.clone()));
                }
            }

            let req = SpawnRequest {
                task,
                role: resolved_role_name,
                soul: resolved_soul,
                model: resolved_model,
                provider: resolved_provider,
                depth,
                max_depth: resolved_max_depth,
                max_children: resolved_max_children,
                timeout_sec: timeout,
                system_prompt: sys_prompt,
                parent_id: parent,
            };

            match manager.spawn(req).await {
                Ok(resp) => {
                    if json_output && detach {
                        println!("{}", serde_json::to_string_pretty(&resp)?);
                    } else if detach {
                        println!("Agent started with id {}", resp.id);
                        if let Some(ref s) = resp.soul {
                            println!("Soul: {s}");
                        }
                        if let Some(ref r) = resp.role {
                            println!("Role: {r}");
                        }
                        println!("Model: {}", resp.model);
                        println!("Provider: {}", resp.provider);
                        println!("(detached — use 'status {}' to check later)", resp.id);
                    } else {
                        // Wait for completion
                        if !json_output {
                            println!("Agent {} running (model: {}, provider: {})...",
                                resp.id, resp.model, resp.provider);
                        }
                        match manager.wait_for_completion(resp.id).await {
                            Ok(info) => {
                                if json_output {
                                    println!("{}", serde_json::to_string_pretty(&info)?);
                                } else {
                                    println!("\n── Agent {} {} ──", info.id, info.status);
                                    if let Some(ref output) = info.last_output {
                                        println!("{output}");
                                    } else {
                                        println!("(no output)");
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Error waiting for agent: {e}");
                            }
                        }
                    }
                    0
                }
                Err(e) => {
                    if json_output {
                        println!("{}", json!({"error": e.to_string()}));
                    } else {
                        eprintln!("Error: {e}");
                    }
                    if e.to_string().contains("quota") || e.to_string().contains("exceeded") {
                        10
                    } else {
                        1
                    }
                }
            }
        }

        Commands::Status { id } => match manager.get_status(id).await {
            Ok(info) => {
                if json_output {
                    println!("{}", serde_json::to_string_pretty(&info)?);
                } else {
                    println!("Agent {}: {}", info.id, info.status);
                    if let Some(ref phase) = info.phase {
                        println!("Phase: {phase}");
                    }
                    println!("Progress: {:.0}%", info.progress * 100.0);
                    println!("Model: {}", info.model);
                    println!("Provider: {}", info.provider);
                    if let Some(remaining) = info.timeout_remaining_sec {
                        println!("Timeout remaining: {remaining}s");
                    }
                    if let Some(ref output) = info.last_output {
                        println!("\n── Output ──");
                        println!("{output}");
                    }
                }
                0
            }
            Err(e) => {
                if json_output {
                    println!("{}", json!({"error": e.to_string()}));
                } else {
                    eprintln!("Error: {e}");
                }
                1
            }
        },

        Commands::Agent(AgentCommands::Steer {
            id,
            instruction,
            prompt_patch,
        }) => {
            let req = SteerRequest {
                instruction,
                prompt_patch,
            };
            match manager.steer(id, req).await {
                Ok(resp) => {
                    if json_output {
                        println!("{}", serde_json::to_string_pretty(&resp)?);
                    } else {
                        println!("Steered agent {}", resp.id);
                        if resp.instruction_appended {
                            println!("Instruction appended");
                        }
                        if resp.system_prompt_patched {
                            println!(
                                "System prompt patched (delta: {} tokens)",
                                resp.patch_size_delta_tokens
                            );
                        }
                    }
                    0
                }
                Err(e) => {
                    if json_output {
                        println!("{}", json!({"error": e.to_string()}));
                    } else {
                        eprintln!("Error: {e}");
                    }
                    1
                }
            }
        }

        Commands::Agent(AgentCommands::Kill { id, all }) => {
            if all {
                match manager.kill_all().await {
                    Ok(count) => {
                        if json_output {
                            println!("{}", json!({"killed_count": count}));
                        } else {
                            println!("Killed {count} agents");
                        }
                        0
                    }
                    Err(e) => {
                        eprintln!("Error: {e}");
                        1
                    }
                }
            } else if let Some(id) = id {
                match manager.kill(id).await {
                    Ok(()) => {
                        if json_output {
                            println!("{}", json!({"killed": id}));
                        } else {
                            println!("Killed agent {id}");
                        }
                        0
                    }
                    Err(e) => {
                        if json_output {
                            println!("{}", json!({"error": e.to_string()}));
                        } else {
                            eprintln!("Error: {e}");
                        }
                        1
                    }
                }
            } else {
                eprintln!("Error: provide an agent ID or --all");
                2
            }
        }

        Commands::Agent(AgentCommands::Pause { id }) => match manager.pause(id).await {
            Ok(()) => {
                if json_output {
                    println!("{}", json!({"paused": id}));
                } else {
                    println!("Paused agent {id}");
                }
                0
            }
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        },

        Commands::Agent(AgentCommands::Resume { id }) => match manager.resume(id).await {
            Ok(()) => {
                if json_output {
                    println!("{}", json!({"resumed": id}));
                } else {
                    println!("Resumed agent {id}");
                }
                0
            }
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        },

        Commands::Agents { cmd } => match cmd {
            AgentsCommands::List { soul, role } => {
                match manager
                    .list_agents(soul.as_deref(), role.as_deref())
                    .await
                {
                    Ok(agents) => {
                        if json_output {
                            println!("{}", serde_json::to_string_pretty(&agents)?);
                        } else if agents.is_empty() {
                            println!("No agents running");
                        } else {
                            println!(
                                "{:<5} {:<12} {:<20} {:<15} {:<10}",
                                "ID", "STATUS", "SOUL", "ROLE", "MODEL"
                            );
                            println!("{}", "-".repeat(62));
                            for a in &agents {
                                println!(
                                    "{:<5} {:<12} {:<20} {:<15} {:<10}",
                                    a.id,
                                    a.status.to_string(),
                                    a.soul.as_deref().unwrap_or("-"),
                                    a.role.as_deref().unwrap_or("-"),
                                    &a.model,
                                );
                            }
                        }
                        0
                    }
                    Err(e) => {
                        eprintln!("Error: {e}");
                        1
                    }
                }
            }
            AgentsCommands::Show { id } => match manager.get_status(id).await {
                Ok(info) => {
                    println!("{}", serde_json::to_string_pretty(&info)?);
                    0
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    1
                }
            },
        },

        Commands::Role(role_cmd) => handle_role_command(role_cmd, json_output)?,

        Commands::Config(config_cmd) => {
            handle_config_command(config_cmd, &config, &manager, json_output).await?
        }

        Commands::Tool(tool_cmd) => handle_tool_command(tool_cmd, json_output)?,

        Commands::Workflow(wf_cmd) => {
            handle_workflow_command(wf_cmd, &manager, &workflow_runner, json_output).await?
        }

        Commands::Provider(prov_cmd) => {
            handle_provider_command(prov_cmd, &config, &manager, json_output).await?
        }

        Commands::Server {
            bind,
            tls_cert: _,
            tls_key: _,
        } => {
            let mut server_config = config.clone();
            if let Some(b) = bind {
                server_config.server.bind = b;
            }
            server::run_server(&server_config, manager, workflow_runner).await?;
            0
        }

        Commands::Logs { id, since } => {
            if let Some(agent_id) = id {
                match logging::read_agent_log(agent_id)? {
                    Some(log) => {
                        if json_output {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&log) {
                                println!("{}", serde_json::to_string_pretty(&val)?);
                            } else {
                                println!("{}", json!({"log": log}));
                            }
                        } else {
                            println!("{log}");
                        }
                    }
                    None => {
                        println!("No logs found for agent {agent_id}");
                    }
                }
            } else {
                let since_str = since.as_deref().unwrap_or("10m");
                let logs = logging::read_logs_since(since_str)?;
                if json_output {
                    println!("{}", serde_json::to_string_pretty(&logs)?);
                } else if logs.is_empty() {
                    println!("No logs found since {since_str}");
                } else {
                    for log in &logs {
                        println!("{}", serde_json::to_string_pretty(log)?);
                        println!("---");
                    }
                }
            }
            0
        }

        Commands::Alias { name } => {
            let exe = std::env::current_exe()?;
            let exe_str = exe.display().to_string();

            if cfg!(target_os = "windows") {
                // Create a .cmd wrapper next to the binary
                let alias_path = exe.parent().unwrap().join(format!("{name}.cmd"));
                let content = format!("@echo off\r\n\"{exe_str}\" %*\r\n");
                std::fs::write(&alias_path, &content)?;
                if json_output {
                    println!(
                        "{}",
                        json!({"alias": name, "path": alias_path.display().to_string()})
                    );
                } else {
                    println!("Created alias '{name}' -> {}", alias_path.display());
                    println!(
                        "Ensure {} is in your PATH.",
                        exe.parent().unwrap().display()
                    );
                }
            } else {
                // Create a symlink next to the binary
                let alias_path = exe.parent().unwrap().join(&name);
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(&exe, &alias_path)?;
                }
                #[cfg(not(unix))]
                {
                    std::fs::copy(&exe, &alias_path)?;
                }
                if json_output {
                    println!(
                        "{}",
                        json!({"alias": name, "path": alias_path.display().to_string()})
                    );
                } else {
                    println!("Created alias '{name}' -> {}", alias_path.display());
                    println!(
                        "Ensure {} is in your PATH.",
                        exe.parent().unwrap().display()
                    );
                }
            }
            0
        }

        Commands::Diagnose => {
            let agents = manager.list_agents(None, None).await?;
            let providers = manager.get_providers().await;

            if json_output {
                let mut provider_health = serde_json::Map::new();
                for p in &providers {
                    let status = manager
                        .check_provider(p)
                        .await
                        .unwrap_or_else(|e| e.to_string());
                    provider_health.insert(p.clone(), json!(status));
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "agents": agents,
                        "providers": provider_health,
                        "agent_count": agents.len(),
                    }))?
                );
            } else {
                println!("=== MasterControlProgram Diagnostics ===\n");
                println!("Agents: {} total", agents.len());
                let running = agents
                    .iter()
                    .filter(|a| a.status == agent::AgentStatus::Running)
                    .count();
                println!("  Running: {running}");
                let completed = agents
                    .iter()
                    .filter(|a| a.status == agent::AgentStatus::Completed)
                    .count();
                println!("  Completed: {completed}");
                let failed = agents
                    .iter()
                    .filter(|a| a.status == agent::AgentStatus::Failed)
                    .count();
                println!("  Failed: {failed}");

                println!("\nProviders:");
                for p in &providers {
                    let status = manager
                        .check_provider(p)
                        .await
                        .unwrap_or_else(|e| format!("error: {e}"));
                    println!("  {p}: {status}");
                }
            }
            0
        }
    };

    std::process::exit(exit_code);
}

/// Run a prompt directly without a subcommand.
/// Syntax: mastercontrolprogram "task" [--role NAME] [--model ID] [--provider NAME] [--json]
/// Prints only the agent output to stdout and exits.
async fn run_direct_prompt(manager: &AgentManager, config: &config::McpConfig) -> i32 {
    let raw: Vec<String> = std::env::args().collect();

    // Parse simple flags from raw args
    let mut role: Option<String> = None;
    let mut model: Option<String> = None;
    let mut provider: Option<String> = None;
    let mut json_output = config.cli.json_output;
    let mut task_parts: Vec<String> = Vec::new();

    let mut i = 1usize;
    while i < raw.len() {
        match raw[i].as_str() {
            "--json" => {
                json_output = true;
            }
            "--role" => {
                i += 1;
                if i < raw.len() {
                    role = Some(raw[i].clone());
                }
            }
            "--model" => {
                i += 1;
                if i < raw.len() {
                    model = Some(raw[i].clone());
                }
            }
            "--provider" => {
                i += 1;
                if i < raw.len() {
                    provider = Some(raw[i].clone());
                }
            }
            arg if arg.starts_with("--role=") => {
                role = Some(arg["--role=".len()..].to_string());
            }
            arg if arg.starts_with("--model=") => {
                model = Some(arg["--model=".len()..].to_string());
            }
            arg if arg.starts_with("--provider=") => {
                provider = Some(arg["--provider=".len()..].to_string());
            }
            arg if !arg.starts_with('-') => {
                task_parts.push(arg.to_string());
            }
            _ => {}
        }
        i += 1;
    }

    let task = task_parts.join(" ");
    if task.is_empty() {
        eprintln!("Error: no task provided");
        return 1;
    }

    // Resolve role
    let mut sys_prompt: Option<String> = None;
    let mut resolved_model = model;
    let mut resolved_provider = provider;
    let mut resolved_soul: Option<String> = None;
    let mut resolved_role_name = role.clone();

    if let Some(ref rn) = role {
        if let Ok(role_def) = role::get_role(rn) {
            sys_prompt = Some(role::resolve_system_prompt(&role_def).unwrap_or_default());
            if resolved_model.is_none() {
                resolved_model = role_def.default_model;
            }
            if resolved_provider.is_none() {
                resolved_provider = role_def.default_provider;
            }
            resolved_soul = role_def.soul;
            resolved_role_name = Some(role_def.role.unwrap_or(rn.clone()));
        }
    }

    let req = SpawnRequest {
        task,
        role: resolved_role_name,
        soul: resolved_soul,
        model: resolved_model,
        provider: resolved_provider,
        depth: None,
        max_depth: None,
        max_children: None,
        timeout_sec: None,
        system_prompt: sys_prompt,
        parent_id: None,
    };

    match manager.spawn(req).await {
        Ok(resp) => {
            match manager.wait_for_completion(resp.id).await {
                Ok(info) => {
                    if json_output {
                        println!("{}", serde_json::to_string_pretty(&info).unwrap_or_default());
                    } else if let Some(ref output) = info.last_output {
                        println!("{output}");
                    }
                    if info.status == agent::AgentStatus::Failed {
                        1
                    } else {
                        0
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

fn handle_role_command(cmd: RoleCommands, json_output: bool) -> Result<i32> {
    match cmd {
        RoleCommands::Create {
            name,
            from,
            prompt,
            role: role_tag,
            soul,
            model,
            provider,
        } => {
            let system_prompt = if let Some(ref file) = from {
                Some(std::fs::read_to_string(file)?)
            } else {
                prompt
            };

            let role_def = RoleDefinition {
                name: name.clone(),
                soul,
                role: role_tag,
                prompt_file: from,
                system_prompt,
                default_model: model,
                default_provider: provider,
                max_depth: 2,
                max_children: 5,
                allowed_tools: vec![],
            };

            role::create_role(&role_def)?;

            if json_output {
                println!("{}", serde_json::to_string_pretty(&role_def)?);
            } else {
                println!("Role '{}' created", role_def.name);
            }
            Ok(0)
        }

        RoleCommands::List => {
            let roles = role::list_roles()?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&roles)?);
            } else if roles.is_empty() {
                println!("No roles defined");
            } else {
                println!("{:<15} {:<20} {:<15}", "NAME", "SOUL", "ROLE");
                println!("{}", "-".repeat(50));
                for r in &roles {
                    println!(
                        "{:<15} {:<20} {:<15}",
                        r.name,
                        r.soul.as_deref().unwrap_or("-"),
                        r.role.as_deref().unwrap_or("-"),
                    );
                }
            }
            Ok(0)
        }

        RoleCommands::Show { name } => {
            let role_def = role::get_role(&name)?;
            println!("{}", serde_json::to_string_pretty(&role_def)?);
            Ok(0)
        }

        RoleCommands::Delete { name } => {
            role::delete_role(&name)?;
            if json_output {
                println!("{}", json!({"deleted": name}));
            } else {
                println!("Role '{name}' deleted");
            }
            Ok(0)
        }

        RoleCommands::Patch {
            name,
            prompt_patch,
            model,
            provider,
        } => {
            let updated = role::patch_role(
                &name,
                prompt_patch.as_deref(),
                model.as_deref(),
                provider.as_deref(),
            )?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&updated)?);
            } else {
                println!("Role '{name}' updated");
            }
            Ok(0)
        }
    }
}

async fn handle_config_command(
    cmd: cli::ConfigCommands,
    config: &config::McpConfig,
    manager: &AgentManager,
    json_output: bool,
) -> Result<i32> {
    use cli::ConfigCommands;
    match cmd {
        ConfigCommands::Show => {
            if json_output {
                println!("{}", serde_json::to_string_pretty(config)?);
            } else {
                println!("Default provider:    {}", config.default.provider);
                println!("Default model:       {}", config.default.model);
                if let Some(ref sec) = config.default.secondary_provider {
                    println!(
                        "Secondary provider: {sec} (model: {})",
                        config.default.secondary_model.as_deref().unwrap_or("provider default")
                    );
                }
                if let Some(ref ter) = config.default.tertiary_provider {
                    println!(
                        "Tertiary provider:  {ter} (model: {})",
                        config.default.tertiary_model.as_deref().unwrap_or("provider default")
                    );
                }
                println!(
                    "Default role:        {}",
                    config.default.role.as_deref().unwrap_or("(none)")
                );
                println!(
                    "Default tool:        {}",
                    config.default.tool.as_deref().unwrap_or("(none)")
                );
                println!();
                println!("Server bind:         {}", config.server.bind);
                println!("Server enabled:      {}", config.server.enabled);
                println!();
                println!("Limits:");
                println!(
                    "  Max concurrent agents:  {}",
                    config.limits.max_concurrent_agents
                );
                println!("  Max depth:              {}", config.limits.max_depth);
                println!(
                    "  Max children/parent:    {}",
                    config.limits.max_children_per_parent
                );
                println!(
                    "  Agent timeout:          {}s",
                    config.limits.agent_timeout_sec
                );
                println!(
                    "  Agent poll interval:    {}ms",
                    config.limits.agent_poll_interval_ms
                );
                println!();
                println!("Providers:");
                for (name, entry) in &config.provider {
                    let marker = if name == &config.default.provider {
                        " (primary)"
                    } else if config.default.secondary_provider.as_deref() == Some(name) {
                        " (secondary)"
                    } else if config.default.tertiary_provider.as_deref() == Some(name) {
                        " (tertiary)"
                    } else {
                        ""
                    };
                    println!(
                        "  {name}{marker} — type: {}, model: {}",
                        entry.provider_type,
                        entry.model.as_deref().unwrap_or("-"),
                    );
                }
            }
            Ok(0)
        }

        ConfigCommands::Init => {
            let home = config::mcp_home();
            let config_path = home.join("config.toml");

            if config_path.exists() {
                if json_output {
                    println!("{}", json!({"status": "exists", "path": config_path.display().to_string()}));
                } else {
                    println!("Config already exists at: {}", config_path.display());
                    println!("Use 'config show' to view, or 'config set-provider' to update.");
                }
            } else {
                config::ensure_dirs()?;
                let fresh = config::McpConfig::default();
                config::save_config(&fresh)?;
                if json_output {
                    println!("{}", json!({"status": "created", "path": config_path.display().to_string()}));
                } else {
                    println!("Created default config at: {}", config_path.display());
                    println!();
                    println!("Next steps:");
                    println!("  1. Add a provider:  MasterControlProgram config set-provider nvidia_nim --api-key YOUR_KEY");
                    println!("  2. Pick a model:    MasterControlProgram config models");
                    println!("  3. Set the model:   MasterControlProgram config set-model MODEL_ID");
                    println!("  4. Spawn an agent:  MasterControlProgram spawn \"your task here\"");
                }
            }
            Ok(0)
        }

        ConfigCommands::Validate => {
            let mut errors: Vec<String> = Vec::new();
            let mut warnings: Vec<String> = Vec::new();

            // Check default provider is configured
            if config.provider.is_empty() {
                errors.push("No providers configured. Use 'config set-provider' to add one.".into());
            } else if !config.provider.contains_key(&config.default.provider) {
                errors.push(format!(
                    "Default provider '{}' is not in the provider list. Available: {}",
                    config.default.provider,
                    config.provider.keys().cloned().collect::<Vec<_>>().join(", ")
                ));
            }

            // Check provider entries
            for (name, entry) in &config.provider {
                if entry.api_key.is_none() && entry.provider_type != "bedrock" {
                    warnings.push(format!("Provider '{name}' has no API key configured."));
                }
                if entry.provider_type.is_empty() {
                    errors.push(format!("Provider '{name}' has an empty provider_type."));
                }
            }

            // Check default model
            if config.default.model.is_empty() {
                warnings.push("No default model set.".into());
            }

            // Check default role exists if set
            if let Some(ref role_name) = config.default.role {
                let role_path = config::mcp_home().join("roles").join(format!("{role_name}.toml"));
                if !role_path.exists() {
                    warnings.push(format!("Default role '{role_name}' not found at {}", role_path.display()));
                }
            }

            // Check directories
            let home = config::mcp_home();
            for sub in &["roles", "logs", "tools", "workflows"] {
                if !home.join(sub).exists() {
                    warnings.push(format!("Directory ~/.mastercontrolprogram/{sub} does not exist. Run 'diagnose' to create it."));
                }
            }

            if json_output {
                println!("{}", json!({
                    "valid": errors.is_empty(),
                    "errors": errors,
                    "warnings": warnings,
                }));
            } else if errors.is_empty() && warnings.is_empty() {
                println!("✓ Configuration is valid.");
            } else {
                if !errors.is_empty() {
                    println!("Errors:");
                    for e in &errors {
                        println!("  ✗ {e}");
                    }
                }
                if !warnings.is_empty() {
                    println!("Warnings:");
                    for w in &warnings {
                        println!("  ⚠ {w}");
                    }
                }
                if errors.is_empty() {
                    println!();
                    println!("✓ Configuration is valid (with warnings).");
                }
            }
            Ok(if errors.is_empty() { 0 } else { 1 })
        }

        ConfigCommands::SetDefault { key, value } => {
            let mut new_config = config.clone();
            match key.to_lowercase().as_str() {
                "role" => {
                    new_config.default.role = Some(value.clone());
                    config::save_config(&new_config)?;
                    if json_output {
                        println!("{}", json!({"default_role": value}));
                    } else {
                        println!("Default role set to: {value}");
                    }
                }
                "tool" => {
                    new_config.default.tool = Some(value.clone());
                    config::save_config(&new_config)?;
                    if json_output {
                        println!("{}", json!({"default_tool": value}));
                    } else {
                        println!("Default tool set to: {value}");
                    }
                }
                "secondary-provider" | "secondary_provider" => {
                    new_config.default.secondary_provider = Some(value.clone());
                    config::save_config(&new_config)?;
                    if json_output {
                        println!("{}", json!({"secondary_provider": value}));
                    } else {
                        println!("Secondary provider set to: {value}");
                    }
                }
                "secondary-model" | "secondary_model" => {
                    new_config.default.secondary_model = Some(value.clone());
                    config::save_config(&new_config)?;
                    if json_output {
                        println!("{}", json!({"secondary_model": value}));
                    } else {
                        println!("Secondary model set to: {value}");
                    }
                }
                "tertiary-provider" | "tertiary_provider" => {
                    new_config.default.tertiary_provider = Some(value.clone());
                    config::save_config(&new_config)?;
                    if json_output {
                        println!("{}", json!({"tertiary_provider": value}));
                    } else {
                        println!("Tertiary provider set to: {value}");
                    }
                }
                "tertiary-model" | "tertiary_model" => {
                    new_config.default.tertiary_model = Some(value.clone());
                    config::save_config(&new_config)?;
                    if json_output {
                        println!("{}", json!({"tertiary_model": value}));
                    } else {
                        println!("Tertiary model set to: {value}");
                    }
                }
                "poll-interval" | "poll_interval" | "agent-poll-interval" | "agent_poll_interval" => {
                    let ms: u64 = value.parse().map_err(|_| anyhow::anyhow!("poll-interval must be a number in milliseconds"))?;
                    new_config.limits.agent_poll_interval_ms = ms;
                    config::save_config(&new_config)?;
                    if json_output {
                        println!("{}", json!({"agent_poll_interval_ms": ms}));
                    } else {
                        println!("Agent poll interval set to: {ms}ms");
                    }
                }
                other => {
                    eprintln!(
                        "Unknown default key '{other}'. Valid keys: role, tool, \
                         secondary-provider, secondary-model, tertiary-provider, tertiary-model, poll-interval"
                    );
                    return Ok(1);
                }
            }
            Ok(0)
        }

        ConfigCommands::SetProvider {
            name,
            api_key,
            provider_type,
            url,
            model,
        } => {
            let mut new_config = config.clone();

            // If the provider doesn't exist yet, create it
            if !new_config.provider.contains_key(&name) {
                let inferred_type = provider_type
                    .clone()
                    .unwrap_or_else(|| config::infer_provider_type(&name));
                let effective_url = url.clone().or_else(|| config::infer_provider_url(&inferred_type));
                let entry = config::ProviderEntry {
                    provider_type: inferred_type.clone(),
                    api_key: api_key.clone(),
                    url: effective_url,
                    model: model.clone(),
                    timeout: 300,
                    max_retries: 3,
                    region: None,
                };
                new_config.provider.insert(name.clone(), entry);
                if !json_output {
                    println!("Created provider '{name}' (type: {inferred_type})");
                }
            } else {
                // Update existing provider with any supplied overrides
                let entry = new_config.provider.get_mut(&name).unwrap();
                if let Some(ref key) = api_key {
                    entry.api_key = Some(key.clone());
                }
                if let Some(ref t) = provider_type {
                    entry.provider_type = t.clone();
                }
                if let Some(ref u) = url {
                    entry.url = Some(u.clone());
                }
                if let Some(ref m) = model {
                    entry.model = Some(m.clone());
                }
                if !json_output {
                    println!("Updated provider '{name}'");
                }
            }

            // Set as default provider
            new_config.default.provider = name.clone();

            // Adopt the provider's model as default if available
            if let Some(entry) = new_config.provider.get(&name) {
                if let Some(ref m) = entry.model {
                    new_config.default.model = m.clone();
                }
            }

            config::save_config(&new_config)?;

            if json_output {
                println!(
                    "{}",
                    json!({
                        "default_provider": new_config.default.provider,
                        "default_model": new_config.default.model,
                    })
                );
            } else {
                println!("Default provider set to: {}", new_config.default.provider);
                println!("Default model set to:    {}", new_config.default.model);
            }
            Ok(0)
        }

        ConfigCommands::SetModel { name } => {
            // If the user passed a number, resolve it from the model list
            let resolved_name = if let Ok(num) = name.parse::<usize>() {
                let provider_name = &config.default.provider;
                match manager.list_models(provider_name).await {
                    Ok(models) if num >= 1 && num <= models.len() => {
                        models[num - 1].clone()
                    }
                    Ok(models) => {
                        eprintln!(
                            "Model number {num} out of range (1-{}). Use 'config models' to see the list.",
                            models.len()
                        );
                        return Ok(1);
                    }
                    Err(_) => {
                        eprintln!("Could not fetch model list to resolve number. Use the full model name instead.");
                        return Ok(1);
                    }
                }
            } else {
                name.clone()
            };

            let mut new_config = config.clone();
            new_config.default.model = resolved_name.clone();
            config::save_config(&new_config)?;

            if json_output {
                println!(
                    "{}",
                    json!({"default_model": resolved_name, "default_provider": config.default.provider})
                );
            } else {
                println!("Default model set to: {resolved_name}");
            }
            Ok(0)
        }

        ConfigCommands::Models { provider } => {
            let provider_name = provider.as_deref().unwrap_or(&config.default.provider);

            match manager.list_models(provider_name).await {
                Ok(models) => {
                    if json_output {
                        println!(
                            "{}",
                            json!({"provider": provider_name, "models": models})
                        );
                    } else if models.is_empty() {
                        println!("No models found for provider '{provider_name}'");
                    } else {
                        println!("Models available from '{provider_name}':\n");
                        for (i, m) in models.iter().enumerate() {
                            let marker = if *m == config.default.model {
                                " ← default"
                            } else {
                                ""
                            };
                            println!("  {:>3}. {m}{marker}", i + 1);
                        }
                        println!(
                            "\nUse `MasterControlProgram config set-model <MODEL or NUMBER>` to change the default."
                        );
                    }
                    Ok(0)
                }
                Err(e) => {
                    if json_output {
                        println!(
                            "{}",
                            json!({"provider": provider_name, "error": e.to_string()})
                        );
                    } else {
                        eprintln!("Error listing models for '{provider_name}': {e}");
                    }
                    Ok(1)
                }
            }
        }
    }
}

async fn handle_provider_command(
    cmd: ProviderCommands,
    config: &config::McpConfig,
    manager: &AgentManager,
    json_output: bool,
) -> Result<i32> {
    match cmd {
        ProviderCommands::List => {
            let providers = manager.get_providers().await;
            if json_output {
                let entries: Vec<_> = providers
                    .iter()
                    .filter_map(|name| {
                        config.provider.get(name).map(|e| {
                            json!({
                                "name": name,
                                "type": e.provider_type,
                                "url": e.url,
                                "model": e.model,
                            })
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if providers.is_empty() {
                println!("No providers configured");
            } else {
                println!("{:<20} {:<15} {:<40}", "NAME", "TYPE", "URL");
                println!("{}", "-".repeat(75));
                for name in &providers {
                    if let Some(entry) = config.provider.get(name) {
                        println!(
                            "{:<20} {:<15} {:<40}",
                            name,
                            entry.provider_type,
                            entry.url.as_deref().unwrap_or("-"),
                        );
                    }
                }
            }
            Ok(0)
        }

        ProviderCommands::Show { name, show_secrets } => {
            if let Some(entry) = config.provider.get(&name) {
                let mut val = json!({
                    "name": name,
                    "type": entry.provider_type,
                    "url": entry.url,
                    "model": entry.model,
                    "timeout": entry.timeout,
                    "max_retries": entry.max_retries,
                });
                if show_secrets {
                    val["api_key"] = json!(entry.resolved_api_key());
                } else {
                    val["api_key"] = json!("***REDACTED***");
                }
                println!("{}", serde_json::to_string_pretty(&val)?);
            } else {
                eprintln!("Provider '{name}' not found");
                return Ok(1);
            }
            Ok(0)
        }

        ProviderCommands::Check { name } => match manager.check_provider(&name).await {
            Ok(msg) => {
                if json_output {
                    println!("{}", json!({"provider": name, "status": msg}));
                } else {
                    println!("{msg}");
                }
                Ok(0)
            }
            Err(e) => {
                if json_output {
                    println!("{}", json!({"provider": name, "error": e.to_string()}));
                } else {
                    eprintln!("Error: {e}");
                }
                Ok(1)
            }
        },
    }
}

fn handle_tool_command(cmd: cli::ToolCommands, json_output: bool) -> Result<i32> {
    use cli::ToolCommands;
    match cmd {
        ToolCommands::Register {
            name,
            role,
            workflow: wf_path,
            description: _desc,
        } => {
            let tool_def = if let Some(role_name) = role {
                tool::register_from_role(&name, &role_name)?
            } else if let Some(wf) = wf_path {
                tool::register_from_workflow(&name, &wf)?
            } else {
                anyhow::bail!("Provide --role or --workflow to bind the tool");
            };

            if json_output {
                println!("{}", serde_json::to_string_pretty(&tool_def)?);
            } else {
                println!("Tool '{}' registered", tool_def.name);
                if let Some(ref r) = tool_def.role_binding {
                    println!("  Bound to role: {r}");
                }
                if let Some(ref w) = tool_def.workflow_binding {
                    println!("  Bound to workflow: {w}");
                }
            }
            Ok(0)
        }

        ToolCommands::List => {
            let tools = tool::list_tools()?;
            if json_output {
                let entries: Vec<tool::ToolListEntry> = tools.iter().map(|t| t.into()).collect();
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if tools.is_empty() {
                println!("No tools registered");
            } else {
                println!(
                    "{:<20} {:<30} {:<15} {:<15}",
                    "NAME", "DESCRIPTION", "ROLE", "WORKFLOW"
                );
                println!("{}", "-".repeat(80));
                for t in &tools {
                    let desc = if t.description.len() > 28 {
                        format!("{}…", &t.description[..27])
                    } else {
                        t.description.clone()
                    };
                    println!(
                        "{:<20} {:<30} {:<15} {:<15}",
                        t.name,
                        desc,
                        t.role_binding.as_deref().unwrap_or("-"),
                        t.workflow_binding.as_deref().unwrap_or("-"),
                    );
                }
            }
            Ok(0)
        }

        ToolCommands::Show { name } => {
            let t = tool::get_tool(&name)?;
            println!("{}", serde_json::to_string_pretty(&t)?);
            Ok(0)
        }

        ToolCommands::Delete { name } => {
            tool::delete_tool(&name)?;
            if json_output {
                println!("{}", json!({"deleted": name}));
            } else {
                println!("Tool '{name}' deleted");
            }
            Ok(0)
        }
    }
}

async fn handle_workflow_command(
    cmd: cli::WorkflowCommands,
    manager: &Arc<AgentManager>,
    runner: &Arc<workflow::WorkflowRunner>,
    json_output: bool,
) -> Result<i32> {
    use cli::WorkflowCommands;
    match cmd {
        WorkflowCommands::Run { name } => {
            let wf = workflow::load_workflow(&name)?;
            let run_id = runner.run(wf.clone(), manager.clone()).await?;
            if json_output {
                println!(
                    "{}",
                    json!({"run_id": run_id, "workflow": wf.name, "status": "running"})
                );
            } else {
                println!("Workflow '{}' started (run ID: {run_id})", wf.name);
            }
            Ok(0)
        }

        WorkflowCommands::List => {
            let workflows = workflow::list_workflows()?;
            if json_output {
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
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if workflows.is_empty() {
                println!("No workflows found");
            } else {
                println!("{:<25} {:<8} {:<6} {}", "NAME", "VERSION", "STEPS", "DESCRIPTION");
                println!("{}", "-".repeat(70));
                for w in &workflows {
                    println!(
                        "{:<25} {:<8} {:<6} {}",
                        w.name,
                        w.version,
                        w.steps.len(),
                        w.description,
                    );
                }
            }
            Ok(0)
        }

        WorkflowCommands::Show { name } => {
            let wf = workflow::load_workflow(&name)?;
            println!("{}", serde_json::to_string_pretty(&wf)?);
            Ok(0)
        }

        WorkflowCommands::Status { id } => {
            let info = runner.get_run(id).await?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                println!("Workflow run {}: {}", info.run_id, info.status);
                println!("Workflow: {}", info.workflow_name);
                if let Some(ref step) = info.current_step {
                    println!("Current step: {step}");
                }
                println!("Steps completed: {}", info.step_results.len());
                for r in &info.step_results {
                    let status_marker = if r.status == "ok" { "✓" } else { "✗" };
                    println!("  {status_marker} {} ({})", r.step_id, r.action);
                    if let Some(ref e) = r.error {
                        println!("    Error: {e}");
                    }
                }
            }
            Ok(0)
        }

        WorkflowCommands::Stop { id } => {
            runner.stop(id).await?;
            if json_output {
                println!("{}", json!({"stopped": id}));
            } else {
                println!("Stopped workflow run {id}");
            }
            Ok(0)
        }

        WorkflowCommands::Validate { file } => {
            let text = std::fs::read_to_string(&file)?;
            match workflow::parse_workflow_yaml(&text) {
                Ok(wf) => match workflow::validate_workflow(&wf) {
                    Ok(()) => {
                        if json_output {
                            println!(
                                "{}",
                                json!({"valid": true, "name": wf.name, "steps": wf.steps.len()})
                            );
                        } else {
                            println!(
                                "Workflow '{}' is valid ({} steps)",
                                wf.name,
                                wf.steps.len()
                            );
                        }
                        Ok(0)
                    }
                    Err(e) => {
                        if json_output {
                            println!("{}", json!({"valid": false, "error": e.to_string()}));
                        } else {
                            eprintln!("Validation error: {e}");
                        }
                        Ok(1)
                    }
                },
                Err(e) => {
                    if json_output {
                        println!("{}", json!({"valid": false, "error": e.to_string()}));
                    } else {
                        eprintln!("Parse error: {e}");
                    }
                    Ok(1)
                }
            }
        }
    }
}
