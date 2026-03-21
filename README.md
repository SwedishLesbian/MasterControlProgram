# MasterControlProgram

A CLI-first agent-orchestration system that lets users and agents spawn, monitor, and steer long-running AI subagents. Runs as a single binary on Linux and Windows.

## Features

- **Multi-provider inference** — OpenAI, Anthropic, NVIDIA NIM, HuggingFace, Amazon Bedrock, and any OpenAI-compatible endpoint
- **Role-based agents** — soul.md-style identity files with system prompts, tools, and per-role model defaults
- **Agent lifecycle** — spawn, status, steer (append instructions / patch system prompt), pause, resume, kill
- **Constraint enforcement** — `maxDepth`, `maxChildren`, `maxConcurrentAgents`, per-agent timeouts
- **HTTP server mode** — REST API for agent orchestration, usable as a control plane by other agents
- **JSON-first** — every command supports `--json` for machine-readable output; fully scriptable and non-interactive
- **Cross-platform** — identical CLI, config, and behavior on Linux and Windows

## Installation

Download the latest binary from [Releases](https://github.com/SwedishLesbian/MasterControlProgram/releases):

| Platform | Binary |
|----------|--------|
| Linux (glibc) | `MasterControlProgram-linux-x86_64` |
| Linux (static/musl) | `MasterControlProgram-linux-x86_64-musl` |
| Windows | `MasterControlProgram-windows-x86_64.exe` |

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
cargo build --release
```

## Quick start

### 1. Configure a provider

Create `~/.mcp/config.toml`:

```toml
[default]
provider = "nvidia-nim"
model = "meta/llama-3.1-8b-instruct"

[provider.nvidia-nim]
type = "nvidia-nim"
url = "https://integrate.api.nvidia.com/v1"
model = "meta/llama-3.1-8b-instruct"
api_key = "<env:MCP_NVIDIA_NIM_KEY>"
timeout = 120
max_retries = 2
```

Set your API key:

```bash
export MCP_NVIDIA_NIM_KEY="nvapi-..."
```

### 2. Spawn an agent

```bash
MasterControlProgram spawn "Write a fizzbuzz implementation in Rust"
```

```
Agent started with id 1
Model: meta/llama-3.1-8b-instruct
Provider: nvidia-nim
```

### 3. Check status

```bash
MasterControlProgram status 1
MasterControlProgram status 1 --json
```

### 4. Steer an agent

```bash
MasterControlProgram agent steer 1 "Add unit tests"
MasterControlProgram agent steer 1 --prompt-patch="Always use idiomatic Rust."
```

### 5. List agents

```bash
MasterControlProgram agents list
MasterControlProgram agents list --soul=rust-engineer --json
```

## CLI reference

```
MasterControlProgram <COMMAND> [OPTIONS]

Commands:
  spawn       Spawn a new agent
  status      Get status of an agent
  agent       Agent management (steer, kill, pause, resume)
  agents      List / show agents
  role        Role management (create, list, show, delete, patch)
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

Roles live in `~/.mcp/roles/*.toml` and define agent identity:

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
MasterControlProgram role create coder --from=coder.soul.md --soul=rust-native-engineer
MasterControlProgram spawn --role=coder "Build a REST API"
```

## Tool Registry (v0.1.2+)

Register agents as discoverable tools with input/output schemas:

```bash
# Register a role-bound tool
MasterControlProgram tool register coder_agent --role=coder

# Register a workflow-bound tool
MasterControlProgram tool register build_pipeline --workflow=build.yaml

# List / show / delete
MasterControlProgram tool list --json
MasterControlProgram tool show coder_agent
MasterControlProgram tool delete coder_agent
```

Tools are discoverable via the server at `GET /tools` and included in `GET /mcp-tools`.

## Workflow Engine (v0.1.2+)

Define multi-step YAML workflows in `~/.mcp/workflows/`:

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
MasterControlProgram workflow run build_and_test.yaml
MasterControlProgram workflow status 1
MasterControlProgram workflow stop 1
MasterControlProgram workflow list
MasterControlProgram workflow validate build_and_test.yaml
```

## Server mode

```bash
MasterControlProgram server --bind=127.0.0.1:29999
```

Exposes REST endpoints:

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/spawn` | Spawn agent |
| GET | `/agent/{id}` | Agent status |
| POST | `/agent/{id}/steer` | Steer agent |
| POST | `/agent/{id}/kill` | Kill agent |
| POST | `/agent/{id}/pause` | Pause agent |
| POST | `/agent/{id}/resume` | Resume agent |
| GET | `/agents` | List all agents |
| GET | `/providers` | List providers |
| GET | `/providers/{name}/check` | Provider health |
| GET | `/tools` | List registered tools |
| GET | `/tools/{name}` | Get tool details |
| GET | `/workflows` | List workflows |
| GET | `/workflows/{name}` | Get workflow details |
| POST | `/workflows/run` | Execute a workflow |
| GET | `/workflow-runs/{id}` | Workflow run status |
| POST | `/workflow-runs/{id}/stop` | Stop workflow run |
| GET | `/mcp-tools` | MCP-style tool discovery |

## Provider configuration

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

## License

MIT
