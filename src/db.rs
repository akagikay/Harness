use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::config::{HarnessConfig, Project, ServerNodeConfig};
use crate::engine::context::ChatMessage;
use crate::engine::events::HarnessEvent;
use crate::state::TurnCheckpoint;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DbConversationRecord {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub status: String,
    pub model: String,
    pub provider_id: String,
    #[serde(default)]
    pub reasoning_effort: String,
    pub server_id: String,
    pub orchestrator_mode: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct HarnessDb {
    conn: Arc<Mutex<Connection>>,
    /// Master key used to encrypt stored credentials (`app_secrets`).
    key: Arc<crate::crypto::MasterKey>,
}

impl HarnessDb {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create database directory: {}", e))?;
            }
        }
        let conn = Connection::open(path)
            .map_err(|e| format!("Failed to open SQLite database {}: {}", path.display(), e))?;
        let key = crate::crypto::MasterKey::load_or_create(&crate::crypto::db_dir(path))
            .map_err(|e| format!("Failed to initialize credential encryption key: {e}"))?;
        Self::init_conn(conn, key)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open in-memory SQLite database: {}", e))?;
        Self::init_conn(conn, crate::crypto::MasterKey::generate())
    }

    /// Test-only: overwrite the config row with arbitrary (e.g. corrupt) JSON to simulate a
    /// stored-config deserialize failure.
    #[cfg(test)]
    pub fn inject_raw_config_for_test(&self, raw_json: &str) {
        let now = chrono::Local::now().to_rfc3339();
        let conn = self.conn.lock().expect("lock conn");
        conn.execute(
            "INSERT INTO app_config (id, config_json, updated_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET config_json = excluded.config_json, updated_at = excluded.updated_at",
            params![raw_json, now],
        )
        .expect("inject raw config row");
    }

    fn init_conn(conn: Connection, key: crate::crypto::MasterKey) -> Result<Self, String> {
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            -- Reduce I/O + CPU and prevent SQLITE_BUSY under concurrent access.
            PRAGMA busy_timeout = 5000;
            PRAGMA temp_store = MEMORY;
            PRAGMA cache_size = -16000;       -- ~16 MiB page cache
            PRAGMA mmap_size = 268435456;     -- 256 MiB memory-mapped reads
            PRAGMA wal_autocheckpoint = 1000;
            PRAGMA journal_size_limit = 67108864;  -- cap WAL growth at 64 MiB

            CREATE TABLE IF NOT EXISTS app_config (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                config_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            -- Credentials, encrypted with the master key (AES-256-GCM).
            CREATE TABLE IF NOT EXISTS app_secrets (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                ciphertext TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                path TEXT NOT NULL,
                icon TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS servers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                url TEXT NOT NULL,
                api_token TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1,
                is_default INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL DEFAULT '',
                title TEXT NOT NULL,
                status TEXT NOT NULL,
                model TEXT NOT NULL,
                provider_id TEXT NOT NULL DEFAULT '',
                reasoning_effort TEXT NOT NULL DEFAULT '',
                server_id TEXT NOT NULL DEFAULT 'local',
                orchestrator_mode INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_conversations_project
                ON conversations(project_id, updated_at DESC);

            CREATE INDEX IF NOT EXISTS idx_conversations_updated
                ON conversations(updated_at DESC, created_at DESC);

            CREATE TABLE IF NOT EXISTS conversation_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                event_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_conversation_events_session
                ON conversation_events(session_id, seq ASC, id ASC);

            CREATE TABLE IF NOT EXISTS conversation_messages (
                session_id TEXT PRIMARY KEY,
                messages_json TEXT NOT NULL,
                checkpoints_json TEXT NOT NULL,
                next_turn_id INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL
            );
            ",
        )
        .map_err(|e| format!("Failed to initialize SQLite schema: {}", e))?;

        let _ = conn.execute(
            "ALTER TABLE conversations ADD COLUMN reasoning_effort TEXT NOT NULL DEFAULT ''",
            [],
        );

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            key: Arc::new(key),
        })
    }

    /// Persist the config with credentials split out and encrypted.
    ///
    /// `config_json` never contains a secret; the credential blob is encrypted
    /// with the master key into `app_secrets`. Both rows are written in one
    /// transaction so they cannot diverge.
    pub fn save_config(&self, conf: &HarnessConfig) -> Result<(), String> {
        let mut plain = conf.clone();
        let blob = plain.extract_secrets();
        let json = serde_json::to_string(&plain).map_err(|e| e.to_string())?;
        let blob_json = serde_json::to_string(&blob).map_err(|e| e.to_string())?;
        let ciphertext = if blob.is_empty() {
            String::new()
        } else {
            self.key
                .encrypt(&blob_json)
                .map_err(|e| format!("Failed to encrypt credentials: {e}"))?
        };
        let now = chrono::Local::now().to_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO app_config (id, config_json, updated_at)
             VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET config_json = excluded.config_json, updated_at = excluded.updated_at",
            params![json, now],
        )
        .map_err(|e| format!("Failed to save config to DB: {}", e))?;
        if ciphertext.is_empty() {
            tx.execute("DELETE FROM app_secrets WHERE id = 1", [])
                .map_err(|e| format!("Failed to clear credentials: {}", e))?;
        } else {
            tx.execute(
                "INSERT INTO app_secrets (id, ciphertext, updated_at)
                 VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET ciphertext = excluded.ciphertext, updated_at = excluded.updated_at",
                params![ciphertext, now],
            )
            .map_err(|e| format!("Failed to save credentials: {}", e))?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Reclaim storage + refresh planner stats. Safe to run while serving;
    /// called on shutdown and exposed via `POST /api/db/optimize`.
    pub fn optimize(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        // Best-effort; a checkpoint needs no active readers to fully truncate.
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        conn.execute_batch("ANALYZE; PRAGMA optimize; VACUUM;")
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Load the config and re-attach decrypted credentials.
    ///
    /// If a legacy row still holds plaintext secrets (pre-encryption database),
    /// they are migrated: encrypted into `app_secrets` and blanked in
    /// `config_json`. A decryption failure is surfaced as an error rather than
    /// silently returning a config with no credentials.
    pub fn load_config(&self) -> Result<Option<HarnessConfig>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let raw: Option<String> = conn
            .query_row(
                "SELECT config_json FROM app_config WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("Failed to load config from DB: {}", e))?;

        let Some(s) = raw else {
            return Ok(None);
        };
        let mut parsed: HarnessConfig = serde_json::from_str(&s)
            .map_err(|e| format!("Failed to deserialize DB config: {}", e))?;

        let ciphertext: Option<String> = conn
            .query_row("SELECT ciphertext FROM app_secrets WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| format!("Failed to load credentials from DB: {}", e))?;

        if let Some(ct) = ciphertext.filter(|c| !c.trim().is_empty()) {
            let plaintext = self
                .key
                .decrypt(&ct)
                .map_err(|e| format!("Failed to decrypt stored credentials: {e}"))?;
            let blob: crate::secrets::SecretsBlob = serde_json::from_str(&plaintext)
                .map_err(|e| format!("Failed to parse stored credentials: {e}"))?;
            parsed.apply_secrets(&blob);
            return Ok(Some(parsed));
        }

        // No encrypted blob: legacy database (or genuinely no secrets).
        if parsed.has_secrets() {
            let blob = parsed.extract_secrets();
            let blob_json = serde_json::to_string(&blob).map_err(|e| e.to_string())?;
            let ct = self
                .key
                .encrypt(&blob_json)
                .map_err(|e| format!("Failed to encrypt credentials during migration: {e}"))?;
            let blanked = serde_json::to_string(&parsed).map_err(|e| e.to_string())?;
            let now = chrono::Local::now().to_rfc3339();
            drop(conn);
            let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE app_config SET config_json = ?1, updated_at = ?2 WHERE id = 1",
                params![blanked, now],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO app_secrets (id, ciphertext, updated_at) VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET ciphertext = excluded.ciphertext, updated_at = excluded.updated_at",
                params![ct, now],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            eprintln!("[harness-db] migrated stored credentials to encrypted storage");
            parsed.apply_secrets(&blob);
        }
        Ok(Some(parsed))
    }

    pub fn upsert_project(&self, project: &Project) -> Result<(), String> {
        let now = chrono::Local::now().to_rfc3339();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO projects (id, name, path, icon, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               path = excluded.path,
               icon = excluded.icon,
               updated_at = excluded.updated_at",
            params![
                project.id,
                project.name,
                project.path.display().to_string(),
                project.icon,
                project.created_at,
                now
            ],
        )
        .map_err(|e| format!("Failed to upsert project: {}", e))?;
        Ok(())
    }

    pub fn delete_project(&self, id: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let changed = conn
            .execute("DELETE FROM projects WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to delete project: {}", e))?;
        Ok(changed > 0)
    }

    pub fn upsert_server(&self, srv: &ServerNodeConfig) -> Result<(), String> {
        let now = chrono::Local::now().to_rfc3339();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO servers (id, name, url, api_token, enabled, is_default, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               url = excluded.url,
               api_token = excluded.api_token,
               enabled = excluded.enabled,
               is_default = excluded.is_default",
            params![
                srv.id,
                srv.name,
                srv.url,
                srv.api_token,
                i32::from(srv.enabled),
                i32::from(srv.is_default),
                now
            ],
        )
        .map_err(|e| format!("Failed to upsert server: {}", e))?;
        Ok(())
    }

    pub fn delete_server(&self, id: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let changed = conn
            .execute("DELETE FROM servers WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to delete server: {}", e))?;
        Ok(changed > 0)
    }

    pub fn upsert_conversation(&self, conv: &DbConversationRecord) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO conversations (id, project_id, title, status, model, provider_id, reasoning_effort, server_id, orchestrator_mode, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
               project_id = excluded.project_id,
               title = excluded.title,
               status = excluded.status,
               model = excluded.model,
               provider_id = excluded.provider_id,
               reasoning_effort = excluded.reasoning_effort,
               server_id = excluded.server_id,
               orchestrator_mode = excluded.orchestrator_mode,
               updated_at = excluded.updated_at",
            params![
                conv.id,
                conv.project_id,
                conv.title,
                conv.status,
                conv.model,
                conv.provider_id,
                conv.reasoning_effort,
                conv.server_id,
                i32::from(conv.orchestrator_mode),
                conv.created_at,
                conv.updated_at
            ],
        )
        .map_err(|e| format!("Failed to upsert conversation: {}", e))?;
        Ok(())
    }

    pub fn list_conversations(&self) -> Result<Vec<DbConversationRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, title, status, model, provider_id, reasoning_effort, server_id, orchestrator_mode, created_at, updated_at
                 FROM conversations
                 ORDER BY updated_at DESC, created_at DESC
                 LIMIT 100",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |row| {
                let orch: i32 = row.get(8)?;
                Ok(DbConversationRecord {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    title: row.get(2)?,
                    status: row.get(3)?,
                    model: row.get(4)?,
                    provider_id: row.get(5)?,
                    reasoning_effort: row.get(6)?,
                    server_id: row.get(7)?,
                    orchestrator_mode: orch != 0,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut list = Vec::new();
        for row in rows {
            list.push(row.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    pub fn get_conversation(&self, id: &str) -> Result<Option<DbConversationRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT id, project_id, title, status, model, provider_id, reasoning_effort, server_id, orchestrator_mode, created_at, updated_at
             FROM conversations WHERE id = ?1",
            params![id],
            |row| {
                let orch: i32 = row.get(8)?;
                Ok(DbConversationRecord {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    title: row.get(2)?,
                    status: row.get(3)?,
                    model: row.get(4)?,
                    provider_id: row.get(5)?,
                    reasoning_effort: row.get(6)?,
                    server_id: row.get(7)?,
                    orchestrator_mode: orch != 0,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    pub fn delete_conversation(&self, id: &str) -> Result<bool, String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM conversation_events WHERE session_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM conversation_messages WHERE session_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        let changed = tx
            .execute("DELETE FROM conversations WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(changed > 0)
    }

    pub fn reassign_project_conversations(&self, from_project_id: &str, to_project_id: &str) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let changed = conn
            .execute(
                "UPDATE conversations SET project_id = ?1 WHERE project_id = ?2",
                params![to_project_id, from_project_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(changed)
    }

    pub fn replace_events(&self, session_id: &str, events: &[HarnessEvent]) -> Result<(), String> {
        let now = chrono::Local::now().to_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM conversation_events WHERE session_id = ?1",
            params![session_id],
        )
        .map_err(|e| e.to_string())?;

        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO conversation_events (session_id, seq, event_json, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|e| e.to_string())?;
            for (idx, evt) in events.iter().enumerate() {
                let json = serde_json::to_string(evt).map_err(|e| e.to_string())?;
                stmt.execute(params![session_id, idx as i64, json, now])
                    .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load_events(&self, session_id: &str) -> Result<Vec<HarnessEvent>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT event_json FROM conversation_events WHERE session_id = ?1 ORDER BY seq ASC, id ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![session_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut events = Vec::new();
        for row in rows {
            let json = row.map_err(|e| e.to_string())?;
            match serde_json::from_str::<HarnessEvent>(&json) {
                Ok(evt) => events.push(evt),
                Err(e) => {
                    // Do NOT silently swallow corrupt event rows: log so operators can diagnose
                    // schema drift / partial writes while still returning the readable rows.
                    eprintln!(
                        "[harness-db] skipping unparseable event row for session '{}': {} (json prefix: {})",
                        session_id,
                        e,
                        json.chars().take(120).collect::<String>()
                    );
                }
            }
        }
        Ok(events)
    }

    /// Persist a full session snapshot (conversation row + events + messages/checkpoints) in ONE
    /// SQLite transaction so concurrent persists cannot tear or interleave the three writes.
    pub fn persist_session_snapshot(
        &self,
        conv: &DbConversationRecord,
        events: &[HarnessEvent],
        messages: &[ChatMessage],
        checkpoints: &[TurnCheckpoint],
        next_turn_id: usize,
    ) -> Result<(), String> {
        let now = chrono::Local::now().to_rfc3339();
        let msgs_json = serde_json::to_string(messages).map_err(|e| e.to_string())?;
        let cps_json = serde_json::to_string(checkpoints).map_err(|e| e.to_string())?;
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        tx.execute(
            "INSERT INTO conversations (id, project_id, title, status, model, provider_id, reasoning_effort, server_id, orchestrator_mode, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
               project_id = excluded.project_id,
               title = excluded.title,
               status = excluded.status,
               model = excluded.model,
               provider_id = excluded.provider_id,
               reasoning_effort = excluded.reasoning_effort,
               server_id = excluded.server_id,
               orchestrator_mode = excluded.orchestrator_mode,
               updated_at = excluded.updated_at",
            params![
                conv.id,
                conv.project_id,
                conv.title,
                conv.status,
                conv.model,
                conv.provider_id,
                conv.reasoning_effort,
                conv.server_id,
                i32::from(conv.orchestrator_mode),
                conv.created_at,
                conv.updated_at
            ],
        )
        .map_err(|e| format!("Failed to upsert conversation: {}", e))?;

        tx.execute(
            "DELETE FROM conversation_events WHERE session_id = ?1",
            params![conv.id],
        )
        .map_err(|e| e.to_string())?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO conversation_events (session_id, seq, event_json, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|e| e.to_string())?;
            for (idx, evt) in events.iter().enumerate() {
                let json = serde_json::to_string(evt).map_err(|e| e.to_string())?;
                stmt.execute(params![conv.id, idx as i64, json, now])
                    .map_err(|e| e.to_string())?;
            }
        }

        tx.execute(
            "INSERT INTO conversation_messages (session_id, messages_json, checkpoints_json, next_turn_id, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id) DO UPDATE SET
               messages_json = excluded.messages_json,
               checkpoints_json = excluded.checkpoints_json,
               next_turn_id = excluded.next_turn_id,
               updated_at = excluded.updated_at",
            params![conv.id, msgs_json, cps_json, next_turn_id as i64, now],
        )
        .map_err(|e| e.to_string())?;

        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn save_messages_and_checkpoints(
        &self,
        session_id: &str,
        messages: &[ChatMessage],
        checkpoints: &[TurnCheckpoint],
        next_turn_id: usize,
    ) -> Result<(), String> {
        let now = chrono::Local::now().to_rfc3339();
        let msgs_json = serde_json::to_string(messages).map_err(|e| e.to_string())?;
        let cps_json = serde_json::to_string(checkpoints).map_err(|e| e.to_string())?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO conversation_messages (session_id, messages_json, checkpoints_json, next_turn_id, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id) DO UPDATE SET
               messages_json = excluded.messages_json,
               checkpoints_json = excluded.checkpoints_json,
               next_turn_id = excluded.next_turn_id,
               updated_at = excluded.updated_at",
            params![session_id, msgs_json, cps_json, next_turn_id as i64, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load_messages_and_checkpoints(
        &self,
        session_id: &str,
    ) -> Result<Option<(Vec<ChatMessage>, Vec<TurnCheckpoint>, usize)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let row: Option<(String, String, i64)> = conn
            .query_row(
                "SELECT messages_json, checkpoints_json, next_turn_id
                 FROM conversation_messages WHERE session_id = ?1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;

        match row {
            Some((msgs_json, cps_json, turn_id)) => {
                // Propagate parse failures instead of silently zeroing the conversation.
                let msgs: Vec<ChatMessage> = serde_json::from_str(&msgs_json)
                    .map_err(|e| format!("Failed to deserialize messages for session '{}': {}", session_id, e))?;
                let cps: Vec<TurnCheckpoint> = serde_json::from_str(&cps_json)
                    .map_err(|e| format!("Failed to deserialize checkpoints for session '{}': {}", session_id, e))?;
                Ok(Some((msgs, cps, turn_id.max(0) as usize)))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_config_errors_on_corrupt_row_instead_of_returning_defaults() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        db.save_config(&HarnessConfig::default()).unwrap();
        db.inject_raw_config_for_test("{not json");
        let res = db.load_config();
        let err = res.expect_err("corrupt config must surface an error");
        assert!(err.contains("Failed to deserialize DB config"), "unexpected: {}", err);
    }

    fn sample_record() -> DbConversationRecord {
        DbConversationRecord {
            id: "ses_test".to_string(),
            project_id: "harness".to_string(),
            title: "Test".to_string(),
            status: "completed".to_string(),
            model: "mock".to_string(),
            provider_id: "nvidia".to_string(),
            reasoning_effort: "high".to_string(),
            server_id: "local".to_string(),
            orchestrator_mode: false,
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-01T00:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn persist_session_snapshot_writes_all_three_tables() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        let rec = sample_record();
        let events = vec![HarnessEvent::IterationStep { step: 1, max_steps: 5 }];
        let msgs = vec![ChatMessage::user("hello")];
        let cps = vec![TurnCheckpoint {
            turn_id: 1,
            prompt: "hello".to_string(),
            history_index: 0,
            llm_msg_start_idx: 0,
            timestamp: "2026-01-01T00:00:00+00:00".to_string(),
        }];

        db.persist_session_snapshot(&rec, &events, &msgs, &cps, 1)
            .expect("snapshot persist");

        let conv = db.get_conversation("ses_test").unwrap().expect("conv row");
        assert_eq!(conv.title, "Test");
        assert_eq!(conv.status, "completed");
        let loaded_events = db.load_events("ses_test").unwrap();
        assert_eq!(loaded_events.len(), 1);
        let (loaded_msgs, loaded_cps, turn) = db
            .load_messages_and_checkpoints("ses_test")
            .unwrap()
            .expect("messages row");
        assert_eq!(loaded_msgs.len(), 1);
        assert_eq!(loaded_cps.len(), 1);
        assert_eq!(turn, 1);
    }

    #[test]
    fn persist_session_snapshot_atomic_then_overwrite() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        let mut rec = sample_record();
        rec.status = "running".to_string();
        db.persist_session_snapshot(&rec, &[], &[], &[], 0)
            .expect("first snapshot");
        rec.status = "stopped".to_string();
        rec.title = "Updated".to_string();
        db.persist_session_snapshot(&rec, &[], &[], &[], 4)
            .expect("second snapshot");

        let conv = db.get_conversation("ses_test").unwrap().expect("conv row");
        assert_eq!(conv.status, "stopped");
        assert_eq!(conv.title, "Updated");
        let (_, _, turn) = db
            .load_messages_and_checkpoints("ses_test")
            .unwrap()
            .expect("messages row");
        assert_eq!(turn, 4);
    }

    #[test]
    fn load_messages_and_checkpoints_errors_instead_of_zeroing() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        db.upsert_conversation(&sample_record()).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO conversation_messages (session_id, messages_json, checkpoints_json, next_turn_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["ses_test", "{not json", "[]", 0i64, "2026-01-01T00:00:00+00:00"],
            )
            .unwrap();
        }
        let res = db.load_messages_and_checkpoints("ses_test");
        let err = res.expect_err("corrupt messages must surface an error, not default to empty");
        assert!(err.contains("Failed to deserialize messages"), "unexpected error: {}", err);
    }

    #[test]
    fn load_events_keeps_good_rows_when_one_row_is_corrupt() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        db.upsert_conversation(&sample_record()).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO conversation_events (session_id, seq, event_json, created_at)
                 VALUES (?1, 0, ?2, ?3)",
                params![
                    "ses_test",
                    serde_json::to_string(&HarnessEvent::IterationStep { step: 1, max_steps: 5 }).unwrap(),
                    "2026-01-01T00:00:00+00:00"
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO conversation_events (session_id, seq, event_json, created_at)
                 VALUES (?1, 1, ?2, ?3)",
                params!["ses_test", "{corrupt", "2026-01-01T00:00:00+00:00"],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO conversation_events (session_id, seq, event_json, created_at)
                 VALUES (?1, 2, ?2, ?3)",
                params![
                    "ses_test",
                    serde_json::to_string(&HarnessEvent::IterationStep { step: 2, max_steps: 5 }).unwrap(),
                    "2026-01-01T00:00:00+00:00"
                ],
            )
            .unwrap();
        }
        let events = db.load_events("ses_test").expect("good rows survive");
        assert_eq!(events.len(), 2);
        match (&events[0], &events[1]) {
            (
                HarnessEvent::IterationStep { step: 1, .. },
                HarnessEvent::IterationStep { step: 2, .. },
            ) => {}
            other => panic!("unexpected event ordering/content: {:?}", other),
        }
    }
}

#[cfg(test)]
mod encryption_tests {
    use super::*;

    fn temp_db(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("harness-db-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("harness.db");
        (dir, db_path)
    }

    #[test]
    fn legacy_plaintext_config_is_migrated_to_encrypted() {
        let (dir, db_path) = temp_db("legacy");
        {
            let db = HarnessDb::open(&db_path).unwrap();
            let mut legacy = HarnessConfig::default();
            legacy.api_key = "legacy-plaintext-key".to_string();
            let json = serde_json::to_string(&legacy).unwrap();
            assert!(json.contains("legacy-plaintext-key"));
            db.inject_raw_config_for_test(&json);
        }

        // First load migrates and still returns the credential.
        let db = HarnessDb::open(&db_path).unwrap();
        let loaded = db.load_config().unwrap().unwrap();
        assert_eq!(loaded.api_key, "legacy-plaintext-key");
        drop(db);

        let raw = String::from_utf8_lossy(&std::fs::read(&db_path).unwrap()).to_string();
        assert!(
            !raw.contains("legacy-plaintext-key"),
            "plaintext secret must be scrubbed after migration"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_config_never_contains_secret_in_config_json() {
        let (dir, db_path) = temp_db("split");
        let db = HarnessDb::open(&db_path).unwrap();
        let mut cfg = HarnessConfig::default();
        cfg.api_key = "top-secret".to_string();
        cfg.search.brave_api_key = "brave-secret".to_string();
        db.save_config(&cfg).unwrap();

        let conn = db.conn.lock().unwrap();
        let json: String = conn
            .query_row("SELECT config_json FROM app_config WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        let ct: String = conn
            .query_row("SELECT ciphertext FROM app_secrets WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        drop(conn);

        assert!(!json.contains("top-secret"));
        assert!(!json.contains("brave-secret"));
        assert!(ct.starts_with(crate::crypto::CIPHER_PREFIX));
        assert!(!ct.contains("top-secret"));

        let loaded = db.load_config().unwrap().unwrap();
        assert_eq!(loaded.api_key, "top-secret");
        assert_eq!(loaded.search.brave_api_key, "brave-secret");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undecryptable_secrets_surface_an_error() {
        let (dir, db_path) = temp_db("badkey");
        {
            let db = HarnessDb::open(&db_path).unwrap();
            let mut cfg = HarnessConfig::default();
            cfg.api_key = "secret".to_string();
            db.save_config(&cfg).unwrap();
        }
        // Corrupt the ciphertext so authentication fails.
        {
            let db = HarnessDb::open(&db_path).unwrap();
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE app_secrets SET ciphertext = 'v1:AAAA' WHERE id = 1",
                [],
            )
            .unwrap();
        }
        let db = HarnessDb::open(&db_path).unwrap();
        let err = db.load_config().expect_err("tampered ciphertext must not decrypt");
        assert!(err.contains("decrypt"), "unexpected: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}