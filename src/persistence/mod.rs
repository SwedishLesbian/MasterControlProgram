use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::sync::Mutex;

use crate::agent::{AgentInfo, AgentStatus};
use crate::provider::ChatMessage;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open (or create) the database at ~/.mastercontrolprogram/mcp.db.
    pub fn open() -> Result<Self> {
        let db_path = crate::config::mcp_home().join("mcp.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

        // Enable WAL mode for better concurrent access
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        db.cleanup_stale_agents()?;
        Ok(db)
    }

    /// Create tables if they don't exist.
    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS agents (
                id              INTEGER PRIMARY KEY,
                task            TEXT NOT NULL,
                soul            TEXT,
                role            TEXT,
                model           TEXT NOT NULL,
                provider        TEXT NOT NULL,
                status          TEXT NOT NULL DEFAULT 'queued',
                phase           TEXT,
                progress        REAL NOT NULL DEFAULT 0.0,
                max_depth       INTEGER NOT NULL DEFAULT 2,
                max_children    INTEGER NOT NULL DEFAULT 5,
                depth           INTEGER NOT NULL DEFAULT 0,
                parent_id       INTEGER,
                system_prompt   TEXT NOT NULL DEFAULT '',
                last_output     TEXT,
                last_output_tokens INTEGER,
                timeout_sec     INTEGER NOT NULL DEFAULT 600,
                error           TEXT,
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL,
                FOREIGN KEY (parent_id) REFERENCES agents(id)
            );

            CREATE TABLE IF NOT EXISTS agent_messages (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id    INTEGER NOT NULL,
                seq         INTEGER NOT NULL,
                role        TEXT NOT NULL,
                content     TEXT NOT NULL,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (agent_id) REFERENCES agents(id)
            );

            CREATE INDEX IF NOT EXISTS idx_messages_agent ON agent_messages(agent_id, seq);
            ",
        )?;
        Ok(())
    }

    /// Mark any agents left as 'running'/'queued'/'paused' from a previous
    /// process as 'failed' (the host process died).
    fn cleanup_stale_agents(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE agents SET status = 'failed', phase = 'process-died',
             error = 'Host process terminated unexpectedly',
             updated_at = ?1
             WHERE status IN ('running', 'queued', 'paused')",
            params![Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Get the next available agent ID.
    pub fn next_agent_id(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let max_id: Option<u64> =
            conn.query_row("SELECT MAX(id) FROM agents", [], |row| row.get(0))?;
        Ok(max_id.unwrap_or(0) + 1)
    }

    /// Insert a new agent record.
    pub fn insert_agent(&self, info: &AgentInfo) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO agents (id, task, soul, role, model, provider, status, phase, progress,
             max_depth, max_children, depth, parent_id, system_prompt, last_output,
             last_output_tokens, timeout_sec, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                info.id,
                info.task,
                info.soul,
                info.role,
                info.model,
                info.provider,
                info.status.to_string(),
                info.phase,
                info.progress,
                info.max_depth,
                info.max_children,
                info.depth,
                info.parent_id,
                info.system_prompt,
                info.last_output,
                info.last_output_tokens.map(|t| t as i64),
                info.timeout_sec as i64,
                info.created_at.to_rfc3339(),
                now,
            ],
        )?;

        // Insert initial messages
        for (seq, msg) in info.messages.iter().enumerate() {
            conn.execute(
                "INSERT INTO agent_messages (agent_id, seq, role, content) VALUES (?1, ?2, ?3, ?4)",
                params![info.id, seq as i64, msg.role, msg.content],
            )?;
        }

        Ok(())
    }

    /// Update agent status and related fields.
    pub fn update_agent_status(
        &self,
        id: u64,
        status: &AgentStatus,
        phase: Option<&str>,
        progress: f64,
        last_output: Option<&str>,
        last_output_tokens: Option<u64>,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE agents SET status = ?1, phase = ?2, progress = ?3,
             last_output = ?4, last_output_tokens = ?5, error = ?6, updated_at = ?7
             WHERE id = ?8",
            params![
                status.to_string(),
                phase,
                progress,
                last_output,
                last_output_tokens.map(|t| t as i64),
                error,
                Utc::now().to_rfc3339(),
                id,
            ],
        )?;
        Ok(())
    }

    /// Update the system prompt for an agent.
    pub fn update_system_prompt(&self, id: u64, prompt: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE agents SET system_prompt = ?1, updated_at = ?2 WHERE id = ?3",
            params![prompt, Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    /// Insert a message into the conversation history.
    pub fn insert_message(&self, agent_id: u64, seq: usize, msg: &ChatMessage) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_messages (agent_id, seq, role, content) VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, seq as i64, msg.role, msg.content],
        )?;
        Ok(())
    }

    /// Get a single agent by ID.
    pub fn get_agent(&self, id: u64) -> Result<Option<AgentInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task, soul, role, model, provider, status, phase, progress,
             max_depth, max_children, depth, parent_id, system_prompt, last_output,
             last_output_tokens, timeout_sec, created_at, error
             FROM agents WHERE id = ?1",
        )?;

        let mut rows = stmt.query(params![id])?;
        let row = match rows.next()? {
            Some(r) => r,
            None => return Ok(None),
        };

        let info = self.row_to_agent_info(row, &conn)?;
        Ok(Some(info))
    }

    /// List all agents, optionally filtered by soul or role.
    pub fn list_agents(
        &self,
        soul_filter: Option<&str>,
        role_filter: Option<&str>,
    ) -> Result<Vec<AgentInfo>> {
        let conn = self.conn.lock().unwrap();

        let mut sql = String::from(
            "SELECT id, task, soul, role, model, provider, status, phase, progress,
             max_depth, max_children, depth, parent_id, system_prompt, last_output,
             last_output_tokens, timeout_sec, created_at, error
             FROM agents WHERE 1=1",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(soul) = soul_filter {
            sql.push_str(" AND soul = ?");
            param_values.push(Box::new(soul.to_string()));
        }
        if let Some(role) = role_filter {
            sql.push_str(" AND role = ?");
            param_values.push(Box::new(role.to_string()));
        }
        sql.push_str(" ORDER BY id");

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(params_refs.as_slice())?;

        let mut agents = Vec::new();
        while let Some(row) = rows.next()? {
            agents.push(self.row_to_agent_info(row, &conn)?);
        }
        Ok(agents)
    }

    /// Get conversation messages for an agent.
    pub fn get_messages(&self, agent_id: u64) -> Result<Vec<ChatMessage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT role, content FROM agent_messages WHERE agent_id = ?1 ORDER BY seq",
        )?;
        let messages = stmt
            .query_map(params![agent_id], |row| {
                Ok(ChatMessage {
                    role: row.get(0)?,
                    content: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(messages)
    }

    fn row_to_agent_info(
        &self,
        row: &rusqlite::Row,
        conn: &Connection,
    ) -> Result<AgentInfo> {
        let id: u64 = row.get(0)?;
        let status_str: String = row.get(6)?;
        let created_str: String = row.get(17)?;
        let timeout_sec: i64 = row.get(16)?;

        let status = parse_status(&status_str);
        let created_at = DateTime::parse_from_rfc3339(&created_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let elapsed = (Utc::now() - created_at).num_seconds().max(0) as u64;
        let timeout_remaining = (timeout_sec as u64).saturating_sub(elapsed);

        // Load children
        let mut child_stmt = conn.prepare("SELECT id FROM agents WHERE parent_id = ?1")?;
        let children: Vec<u64> = child_stmt
            .query_map(params![id], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        // Load messages
        let mut msg_stmt = conn.prepare(
            "SELECT role, content FROM agent_messages WHERE agent_id = ?1 ORDER BY seq",
        )?;
        let messages: Vec<ChatMessage> = msg_stmt
            .query_map(params![id], |r| {
                Ok(ChatMessage {
                    role: r.get(0)?,
                    content: r.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        let last_output_tokens: Option<i64> = row.get(15)?;
        let error: Option<String> = row.get(18)?;

        Ok(AgentInfo {
            id,
            task: row.get(1)?,
            soul: row.get(2)?,
            role: row.get(3)?,
            model: row.get(4)?,
            provider: row.get(5)?,
            status,
            phase: row.get(7)?,
            progress: row.get(8)?,
            max_depth: row.get(9)?,
            max_children: row.get(10)?,
            depth: row.get(11)?,
            parent_id: row.get(12)?,
            children,
            system_prompt: row.get(13)?,
            messages,
            last_output: row.get::<_, Option<String>>(14)?
                .or(error),
            last_output_tokens: last_output_tokens.map(|t| t as u64),
            timeout_sec: timeout_sec as u64,
            created_at,
            timeout_remaining_sec: Some(timeout_remaining),
        })
    }

    /// Open an in-memory database (for testing).
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .with_context(|| "Failed to open in-memory database")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }
}

fn parse_status(s: &str) -> AgentStatus {
    match s {
        "queued" => AgentStatus::Queued,
        "running" => AgentStatus::Running,
        "waiting-on-user" => AgentStatus::WaitingOnUser,
        "completed" => AgentStatus::Completed,
        "failed" => AgentStatus::Failed,
        "killed" => AgentStatus::Killed,
        "paused" => AgentStatus::Paused,
        _ => AgentStatus::Failed,
    }
}
