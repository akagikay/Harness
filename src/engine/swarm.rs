use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Notify, RwLock};

use crate::engine::events::HarnessEvent;

pub const MAX_BOARD_MESSAGES: usize = 200;
pub const MAX_ARTIFACTS: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmMessage {
    pub seq: usize,
    pub sender: String,
    pub target: Option<String>,
    pub content: String,
    pub timestamp: String,
    #[serde(default)]
    pub job_tag: Option<String>,
    #[serde(default)]
    pub specialist: Option<String>,
}

impl SwarmMessage {
    pub fn to_event(&self) -> HarnessEvent {
        HarnessEvent::SwarmBoardMessage {
            seq: self.seq,
            sender: self.sender.clone(),
            target: self.target.clone(),
            content: self.content.clone(),
            timestamp: self.timestamp.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobTag {
    pub id: String,
    pub title: String,
    pub description: String,
    /// One of: "open", "in_progress", "done".
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialistTag {
    pub id: String,
    pub name: String,
    pub specialty: String,
    #[serde(default)]
    pub default_read_only: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub id: String,
    pub role: String,
    pub task: String,
    pub parent_id: Option<String>,
    /// One of: "running", "waiting", "completed", "stopped", "failed".
    pub status: String,
    pub created_at: String,
    pub summary: String,
    #[serde(default)]
    pub job_tag: Option<String>,
    #[serde(default)]
    pub specialist: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub last_seq: Option<usize>,
}

#[derive(Clone)]
pub struct SubagentHandle {
    pub info: SubagentInfo,
    pub cancel_notify: Arc<Notify>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub path: String,
    pub preview: String,
    pub created_at: String,
}

impl ArtifactRecord {
    pub fn to_event(&self) -> HarnessEvent {
        HarnessEvent::ArtifactCreated {
            id: self.id.clone(),
            title: self.title.clone(),
            kind: self.kind.clone(),
            path: self.path.clone(),
            preview: self.preview.clone(),
        }
    }
}

struct SwarmBoardInner {
    messages: RwLock<VecDeque<SwarmMessage>>,
    next_seq: AtomicUsize,
    state_seq: AtomicUsize,
    title: RwLock<String>,
    msg_tx: broadcast::Sender<SwarmMessage>,
    notify: Notify,
    subagents: RwLock<HashMap<String, SubagentHandle>>,
    artifacts: RwLock<Vec<ArtifactRecord>>,
    jobs: RwLock<Vec<JobTag>>,
    specialists: RwLock<Vec<SpecialistTag>>,
}

/// Lightweight, memory-bounded per-session swarm message board and subagent registry.
#[derive(Clone)]
pub struct SwarmBoard {
    inner: Arc<SwarmBoardInner>,
}

pub type SwarmCoordinator = SwarmBoard;

impl Default for SwarmBoard {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_opt_tag(tag: Option<String>) -> Option<String> {
    tag.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn normalize_job_status(status: &str) -> Option<&'static str> {
    match status.trim().to_ascii_lowercase().as_str() {
        "open" | "todo" | "pending" => Some("open"),
        "in_progress" | "in-progress" | "active" | "running" => Some("in_progress"),
        "done" | "completed" | "closed" => Some("done"),
        _ => None,
    }
}

impl SwarmBoard {
    pub fn new() -> Self {
        let (msg_tx, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(SwarmBoardInner {
                messages: RwLock::new(VecDeque::with_capacity(32)),
                next_seq: AtomicUsize::new(1),
                state_seq: AtomicUsize::new(0),
                title: RwLock::new(String::new()),
                msg_tx,
                notify: Notify::new(),
                subagents: RwLock::new(HashMap::new()),
                artifacts: RwLock::new(Vec::new()),
                jobs: RwLock::new(Vec::new()),
                specialists: RwLock::new(Vec::new()),
            }),
        }
    }

    /// Set a human-readable title for this swarm.
    pub async fn set_title(&self, title: impl Into<String>) -> String {
        let clean: String = title
            .into()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(160)
            .collect();
        let mut lock = self.inner.title.write().await;
        *lock = clean.clone();
        self.inner.state_seq.fetch_add(1, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
        clean
    }

    /// Get the human-readable swarm title (empty string if unset).
    pub async fn get_title(&self) -> String {
        let lock = self.inner.title.read().await;
        lock.clone()
    }

    /// Return the latest assigned message sequence number (`0` if empty).
    pub fn latest_seq(&self) -> usize {
        self.inner.next_seq.load(Ordering::SeqCst).saturating_sub(1)
    }

    /// Post a message to the board, capping retained history at `MAX_BOARD_MESSAGES`.
    pub async fn post_message(
        &self,
        sender: impl Into<String>,
        target: Option<String>,
        content: impl Into<String>,
    ) -> SwarmMessage {
        self.post_message_with_tags(sender, target, content, None, None)
            .await
    }

    /// Post a message to the board with optional `(job)` and `(specialist)` tags.
    pub async fn post_message_with_tags(
        &self,
        sender: impl Into<String>,
        target: Option<String>,
        content: impl Into<String>,
        job_tag: Option<String>,
        specialist: Option<String>,
    ) -> SwarmMessage {
        let seq = self.inner.next_seq.fetch_add(1, Ordering::SeqCst);
        let msg = SwarmMessage {
            seq,
            sender: sender.into(),
            target,
            content: content.into(),
            timestamp: chrono::Local::now().to_rfc3339(),
            job_tag: normalize_opt_tag(job_tag),
            specialist: normalize_opt_tag(specialist),
        };

        {
            let mut lock = self.inner.messages.write().await;
            if lock.len() >= MAX_BOARD_MESSAGES {
                lock.pop_front();
            }
            lock.push_back(msg.clone());
        }

        {
            let mut subs = self.inner.subagents.write().await;
            if let Some(handle) = subs.get_mut(&msg.sender) {
                handle.info.last_seq = Some(msg.seq);
            }
        }

        self.inner.state_seq.fetch_add(1, Ordering::SeqCst);
        let _ = self.inner.msg_tx.send(msg.clone());
        self.inner.notify.notify_waiters();
        msg
    }

    /// Clear retained messages on the board.
    pub async fn clear_messages(&self) {
        let mut lock = self.inner.messages.write().await;
        lock.clear();
    }

    /// Read all retained messages with `seq > since` in chronological order.
    pub async fn read_messages(&self, since: usize) -> Vec<SwarmMessage> {
        let lock = self.inner.messages.read().await;
        lock.iter()
            .filter(|m| m.seq > since)
            .cloned()
            .collect()
    }

    /// Return all currently retained messages on the board.
    pub async fn all_messages(&self) -> Vec<SwarmMessage> {
        self.read_messages(0).await
    }

    /// Subscribe to real-time board messages.
    pub fn subscribe(&self) -> broadcast::Receiver<SwarmMessage> {
        self.inner.msg_tx.subscribe()
    }

    /// Expose the board's wakeup `Notify` handle.
    pub fn notify_handle(&self) -> &Notify {
        &self.inner.notify
    }

    /// Zero-CPU wait until a message after `since` mentions `agent_name` or `all`.
    pub async fn wait_for_mention(&self, agent_name: &str, since: usize) -> Vec<SwarmMessage> {
        // Subscribe before reading to avoid missing a message posted between check and wait.
        let mut rx = self.subscribe();
        let existing = self.read_messages(since).await;
        if existing.iter().any(|m| message_mentions(m, agent_name)) {
            return existing;
        }

        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if msg.seq > since && message_mentions(&msg, agent_name) {
                        return self.read_messages(since).await;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let msgs = self.read_messages(since).await;
                    if msgs.iter().any(|m| message_mentions(m, agent_name)) {
                        return msgs;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return Vec::new(),
            }
        }
    }

    /// Cancellable variant of `wait_for_mention`. Returns `None` if `cancel` fires first.
    pub async fn wait_for_mention_or_cancel(
        &self,
        agent_name: &str,
        since: usize,
        cancel: &Notify,
    ) -> Option<Vec<SwarmMessage>> {
        tokio::select! {
            msgs = self.wait_for_mention(agent_name, since) => Some(msgs),
            _ = cancel.notified() => None,
        }
    }

    /// Zero-CPU wait for board activity (`seq > since`) or subagent state transition,
    /// bounded by `timeout_secs` and cancellable via `cancel`.
    /// Returns `(messages_since, active_subagents_remaining, timed_out)` or `None` if cancelled.
    pub async fn wait_for_activity_or_cancel(
        &self,
        agent_name: &str,
        since: usize,
        mentions_only: bool,
        timeout_secs: u64,
        cancel: &Notify,
    ) -> Option<(Vec<SwarmMessage>, usize, bool)> {
        let mut rx = self.subscribe();
        let initial_terminal = {
            let lock = self.inner.subagents.read().await;
            lock.values()
                .filter(|h| !matches!(h.info.status.as_str(), "running" | "waiting"))
                .count()
        };

        let existing = self.read_messages(since).await;
        let has_hit = if mentions_only {
            existing.iter().any(|m| message_mentions(m, agent_name))
        } else {
            !existing.is_empty()
        };
        if has_hit {
            let active = self.active_subagent_count().await;
            return Some((existing, active, false));
        }

        let deadline = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs.clamp(1, 120)));
        tokio::pin!(deadline);

        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);

            let current_terminal = {
                let lock = self.inner.subagents.read().await;
                lock.values()
                    .filter(|h| !matches!(h.info.status.as_str(), "running" | "waiting"))
                    .count()
            };
            if !mentions_only && current_terminal > initial_terminal {
                let msgs = self.read_messages(since).await;
                let active = self.active_subagent_count().await;
                return Some((msgs, active, false));
            }

            tokio::select! {
                _ = cancel.notified() => return None,
                _ = &mut deadline => {
                    let msgs = self.read_messages(since).await;
                    let active = self.active_subagent_count().await;
                    return Some((msgs, active, true));
                }
                recv_res = rx.recv() => {
                    match recv_res {
                        Ok(msg) => {
                            let matched = msg.seq > since && (!mentions_only || message_mentions(&msg, agent_name));
                            if matched {
                                let msgs = self.read_messages(since).await;
                                let active = self.active_subagent_count().await;
                                return Some((msgs, active, false));
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let msgs = self.read_messages(since).await;
                            let matched = if mentions_only {
                                msgs.iter().any(|m| message_mentions(m, agent_name))
                            } else {
                                !msgs.is_empty()
                            };
                            if matched {
                                let active = self.active_subagent_count().await;
                                return Some((msgs, active, false));
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            let msgs = self.read_messages(since).await;
                            let active = self.active_subagent_count().await;
                            return Some((msgs, active, false));
                        }
                    }
                }
                _ = &mut notified => {
                    let msgs = self.read_messages(since).await;
                    let has_msgs = if mentions_only {
                        msgs.iter().any(|m| message_mentions(m, agent_name))
                    } else {
                        !msgs.is_empty()
                    };
                    let now_terminal = {
                        let lock = self.inner.subagents.read().await;
                        lock.values()
                            .filter(|h| !matches!(h.info.status.as_str(), "running" | "waiting"))
                            .count()
                    };
                    if has_msgs || (!mentions_only && now_terminal > initial_terminal) {
                        let active = self.active_subagent_count().await;
                        return Some((msgs, active, false));
                    }
                }
            }
        }
    }

    /// Zero-CPU wait until ALL currently active (`running` or `waiting`) subagents finish
    /// OR `timeout_secs` elapses OR `cancel` fires.
    /// Returns `(messages_since, active_subagents_remaining, timed_out)` or `None` if cancelled.
    pub async fn wait_for_all_or_cancel(
        &self,
        since: usize,
        timeout_secs: u64,
        cancel: &Notify,
    ) -> Option<(Vec<SwarmMessage>, usize, bool)> {
        let active_now = self.active_subagent_count().await;
        if active_now == 0 {
            let msgs = self.read_messages(since).await;
            return Some((msgs, 0, false));
        }

        let deadline = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs.clamp(1, 120)));
        tokio::pin!(deadline);

        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);

            let active = self.active_subagent_count().await;
            if active == 0 {
                let msgs = self.read_messages(since).await;
                return Some((msgs, 0, false));
            }

            tokio::select! {
                _ = cancel.notified() => return None,
                _ = &mut deadline => {
                    let msgs = self.read_messages(since).await;
                    let remaining = self.active_subagent_count().await;
                    return Some((msgs, remaining, remaining > 0));
                }
                _ = &mut notified => {
                    let remaining = self.active_subagent_count().await;
                    if remaining == 0 {
                        let msgs = self.read_messages(since).await;
                        return Some((msgs, 0, false));
                    }
                }
            }
        }
    }

    /// Register a newly spawned subagent with initial status `"running"`.
    pub async fn register_subagent(
        &self,
        id: impl Into<String>,
        role: impl Into<String>,
        task: impl Into<String>,
        parent_id: Option<String>,
    ) -> SubagentHandle {
        self.register_subagent_with_tags(id, role, task, parent_id, None, None)
            .await
    }

    /// Register a newly spawned subagent with optional `(job)` and `(specialist)` tags.
    pub async fn register_subagent_with_tags(
        &self,
        id: impl Into<String>,
        role: impl Into<String>,
        task: impl Into<String>,
        parent_id: Option<String>,
        job_tag: Option<String>,
        specialist: Option<String>,
    ) -> SubagentHandle {
        let id_str = id.into();
        let handle = SubagentHandle {
            info: SubagentInfo {
                id: id_str.clone(),
                role: role.into(),
                task: task.into(),
                parent_id,
                status: "running".to_string(),
                created_at: chrono::Local::now().to_rfc3339(),
                summary: String::new(),
                job_tag: normalize_opt_tag(job_tag),
                specialist: normalize_opt_tag(specialist),
                model: None,
                server_id: None,
                last_seq: None,
            },
            cancel_notify: Arc::new(Notify::new()),
        };

        let mut lock = self.inner.subagents.write().await;
        lock.insert(id_str, handle.clone());
        self.inner.state_seq.fetch_add(1, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
        handle
    }

    /// Set the resolved model identifier for a registered subagent.
    pub async fn set_subagent_model(&self, id: &str, model: Option<String>) -> bool {
        let clean = normalize_opt_tag(model);
        let mut lock = self.inner.subagents.write().await;
        if let Some(handle) = lock.get_mut(id) {
            handle.info.model = clean;
            true
        } else {
            false
        }
    }

    /// Set the target execution server identifier for a registered subagent.
    pub async fn set_subagent_server(&self, id: &str, server_id: Option<String>) -> bool {
        let clean = normalize_opt_tag(server_id);
        let mut lock = self.inner.subagents.write().await;
        if let Some(handle) = lock.get_mut(id) {
            handle.info.server_id = clean;
            true
        } else {
            false
        }
    }

    /// Assign or update `(job)` and/or `(specialist)` tags on an existing subagent.
    pub async fn assign_subagent_tags(
        &self,
        id: &str,
        job_tag: Option<String>,
        specialist: Option<String>,
    ) -> bool {
        let mut lock = self.inner.subagents.write().await;
        if let Some(handle) = lock.get_mut(id) {
            if let Some(jt) = normalize_opt_tag(job_tag) {
                handle.info.job_tag = Some(jt);
            }
            if let Some(sp) = normalize_opt_tag(specialist) {
                handle.info.specialist = Some(sp);
            }
            true
        } else {
            false
        }
    }

    /// Create or update a `(job)` tag tracked on this swarm board.
    pub async fn create_or_update_job(
        &self,
        id: String,
        title: String,
        description: String,
        status: Option<String>,
    ) -> JobTag {
        let clean_id = {
            let trimmed = id.trim();
            if trimmed.is_empty() {
                "job".to_string()
            } else {
                trimmed.to_string()
            }
        };
        let clean_title = title.trim().to_string();
        let clean_desc = description.trim().to_string();
        let parsed_status = status
            .as_deref()
            .and_then(normalize_job_status)
            .map(str::to_string);

        let mut lock = self.inner.jobs.write().await;
        if let Some(existing) = lock.iter_mut().find(|j| j.id.eq_ignore_ascii_case(&clean_id)) {
            if !clean_title.is_empty() {
                existing.title = clean_title;
            }
            if !clean_desc.is_empty() {
                existing.description = clean_desc;
            }
            if let Some(st) = parsed_status {
                existing.status = st;
            }
            return existing.clone();
        }

        let job = JobTag {
            id: clean_id.clone(),
            title: if clean_title.is_empty() {
                clean_id
            } else {
                clean_title
            },
            description: clean_desc,
            status: parsed_status.unwrap_or_else(|| "open".to_string()),
            created_at: chrono::Local::now().to_rfc3339(),
        };
        lock.push(job.clone());
        job
    }

    /// Look up a `(job)` tag by ID (case-insensitive).
    pub async fn get_job(&self, id: &str) -> Option<JobTag> {
        let needle = id.trim();
        if needle.is_empty() {
            return None;
        }
        let lock = self.inner.jobs.read().await;
        lock.iter()
            .find(|j| j.id.eq_ignore_ascii_case(needle))
            .cloned()
    }

    /// List all `(job)` tags defined on this swarm board.
    pub async fn list_jobs(&self) -> Vec<JobTag> {
        let lock = self.inner.jobs.read().await;
        lock.clone()
    }

    /// Create or update a `(member specialist)` tag tracked on this swarm board.
    pub async fn create_or_update_specialist(
        &self,
        id: String,
        name: String,
        specialty: String,
        default_read_only: bool,
    ) -> SpecialistTag {
        let clean_id = {
            let trimmed = id.trim();
            if trimmed.is_empty() {
                "specialist".to_string()
            } else {
                trimmed.to_string()
            }
        };
        let clean_name = name.trim().to_string();
        let clean_specialty = specialty.trim().to_string();

        let mut lock = self.inner.specialists.write().await;
        if let Some(existing) = lock
            .iter_mut()
            .find(|s| s.id.eq_ignore_ascii_case(&clean_id) || (!clean_name.is_empty() && s.name.eq_ignore_ascii_case(&clean_name)))
        {
            if !clean_name.is_empty() {
                existing.name = clean_name;
            }
            if !clean_specialty.is_empty() {
                existing.specialty = clean_specialty;
            }
            existing.default_read_only = default_read_only;
            return existing.clone();
        }

        let spec = SpecialistTag {
            id: clean_id.clone(),
            name: if clean_name.is_empty() {
                clean_id
            } else {
                clean_name
            },
            specialty: clean_specialty,
            default_read_only,
            created_at: chrono::Local::now().to_rfc3339(),
        };
        lock.push(spec.clone());
        spec
    }

    /// List all `(member specialist)` tags defined on this swarm board.
    pub async fn list_specialists(&self) -> Vec<SpecialistTag> {
        let lock = self.inner.specialists.read().await;
        lock.clone()
    }

    /// Look up a `(member specialist)` tag by ID or display name (case-insensitive).
    pub async fn get_specialist(&self, id_or_name: &str) -> Option<SpecialistTag> {
        let needle = id_or_name.trim();
        if needle.is_empty() {
            return None;
        }
        let lock = self.inner.specialists.read().await;
        lock.iter()
            .find(|s| s.id.eq_ignore_ascii_case(needle) || s.name.eq_ignore_ascii_case(needle))
            .cloned()
    }

    /// Update an existing subagent's status and optional summary.
    pub async fn update_subagent_status(
        &self,
        id: &str,
        status: &str,
        summary: Option<String>,
    ) -> bool {
        let mut lock = self.inner.subagents.write().await;
        if let Some(handle) = lock.get_mut(id) {
            handle.info.status = status.to_string();
            if let Some(s) = summary {
                handle.info.summary = s;
            }
            drop(lock);
            self.inner.state_seq.fetch_add(1, Ordering::SeqCst);
            self.inner.notify.notify_waiters();
            true
        } else {
            false
        }
    }

    /// Look up a subagent handle by ID.
    pub async fn get_subagent(&self, id: &str) -> Option<SubagentHandle> {
        let lock = self.inner.subagents.read().await;
        lock.get(id).cloned()
    }

    /// List all subagents registered on this board, ordered by creation timestamp.
    pub async fn list_subagents(&self) -> Vec<SubagentInfo> {
        let lock = self.inner.subagents.read().await;
        let mut list: Vec<SubagentInfo> = lock.values().map(|h| h.info.clone()).collect();
        list.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
        list
    }

    /// Signal cancellation for a single subagent and mark active ones as `"stopped"`.
    pub async fn cancel_subagent(&self, id: &str) -> bool {
        let mut lock = self.inner.subagents.write().await;
        if let Some(handle) = lock.get_mut(id) {
            handle.cancel_notify.notify_one();
            if matches!(handle.info.status.as_str(), "running" | "waiting") {
                handle.info.status = "stopped".to_string();
            }
            drop(lock);
            self.inner.state_seq.fetch_add(1, Ordering::SeqCst);
            self.inner.notify.notify_waiters();
            true
        } else {
            false
        }
    }

    /// Cancel every active subagent on this board.
    pub async fn cancel_all_subagents(&self) {
        let mut lock = self.inner.subagents.write().await;
        for handle in lock.values_mut() {
            handle.cancel_notify.notify_one();
            if matches!(handle.info.status.as_str(), "running" | "waiting") {
                handle.info.status = "stopped".to_string();
            }
        }
        drop(lock);
        self.inner.state_seq.fetch_add(1, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    /// Count subagents currently `"running"` or `"waiting"`.
    pub async fn active_subagent_count(&self) -> usize {
        let lock = self.inner.subagents.read().await;
        lock.values()
            .filter(|h| matches!(h.info.status.as_str(), "running" | "waiting"))
            .count()
    }

    /// Record an artifact created during the session (bounded at `MAX_ARTIFACTS`).
    pub async fn add_artifact(
        &self,
        id: impl Into<String>,
        title: impl Into<String>,
        kind: impl Into<String>,
        path: impl Into<String>,
        preview: impl Into<String>,
    ) -> ArtifactRecord {
        let record = ArtifactRecord {
            id: id.into(),
            title: title.into(),
            kind: kind.into(),
            path: path.into(),
            preview: preview.into(),
            created_at: chrono::Local::now().to_rfc3339(),
        };

        let mut lock = self.inner.artifacts.write().await;
        if lock.len() >= MAX_ARTIFACTS {
            lock.remove(0);
        }
        lock.push(record.clone());
        record
    }

    /// List artifacts recorded in this session.
    pub async fn list_artifacts(&self) -> Vec<ArtifactRecord> {
        let lock = self.inner.artifacts.read().await;
        lock.clone()
    }
}

fn message_mentions(msg: &SwarmMessage, agent_name: &str) -> bool {
    if let Some(target) = msg.target.as_deref() {
        if target.eq_ignore_ascii_case(agent_name) || target.eq_ignore_ascii_case("all") {
            return true;
        }
    }
    contains_at_mention(&msg.content, agent_name) || contains_at_mention(&msg.content, "all")
}

fn contains_at_mention(content: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let needle = format!("@{}", name);
    let mut search_from = 0;
    while let Some(rel_idx) = content[search_from..].find(&needle) {
        let start = search_from + rel_idx;
        let preceded_ok = start == 0
            || !content[..start]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric() || c == '_' || c == '-')
                .unwrap_or(false);
        let after = start + needle.len();
        let followed_ok = after >= content.len()
            || !content[after..]
                .chars()
                .next()
                .map(|c| c.is_alphanumeric() || c == '_' || c == '-')
                .unwrap_or(false);
        if preceded_ok && followed_ok {
            return true;
        }
        search_from = start + 1;
    }
    false
}
