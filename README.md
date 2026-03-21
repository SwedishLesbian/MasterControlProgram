# MasterControlProgram

[![Release](https://img.shields.io/github/v/release/SwedishLesbian/MasterControlProgram?style=flat-square&color=blue)](https://github.com/SwedishLesbian/MasterControlProgram/releases/latest)
[![Build](https://img.shields.io/github/actions/workflow/status/SwedishLesbian/MasterControlProgram/release.yml?style=flat-square&label=build)](https://github.com/SwedishLesbian/MasterControlProgram/actions/workflows/release.yml)
[![Tests](https://img.shields.io/github/actions/workflow/status/SwedishLesbian/MasterControlProgram/release.yml?style=flat-square&label=tests&color=brightgreen)](https://github.com/SwedishLesbian/MasterControlProgram/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![Platform: Linux](https://img.shields.io/badge/platform-linux-lightgrey?style=flat-square&logo=linux)](https://github.com/SwedishLesbian/MasterControlProgram/releases)
[![Platform: Windows](https://img.shields.io/badge/platform-windows-lightgrey?style=flat-square&logo=windows)](https://github.com/SwedishLesbian/MasterControlProgram/releases)

> CLI-first agent-orchestration system that lets users and agents spawn, monitor, and steer long-running AI subagents. Runs as a single binary on Linux and Windows.

---

## Features

- **Multi-provider inference** — OpenAI, Anthropic, NVIDIA NIM, HuggingFace, Amazon Bedrock, and any OpenAI-compatible endpoint
- **Role-based agents** — soul.md-style identity files with system prompts, tools, and per-role model defaults
- **Agent lifecycle** — spawn, status, steer (append instructions / patch system prompt), pause, resume, kill
- **Tool registry** — expose agents as schema-described, discoverable tools for other agents
- **Workflow engine** — declarative YAML pipelines with spawn, wait, steer, kill, inspect, and summarize steps
- **Constraint enforcement** — `maxDepth`, `maxChildren`, `maxConcurrentAgents`, per-agent timeouts
- **HTTP server mode** — REST API for agent orchestration, usable as a control plane by other agents
- **JSON-first** — every command supports `--json` for machine-readable output; fully scriptable and non-interactive
- **Local tools** — built-in role for safe file read/write/edit, directory listing, and shell commands
- **Cross-platform** — identical CLI, config, and behavior on Linux and Windows

## Installation

Download the latest binary from [**Releases**](https://github.com/SwedishLesbian/MasterControlProgram/releases/latest):

| Platform | Binary | Notes |
|----------|--------|-------|
| ![Linux](https://img.shields.io/badge/-Linux-lightgrey?logo=linux&logoColor=white) | `MasterControlProgram-linux-x86_64` | Dynamically linked (glibc) |
| ![Linux](https://img.shields.io/badge/-Linux-lightgrey?logo=linux&logoColor=white) | `MasterControlProgram-linux-x86_64-musl` | Statically linked (Docker-friendly) |
| ![Windows](https://img.shields.io/badge/-Windows-blue?logo=windows&logoColor=white) | `MasterControlProgram-windows-x86_64.exe` | MSVC toolchain |

```bash
# Linux — download, make executable, optionally alias
chmod +x MasterControlProgram-linux-x86_64
./MasterControlProgram-linux-x86_64 alias mcp
```

```powershell
# Windows — download and optionally alias
.\MasterControlProgram-windows-x86_64.exe alias mcp
```

### Build from source

```bash
git clone https://github.com/SwedishLesbian/MasterControlProgram.git
cd MasterControlProgram
cargo build --release
```

## Quick start

### 1. Configure a provider

```bash
# Initialize config directory and default config
mcp config init

# Add a provider with your API key
mcp config set-provider nvidia_nim --api-key "nvapi-..."

# Browse available models and pick one
mcp config models
mcp config set-model meta/llama-3.1-8b-instruct

# Validate everything is set up correctly
mcp config validate
```

Or create `~/.mastercontrolprogram/config.toml` manually:

```toml
[default]
provider = "nvidia_nim"
model = "meta/llama-3.1-8b-instruct"

[provider.nvidia_nim]
type = "nvidia_nim"
url = "https://integrate.api.nvidia.com/v1"
model = "meta/llama-3.1-8b-instruct"
api_key = "<env:MCP_NVIDIA_NIM_KEY>"
timeout = 120
max_retries = 2
```

### 2. Spawn an agent

```bash
mcp spawn "Write a fizzbuzz implementation in Rust"
```

```
Agent started with id 1
Model: meta/llama-3.1-8b-instruct
Provider: nvidia-nim
```

### 3. Check status

```bash
mcp status 1
mcp status 1 --json
```

### 4. Steer an agent

```bash
mcp agent steer 1 "Add unit tests"
mcp agent steer 1 --prompt-patch="Always use idiomatic Rust."
```

### 5. List agents

```bash
mcp agents list
mcp agents list --soul=rust-engineer --json
```

## CLI reference

```
mcp <COMMAND> [OPTIONS]

Commands:
  spawn       Spawn a new agent
  status      Get status of an agent
  agent       Agent management (steer, kill, pause, resume)
  agents      List / show agents
  role        Role management (create, list, show, delete, patch)
  config      Configuration (init, show, validate, set-provider, set-model, set-default, models)
  tool        Tool registry (register, list, show, delete)
  workflow    Workflow engine (run, list, show, status, stop, validate)
  provider    Provider management (list, show, check)
  server      Run as HTTP server
  logs        View agent logs
  diagnose    System diagnostics
  alias       Create a shell alias for MasterControlProgram
  help        Print help

Global options:
  --json      Output JSON instead of human-readable text
```

## Roles

Roles live in `~/.mastercontrolprogram/roles/*.toml` and define agent identity:

```toml
name = "coder"
soul = "rust-native-engineer"
role = "code-gen"
prompt_file = "coder.soul.md"
default_model = "meta/llama-3.1-8b-instruct"
default_provider = "nvidia-nim"
max_depth = 1
max_children = 3
allowed_tools = ["gen_code", "read_file", "write_file"]
```

```bash
mcp role create coder --from=coder.soul.md --soul=rust-native-engineer
mcp spawn --role=coder "Build a REST API"
```

## Tool Registry

Register agents as discoverable tools with input/output schemas:

```bash
# Register a role-bound tool
mcp tool register coder_agent --role=coder

# Register a workflow-bound tool
mcp tool register build_pipeline --workflow=build.yaml

# List / show / delete
mcp tool list --json
mcp tool show coder_agent
mcp tool delete coder_agent
```

Tools are discoverable via the server at `GET /tools` and included in `GET /mcp-tools`.

## Workflow Engine

Define multi-step YAML workflows in `~/.mastercontrolprogram/workflows/`:

```yaml
name: build_and_test
version: 1
description: Build code, then test it
globals:
  max_depth: 2
  default_role: coder

steps:
  - id: code
    action: spawn
    role: coder
    task: "Write a REST API in Rust"

  - id: wait_code
    action: wait
    agent: code

  - id: steer_code
    action: steer
    agent: code
    instruction: "Add error handling"

  - id: test
    action: spawn
    role: tester
    task: "Write tests for the REST API"

  - id: wait_test
    action: wait
    agent: test

  - id: summary
    action: summarize
    source: [code, test]
```

**Supported actions:** `spawn`, `wait`, `steer`, `kill`, `pause`, `resume`, `inspect`, `summarize`

```bash
mcp workflow run build_and_test.yaml
mcp workflow status 1
mcp workflow stop 1
mcp workflow list
mcp workflow validate build_and_test.yaml
```

## Getting Started with Local Tools

MCP ships with a recommended `local_coder` role and tool schemas that let agents read, write, edit, and organize files on your machine — safely by default.

### 1. Set up provider and role

```bash
# Configure your provider from the CLI
mcp config set-provider nvidia_nim --api-key "nvapi-..."
mcp config set-model meta/llama-3.1-8b-instruct

# Copy the example role into your roles directory
cp examples/roles/local_coder.toml ~/.mastercontrolprogram/roles/

# Set local_coder as the default role
mcp config set-default role local_coder
```

### 3. Register the tool

```bash
mcp tool register coder_agent --role=local_coder
```

After this, `coder_agent` appears in `mcp tool list` and in `GET /mcp-tools` discovery, so other agents can find and invoke it.

### 4. Spawn a local agent

```bash
mcp spawn "Organize the files in ~/projects/myapp into a clean directory structure"
```

The agent uses the `local_coder` role by default (if configured in `config.toml`), which means:
- It can **read**, **write**, **edit** files and **run commands**
- It will **never delete or rename** files without explicit confirmation
- It **prefers read-only** operations when unsure
- All actions are logged to `~/.mastercontrolprogram/logs/` for auditing

### 5. Available sub-tools

The `local_coder` role exposes these tools to the agent:

| Tool | Description | Input |
|------|-------------|-------|
| `read-file` | Read file contents | `{ "path": "string" }` |
| `write-file` | Create or overwrite a file | `{ "path": "string", "contents": "string" }` |
| `edit-file` | Apply a targeted search/replace edit | `{ "path": "string", "search": "string", "replace": "string" }` |
| `list-files` | List directory contents | `{ "path": "string", "recursive": false }` |
| `run-command` | Run a shell command | `{ "command": "string", "cwd": "string" }` |

Full JSON schemas for each tool are in [`examples/tools/`](examples/tools/).

### Recommended config

```toml
[default]
provider = "nvidia-nim"
model = "nvidia/llama-3.1-70b-instruct"
role = "local_coder"      # default role for mcp spawn
tool = "coder_agent"      # default tool for discovery
```

When `role` is set in `[default]`, every `mcp spawn` that doesn't specify `--role` will automatically use `local_coder`. Workflow steps without an explicit role also fall back to this default.

## Server mode

```bash
mcp server --bind=127.0.0.1:29999
```

Exposes REST endpoints:

| Method | Endpoint | Description |
|--------|----------|-------------|
| **Agents** | | |
| POST | `/spawn` | Spawn agent |
| GET | `/agent/{id}` | Agent status |
| POST | `/agent/{id}/steer` | Steer agent |
| POST | `/agent/{id}/kill` | Kill agent |
| POST | `/agent/{id}/pause` | Pause agent |
| POST | `/agent/{id}/resume` | Resume agent |
| GET | `/agents` | List all agents |
| **Providers** | | |
| GET | `/providers` | List providers |
| GET | `/providers/{name}/check` | Provider health |
| **Tools** | | |
| GET | `/tools` | List registered tools |
| GET | `/tools/{name}` | Get tool details |
| **Workflows** | | |
| GET | `/workflows` | List workflows |
| GET | `/workflows/{name}` | Get workflow details |
| POST | `/workflows/run` | Execute a workflow |
| GET | `/workflow-runs/{id}` | Workflow run status |
| POST | `/workflow-runs/{id}/stop` | Stop workflow run |
| **Discovery** | | |
| GET | `/mcp-tools` | MCP-style tool discovery |

## Provider configuration

<details>
<summary><strong>All supported providers</strong></summary>

```toml
[provider.openai]
type = "openai"
url = "https://api.openai.com/v1"
api_key = "<env:OPENAI_API_KEY>"

[provider.anthropic]
type = "anthropic"
url = "https://api.anthropic.com/v1"
api_key = "<env:ANTHROPIC_API_KEY>"

[provider.nvidia-nim]
type = "nvidia-nim"
url = "https://integrate.api.nvidia.com/v1"
api_key = "<env:MCP_NVIDIA_NIM_KEY>"

[provider.huggingface]
type = "huggingface"
url = "https://api-inference.huggingface.co/models"
api_key = "<env:HUGGINGFACE_API_KEY>"

[provider.bedrock]
type = "amazon-bedrock"
region = "us-east-1"

[provider.local]
type = "openai-compatible"
url = "http://localhost:8000/v1"
api_key = "none"
```

</details>

## Architecture

```
src/
├── main.rs          CLI entrypoint + command routing
├── agent/           Agent lifecycle (spawn, run, steer, pause, kill)
├── cli/             CLI command definitions (clap)
├── config/          TOML config loading + env var resolution
├── provider/        Concrete LLM provider implementations
│   ├── openai.rs        OpenAI (+ compatible endpoints)
│   ├── anthropic.rs     Anthropic Messages API
│   ├── nvidia_nim.rs    NVIDIA NIM
│   ├── huggingface.rs   HuggingFace Inference API
│   └── bedrock.rs       Amazon Bedrock (AWS SDK)
├── role/            Role/soul management
├── tool/            Agent-as-tool registry
├── workflow/        YAML workflow engine
├── server/          HTTP REST API (Axum)
└── logging/         Per-agent JSON logs
```

## License

MIT
