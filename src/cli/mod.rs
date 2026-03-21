use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "MasterControlProgram", about = "Master Control Program – agent orchestration CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Output JSON instead of human-readable text
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Spawn a new agent
    Spawn {
        /// The task description
        task: String,

        /// Role name to use
        #[arg(long)]
        role: Option<String>,

        /// Model to use
        #[arg(long)]
        model: Option<String>,

        /// Provider to use
        #[arg(long)]
        provider: Option<String>,

        /// Current depth (for child agents)
        #[arg(long)]
        depth: Option<u32>,

        /// Max depth for spawning children
        #[arg(long, alias = "max-depth")]
        max_depth: Option<u32>,

        /// Max children per agent
        #[arg(long, alias = "max-children")]
        max_children: Option<u32>,

        /// Soul/identity label
        #[arg(long)]
        soul: Option<String>,

        /// Timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,

        /// System prompt override
        #[arg(long)]
        system_prompt: Option<String>,

        /// Parent agent ID
        #[arg(long)]
        parent: Option<u64>,
    },

    /// Get status of an agent
    Status {
        /// Agent ID
        id: u64,
    },

    /// Agent management commands
    #[command(subcommand)]
    Agent(AgentCommands),

    /// Agent list shorthand
    Agents {
        #[command(subcommand)]
        cmd: AgentsCommands,
    },

    /// Role management
    #[command(subcommand)]
    Role(RoleCommands),

    /// Configuration management
    #[command(subcommand)]
    Config(ConfigCommands),

    /// Provider management
    #[command(subcommand)]
    Provider(ProviderCommands),

    /// Tool registry management
    #[command(subcommand)]
    Tool(ToolCommands),

    /// Workflow management
    #[command(subcommand)]
    Workflow(WorkflowCommands),

    /// Run as HTTP server
    Server {
        /// Bind address
        #[arg(long)]
        bind: Option<String>,

        /// TLS certificate file
        #[arg(long)]
        tls_cert: Option<String>,

        /// TLS key file
        #[arg(long)]
        tls_key: Option<String>,
    },

    /// View agent logs
    Logs {
        /// Agent ID
        id: Option<u64>,

        /// Show logs since duration (e.g., 10m, 1h)
        #[arg(long)]
        since: Option<String>,
    },

    /// System diagnostics
    Diagnose,

    /// Create a shell alias (e.g., 'mcp') for MasterControlProgram
    Alias {
        /// The alias name to create (default: mcp)
        #[arg(default_value = "mcp")]
        name: String,
    },
}

#[derive(Subcommand)]
pub enum AgentCommands {
    /// Steer an agent with new instructions
    Steer {
        /// Agent ID
        id: u64,

        /// Instruction text
        instruction: Option<String>,

        /// Patch the system prompt
        #[arg(long)]
        prompt_patch: Option<String>,
    },

    /// Kill an agent
    Kill {
        /// Agent ID (omit with --all to kill all)
        id: Option<u64>,

        /// Kill all agents
        #[arg(long)]
        all: bool,
    },

    /// Pause an agent
    Pause {
        /// Agent ID
        id: u64,
    },

    /// Resume a paused agent
    Resume {
        /// Agent ID
        id: u64,
    },
}

#[derive(Subcommand)]
pub enum AgentsCommands {
    /// List all agents
    List {
        /// Filter by soul
        #[arg(long)]
        soul: Option<String>,

        /// Filter by role
        #[arg(long)]
        role: Option<String>,
    },

    /// Show details for a specific agent
    Show {
        /// Agent ID
        id: u64,
    },
}

#[derive(Subcommand)]
pub enum RoleCommands {
    /// Create a new role
    Create {
        /// Role name
        name: String,

        /// Create from a file
        #[arg(long)]
        from: Option<String>,

        /// Inline system prompt
        #[arg(long)]
        prompt: Option<String>,

        /// Role tag (e.g., code-gen, code-review)
        #[arg(long)]
        role: Option<String>,

        /// Soul label
        #[arg(long)]
        soul: Option<String>,

        /// Default model
        #[arg(long)]
        model: Option<String>,

        /// Default provider
        #[arg(long)]
        provider: Option<String>,
    },

    /// List all roles
    List,

    /// Show role details
    Show {
        /// Role name
        name: String,
    },

    /// Delete a role
    Delete {
        /// Role name
        name: String,
    },

    /// Patch a role
    Patch {
        /// Role name
        name: String,

        /// Patch the system prompt
        #[arg(long)]
        prompt_patch: Option<String>,

        /// Set model
        #[arg(long)]
        model: Option<String>,

        /// Set provider
        #[arg(long)]
        provider: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ToolCommands {
    /// Register a new tool
    Register {
        /// Tool name
        name: String,

        /// Bind to a role
        #[arg(long)]
        role: Option<String>,

        /// Bind to a workflow file
        #[arg(long)]
        workflow: Option<String>,

        /// Tool description
        #[arg(long)]
        description: Option<String>,
    },

    /// List all registered tools
    List,

    /// Show tool details
    Show {
        /// Tool name
        name: String,
    },

    /// Delete a tool
    Delete {
        /// Tool name
        name: String,
    },
}

#[derive(Subcommand)]
pub enum WorkflowCommands {
    /// Run a workflow
    Run {
        /// Workflow file path or name
        name: String,
    },

    /// List saved workflows
    List,

    /// Show workflow details
    Show {
        /// Workflow name
        name: String,
    },

    /// Get status of a workflow run
    Status {
        /// Workflow run ID
        id: u64,
    },

    /// Stop a running workflow
    Stop {
        /// Workflow run ID
        id: u64,
    },

    /// Validate a workflow file
    Validate {
        /// Workflow file path
        file: String,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Show the current configuration
    Show,

    /// Set the default provider
    SetProvider {
        /// Provider name (must be configured in config.toml)
        name: String,
    },

    /// Set the default model
    SetModel {
        /// Model ID
        name: String,
    },

    /// List available models for a provider (or the default provider)
    Models {
        /// Provider name (defaults to the current default provider)
        #[arg(long)]
        provider: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ProviderCommands {
    /// List all providers
    List,

    /// Show provider config
    Show {
        /// Provider name
        name: String,

        /// Show secrets (API keys)
        #[arg(long)]
        show_secrets: bool,
    },

    /// Check provider health
    Check {
        /// Provider name
        name: String,
    },
}
