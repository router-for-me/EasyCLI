use base64::Engine;
use chrono::Utc;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, ToSql};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

pub(crate) mod context;

const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: usize = 100;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const REPAIR_BACKUP_KEEP_COUNT: usize = 5;
const REPAIR_PROGRESS_EVENT: &str = "codex-session-repair-progress";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const REFERENCE_TABLE_COLUMNS: [(&str, &str); 12] = [
    ("threads", "id"),
    ("local_thread_catalog", "thread_id"),
    ("automation_runs", "thread_id"),
    ("inbox_items", "thread_id"),
    ("sessions", "id"),
    ("messages", "session_id"),
    ("thread_dynamic_tools", "thread_id"),
    ("thread_goals", "thread_id"),
    ("thread_spawn_edges", "parent_thread_id"),
    ("thread_spawn_edges", "child_thread_id"),
    ("stage1_outputs", "thread_id"),
    ("agent_job_items", "assigned_thread_id"),
];

static CODEX_SESSION_WRITE_LOCK: Mutex<()> = Mutex::new(());
static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListCodexSessionsRequest {
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_page_size")]
    limit: usize,
}

fn default_page_size() -> usize {
    DEFAULT_PAGE_SIZE
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSessionSummary {
    id: String,
    title: String,
    cwd: String,
    model_provider: String,
    archived: bool,
    updated_at_ms: Option<i64>,
    database_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSessionPage {
    codex_home: String,
    database_paths: Vec<String>,
    sessions: Vec<CodexSessionSummary>,
    total_count: usize,
    offset: usize,
    limit: usize,
    has_more: bool,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteCodexSessionsRequest {
    session_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSessionDeleteResult {
    session_id: String,
    status: String,
    message: String,
    backup_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSessionDeleteBatchResult {
    results: Vec<CodexSessionDeleteResult>,
    deleted_count: usize,
    failed_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSessionRepairProgress {
    phase: String,
    percent: u8,
    processed: usize,
    total: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSessionRepairResult {
    target_provider: String,
    changed_rollout_files: usize,
    sqlite_rows_updated: usize,
    skipped_locked_files: Vec<String>,
    backup_path: Option<String>,
    encrypted_content_warning: Option<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionIndexCleanupCandidate {
    id: String,
    thread_name: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionIndexCleanupPreview {
    snapshot_sha256: String,
    candidates: Vec<SessionIndexCleanupCandidate>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplySessionIndexCleanupRequest {
    snapshot_sha256: String,
    thread_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionIndexCleanupResult {
    pruned_entries: usize,
    backup_path: Option<String>,
}

#[derive(Debug)]
struct DatabaseDeletePlan {
    path: PathBuf,
    tables: Map<String, Value>,
    rollout_paths: Vec<PathBuf>,
    has_thread_row: bool,
    has_automation_rows: bool,
}

#[derive(Debug, Clone)]
struct RolloutRepair {
    path: PathBuf,
    original: Vec<u8>,
    next: Vec<u8>,
    original_mtime: Option<SystemTime>,
    thread_id: Option<String>,
    cwd: Option<String>,
    has_user_event: bool,
    providers: Vec<String>,
    encrypted_content: bool,
    changed: bool,
}

#[derive(Debug)]
struct SessionIndexPlan {
    path: PathBuf,
    original: Vec<u8>,
    original_text: String,
    snapshot_sha256: String,
    candidates: Vec<SessionIndexCleanupCandidate>,
}

#[derive(Debug, Clone)]
struct SessionIndexMetadata {
    title: String,
    updated_at_ms: Option<i64>,
}

#[tauri::command]
pub(crate) async fn list_codex_sessions(
    app: tauri::AppHandle,
    request: Option<ListCodexSessionsRequest>,
) -> Result<CodexSessionPage, String> {
    let user_home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let codex_home = resolve_codex_home(&user_home);
    let request = request.unwrap_or(ListCodexSessionsRequest {
        offset: 0,
        limit: DEFAULT_PAGE_SIZE,
    });
    tauri::async_runtime::spawn_blocking(move || {
        list_codex_sessions_from_home(&codex_home, request.offset, request.limit)
    })
    .await
    .map_err(|error| format!("Failed to read Codex session task: {error}"))?
}

#[tauri::command]
pub(crate) async fn delete_codex_sessions(
    app: tauri::AppHandle,
    request: DeleteCodexSessionsRequest,
) -> Result<CodexSessionDeleteBatchResult, String> {
    let user_home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let codex_home = resolve_codex_home(&user_home);
    tauri::async_runtime::spawn_blocking(move || {
        delete_codex_sessions_from_home(&codex_home, request.session_ids)
    })
    .await
    .map_err(|error| format!("Failed to delete Codex session task: {error}"))?
}

#[tauri::command]
pub(crate) async fn repair_codex_session_metadata(
    app: tauri::AppHandle,
) -> Result<CodexSessionRepairResult, String> {
    let user_home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let codex_home = resolve_codex_home(&user_home);
    let progress_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let target_provider = resolve_current_codex_provider(&codex_home)?;
        repair_codex_session_metadata_from_home(&codex_home, &target_provider, |progress| {
            let _ = progress_app.emit(REPAIR_PROGRESS_EVENT, progress);
        })
    })
    .await
    .map_err(|error| format!("Failed to restore Codex historical session task: {error}"))?
}

#[tauri::command]
pub(crate) async fn preview_codex_session_index_cleanup(
    app: tauri::AppHandle,
) -> Result<SessionIndexCleanupPreview, String> {
    let user_home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let codex_home = resolve_codex_home(&user_home);
    tauri::async_runtime::spawn_blocking(move || {
        preview_session_index_cleanup_from_home(&codex_home)
    })
    .await
    .map_err(|error| format!("Failed to preview Codex session index task: {error}"))?
}

#[tauri::command]
pub(crate) async fn apply_codex_session_index_cleanup(
    app: tauri::AppHandle,
    request: ApplySessionIndexCleanupRequest,
) -> Result<SessionIndexCleanupResult, String> {
    let user_home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let codex_home = resolve_codex_home(&user_home);
    tauri::async_runtime::spawn_blocking(move || {
        apply_session_index_cleanup_from_home(
            &codex_home,
            &request.snapshot_sha256,
            &request.thread_ids,
            true,
        )
    })
    .await
    .map_err(|error| format!("Failed to clean up Codex session index task: {error}"))?
}

fn resolve_codex_home(user_home: &Path) -> PathBuf {
    existing_env_directory("CODEX_HOME").unwrap_or_else(|| user_home.join(".codex"))
}

fn resolve_sqlite_home(codex_home: &Path) -> PathBuf {
    existing_env_directory("CODEX_SQLITE_HOME").unwrap_or_else(|| codex_home.to_path_buf())
}

fn existing_env_directory(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os(name).map(PathBuf::from)?;
    (!path.as_os_str().is_empty() && path.is_dir()).then_some(path)
}

fn open_read_only(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Failed to open database {}: {error}", path.display()))?;
    connection
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
        .map_err(|error| format!("Failed to set database timeout {}: {error}", path.display()))?;
    Ok(connection)
}

fn open_read_write(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Failed to open database {}: {error}", path.display()))?;
    connection
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
        .map_err(|error| format!("Failed to set database timeout {}: {error}", path.display()))?;
    Ok(connection)
}

fn sqlite_candidate(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            ["db", "sqlite", "sqlite3"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

fn discover_database_paths(codex_home: &Path, reference_only: bool) -> (Vec<PathBuf>, Vec<String>) {
    let sqlite_home = resolve_sqlite_home(codex_home);
    let mut candidates = Vec::new();
    let mut warnings = Vec::new();
    let sqlite_dir = sqlite_home.join("sqlite");
    if let Ok(entries) = fs::read_dir(&sqlite_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && sqlite_candidate(&path) {
                candidates.push(path);
            }
        }
    }
    let legacy = sqlite_home.join("state_5.sqlite");
    if legacy.is_file() && !candidates.contains(&legacy) {
        candidates.push(legacy.clone());
    }
    candidates.sort_by_key(|path| {
        let legacy_rank = usize::from(path == &legacy);
        let codex_dev_rank =
            usize::from(path.file_name().and_then(OsStr::to_str) != Some("codex-dev.db"));
        (
            legacy_rank,
            codex_dev_rank,
            path.file_name().map(OsStr::to_os_string),
        )
    });
    let wanted = if reference_only {
        REFERENCE_TABLE_COLUMNS
            .iter()
            .map(|(table, _)| *table)
            .collect::<HashSet<_>>()
    } else {
        ["threads", "automation_runs", "inbox_items"]
            .into_iter()
            .collect::<HashSet<_>>()
    };
    let mut discovered = Vec::new();
    for path in candidates {
        let connection = match open_read_only(&path) {
            Ok(connection) => connection,
            Err(error) => {
                warnings.push(error);
                continue;
            }
        };
        let mut has_supported_table = false;
        for table in &wanted {
            match table_exists(&connection, table) {
                Ok(true) => has_supported_table = true,
                Ok(false) => {}
                Err(error) => {
                    warnings.push(format!("Failed to inspect database schema {}: {error}", path.display()))
                }
            }
        }
        if has_supported_table {
            discovered.push(path);
        }
    }
    (discovered, warnings)
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, String> {
    match connection.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
        [table],
        |_| Ok(()),
    ) {
        Ok(()) => Ok(true),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(error) => Err(format!("Failed to read SQLite table list: {error}")),
    }
}

fn table_columns(connection: &Connection, table: &str) -> Result<HashSet<String>, String> {
    if !table_exists(connection, table)? {
        return Ok(HashSet::new());
    }
    let mut statement = connection
        .prepare(&format!(
            "PRAGMA table_info(\"{}\")",
            table.replace('"', "\"\"")
        ))
        .map_err(|error| format!("Failed to read {table} table schema: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Failed to read {table} table schema: {error}"))?;
    rows.collect::<rusqlite::Result<HashSet<_>>>()
        .map_err(|error| format!("Failed to read {table} table schema: {error}"))
}

fn optional_column<'a>(columns: &HashSet<String>, column: &'a str, fallback: &'a str) -> &'a str {
    if columns.contains(column) {
        column
    } else {
        fallback
    }
}

fn normalized_millis_expression(column: &str) -> String {
    format!(
        "CASE WHEN {column} IS NULL THEN NULL WHEN ABS({column}) < 100000000000 THEN CAST({column} * 1000 AS INTEGER) ELSE CAST({column} AS INTEGER) END"
    )
}

fn list_codex_sessions_from_home(
    codex_home: &Path,
    offset: usize,
    requested_limit: usize,
) -> Result<CodexSessionPage, String> {
    let limit = requested_limit.clamp(1, MAX_PAGE_SIZE);
    let fetch_limit = offset.saturating_add(limit).saturating_add(1);
    let (database_paths, mut warnings) = discover_database_paths(codex_home, false);
    let mut sessions = Vec::new();
    let mut total_session_ids = HashSet::new();
    for path in &database_paths {
        match list_sessions_from_database(path, fetch_limit) {
            Ok(mut items) => sessions.append(&mut items),
            Err(error) => warnings.push(error),
        }
        match list_session_ids_from_database(path) {
            Ok(ids) => total_session_ids.extend(ids),
            Err(error) => warnings.push(error),
        }
    }
    let (mut rollout_sessions, rollout_warnings) =
        list_sessions_from_rollouts(codex_home, &total_session_ids);
    sessions.append(&mut rollout_sessions);
    warnings.extend(rollout_warnings);
    total_session_ids.extend(
        sessions
            .iter()
            .map(|session| session_identity_key(&session.id))
            .filter(|id| !id.is_empty()),
    );
    let mut sessions = merge_session_summaries(sessions);
    sessions.sort_by(|left, right| {
        right
            .updated_at_ms
            .cmp(&left.updated_at_ms)
            .then_with(|| right.id.cmp(&left.id))
            .then_with(|| left.database_path.cmp(&right.database_path))
    });
    let has_more = sessions.len() > offset.saturating_add(limit);
    let sessions = sessions.into_iter().skip(offset).take(limit).collect();
    Ok(CodexSessionPage {
        codex_home: codex_home.to_string_lossy().to_string(),
        database_paths: database_paths
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        sessions,
        total_count: total_session_ids.len(),
        offset,
        limit,
        has_more,
        warnings,
    })
}

fn merge_session_summaries(sessions: Vec<CodexSessionSummary>) -> Vec<CodexSessionSummary> {
    let mut merged = Vec::<CodexSessionSummary>::new();
    let mut positions = HashMap::<String, usize>::new();
    for session in sessions {
        let key = session_identity_key(&session.id);
        let Some(index) = positions.get(&key).copied() else {
            positions.insert(key, merged.len());
            merged.push(session);
            continue;
        };
        let existing = &mut merged[index];
        let candidate_is_newer = session.updated_at_ms > existing.updated_at_ms;
        if candidate_is_newer {
            let mut next = session;
            fill_missing_session_fields(&mut next, existing);
            *existing = next;
        } else {
            fill_missing_session_fields(existing, &session);
        }
    }
    merged
}

fn fill_missing_session_fields(target: &mut CodexSessionSummary, fallback: &CodexSessionSummary) {
    if target.title.trim().is_empty() {
        target.title = fallback.title.clone();
    }
    if target.cwd.trim().is_empty() {
        target.cwd = fallback.cwd.clone();
    }
    if target.model_provider.trim().is_empty() {
        target.model_provider = fallback.model_provider.clone();
    }
    if target.database_path.trim().is_empty() {
        target.database_path = fallback.database_path.clone();
    }
    if target.updated_at_ms.is_none() {
        target.updated_at_ms = fallback.updated_at_ms;
    }
}

fn list_sessions_from_rollouts(
    codex_home: &Path,
    indexed_ids: &HashSet<String>,
) -> (Vec<CodexSessionSummary>, Vec<String>) {
    let mut warnings = Vec::new();
    let metadata = match read_session_index_metadata(codex_home) {
        Ok(metadata) => metadata,
        Err(error) => {
            warnings.push(error);
            HashMap::new()
        }
    };
    let paths = match collect_rollout_files(codex_home) {
        Ok(paths) => paths,
        Err(error) => {
            warnings.push(error);
            return (Vec::new(), warnings);
        }
    };
    let mut sessions = Vec::new();
    for path in paths {
        let Some(id) = rollout_thread_id_from_file_name(&path) else {
            continue;
        };
        if indexed_ids.contains(&session_identity_key(&id)) {
            continue;
        }
        sessions.push(summarize_rollout_session(codex_home, &path, &id, &metadata));
    }
    (sessions, warnings)
}

fn read_session_index_metadata(
    codex_home: &Path,
) -> Result<HashMap<String, SessionIndexMetadata>, String> {
    let path = codex_home.join("session_index.jsonl");
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("Failed to read session_index.jsonl: {error}"))?;
    let mut metadata = HashMap::new();
    for line in content.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(object) = record.as_object() else {
            continue;
        };
        let Some(id) = object
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let item = SessionIndexMetadata {
            title: object
                .get("thread_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            updated_at_ms: object
                .get("updated_at")
                .and_then(Value::as_str)
                .and_then(parse_session_timestamp_millis),
        };
        metadata.insert(id.to_string(), item.clone());
        let normalized = normalize_thread_id(id);
        if !normalized.is_empty() {
            metadata.entry(normalized).or_insert(item);
        }
    }
    Ok(metadata)
}

fn summarize_rollout_session(
    codex_home: &Path,
    path: &Path,
    id: &str,
    metadata: &HashMap<String, SessionIndexMetadata>,
) -> CodexSessionSummary {
    let normalized_id = normalize_thread_id(id);
    let index_metadata = metadata.get(id).or_else(|| metadata.get(&normalized_id));
    let updated_at_ms = index_metadata
        .and_then(|item| item.updated_at_ms)
        .or_else(|| {
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        });
    CodexSessionSummary {
        id: id.to_string(),
        title: index_metadata
            .map(|item| item.title.clone())
            .unwrap_or_default(),
        cwd: String::new(),
        model_provider: String::new(),
        archived: path.starts_with(codex_home.join("archived_sessions")),
        updated_at_ms,
        database_path: String::new(),
    }
}

fn parse_session_timestamp_millis(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(value) = value.parse::<i64>() {
        return Some(if value.abs() < 100_000_000_000 {
            value.saturating_mul(1_000)
        } else {
            value
        });
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn list_sessions_from_database(
    path: &Path,
    limit: usize,
) -> Result<Vec<CodexSessionSummary>, String> {
    let connection = open_read_only(path)?;
    let mut sessions = Vec::new();
    if table_exists(&connection, "threads")? {
        sessions.extend(list_thread_rows(&connection, path, limit)?);
    }
    if table_exists(&connection, "automation_runs")? {
        sessions.extend(list_automation_rows(&connection, path, limit)?);
    }
    Ok(sessions)
}

fn list_session_ids_from_database(path: &Path) -> Result<HashSet<String>, String> {
    let connection = open_read_only(path)?;
    let mut ids = HashSet::new();
    for (table, column) in [("threads", "id"), ("automation_runs", "thread_id")] {
        let columns = table_columns(&connection, table)?;
        if !columns.contains(column) {
            continue;
        }
        let sql = format!(
            "SELECT DISTINCT \"{column}\" FROM \"{table}\" WHERE COALESCE(\"{column}\", '') <> ''"
        );
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| format!("Failed to count sessions {}: {error}", path.display()))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("Failed to count sessions {}: {error}", path.display()))?;
        for id in rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("Failed to count sessions {}: {error}", path.display()))?
        {
            let id = session_identity_key(&id);
            if !id.is_empty() {
                ids.insert(id);
            }
        }
    }
    Ok(ids)
}

fn list_thread_rows(
    connection: &Connection,
    path: &Path,
    limit: usize,
) -> Result<Vec<CodexSessionSummary>, String> {
    let columns = table_columns(connection, "threads")?;
    if !columns.contains("id") {
        return Ok(Vec::new());
    }
    let title = if columns.contains("title") && columns.contains("name") {
        "COALESCE(NULLIF(title, ''), NULLIF(name, ''), '')"
    } else {
        optional_column(&columns, "title", optional_column(&columns, "name", "''"))
    };
    let cwd = optional_column(&columns, "cwd", "''");
    let provider = optional_column(&columns, "model_provider", "''");
    let archived = optional_column(&columns, "archived", "0");
    let updated = if columns.contains("updated_at_ms") {
        "updated_at_ms".to_string()
    } else if columns.contains("updated_at") {
        normalized_millis_expression("updated_at")
    } else if columns.contains("created_at_ms") {
        "created_at_ms".to_string()
    } else if columns.contains("created_at") {
        normalized_millis_expression("created_at")
    } else {
        "NULL".to_string()
    };
    let sql = format!(
        "SELECT id, {title}, {cwd}, {provider}, {archived}, {updated} FROM threads ORDER BY COALESCE({updated}, 0) DESC, id DESC LIMIT ?1"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Failed to read session database {}: {error}", path.display()))?;
    let rows = statement
        .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            Ok(CodexSessionSummary {
                id: row.get(0)?,
                title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                cwd: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                model_provider: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                archived: row.get::<_, Option<i64>>(4)?.unwrap_or_default() != 0,
                updated_at_ms: row.get(5)?,
                database_path: path.to_string_lossy().to_string(),
            })
        })
        .map_err(|error| format!("Failed to read session database {}: {error}", path.display()))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed to read session database {}: {error}", path.display()))
}

fn list_automation_rows(
    connection: &Connection,
    path: &Path,
    limit: usize,
) -> Result<Vec<CodexSessionSummary>, String> {
    let columns = table_columns(connection, "automation_runs")?;
    if !columns.contains("thread_id") {
        return Ok(Vec::new());
    }
    let title = optional_column(&columns, "thread_title", "''");
    let cwd = optional_column(&columns, "source_cwd", "''");
    let status = optional_column(&columns, "status", "''");
    let updated = if columns.contains("updated_at") {
        normalized_millis_expression("updated_at")
    } else if columns.contains("created_at") {
        normalized_millis_expression("created_at")
    } else {
        "NULL".to_string()
    };
    let sql = format!(
        "SELECT thread_id, {title}, {cwd}, {status}, {updated} FROM automation_runs WHERE COALESCE(thread_id, '') <> '' ORDER BY COALESCE({updated}, 0) DESC, thread_id DESC LIMIT ?1"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Failed to read automation session database {}: {error}", path.display()))?;
    let rows = statement
        .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            let status = row.get::<_, Option<String>>(3)?.unwrap_or_default();
            Ok(CodexSessionSummary {
                id: row.get(0)?,
                title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                cwd: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                model_provider: String::new(),
                archived: status.eq_ignore_ascii_case("archived"),
                updated_at_ms: row.get(4)?,
                database_path: path.to_string_lossy().to_string(),
            })
        })
        .map_err(|error| format!("Failed to read automation session database {}: {error}", path.display()))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed to read automation session database {}: {error}", path.display()))
}

fn normalize_thread_id(value: &str) -> String {
    value
        .trim()
        .strip_prefix("local:")
        .unwrap_or(value.trim())
        .trim()
        .to_string()
}

fn session_identity_key(value: &str) -> String {
    let normalized = normalize_thread_id(value);
    if normalized.is_empty() {
        value.trim().to_string()
    } else {
        normalized
    }
}

fn delete_codex_sessions_from_home(
    codex_home: &Path,
    session_ids: Vec<String>,
) -> Result<CodexSessionDeleteBatchResult, String> {
    let _guard = CODEX_SESSION_WRITE_LOCK
        .lock()
        .map_err(|_| "Codex session write lock is poisoned".to_string())?;
    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for value in session_ids {
        let id = normalize_thread_id(&value);
        if !id.is_empty() && seen.insert(id.clone()) {
            unique.push(id);
        }
    }
    if unique.is_empty() {
        return Err("Select at least one valid session ID".to_string());
    }
    if unique.len() > MAX_PAGE_SIZE {
        return Err(format!("You can delete up to {MAX_PAGE_SIZE} sessions at a time"));
    }
    let (database_paths, database_warnings) = discover_database_paths(codex_home, false);
    if !database_warnings.is_empty() {
        return Err(format!(
            "Unable to safely inspect all Codex session databases; deletion aborted: {}",
            database_warnings.join("；")
        ));
    }
    let mut results = Vec::new();
    for id in unique {
        results.push(delete_one_session(codex_home, &database_paths, &id));
    }
    let deleted_count = results
        .iter()
        .filter(|result| matches!(result.status.as_str(), "deleted" | "partial"))
        .count();
    let failed_count = results.len().saturating_sub(deleted_count);
    Ok(CodexSessionDeleteBatchResult {
        results,
        deleted_count,
        failed_count,
    })
}

fn delete_one_session(
    codex_home: &Path,
    database_paths: &[PathBuf],
    session_id: &str,
) -> CodexSessionDeleteResult {
    let outcome = (|| -> Result<CodexSessionDeleteResult, String> {
        let mut plans = Vec::new();
        for path in database_paths {
            if let Some(plan) = build_database_delete_plan(path, session_id)? {
                plans.push(plan);
            }
        }
        if plans.is_empty() {
            return Ok(CodexSessionDeleteResult {
                session_id: session_id.to_string(),
                status: "notFound".to_string(),
                message: "Session not found in the local database".to_string(),
                backup_path: None,
            });
        }
        let backup_dir = create_delete_backup(codex_home, session_id, &plans)?;
        for (committed_databases, plan) in plans.iter().enumerate() {
            if let Err(error) = apply_database_delete_plan(plan, session_id) {
                let status = if committed_databases > 0 {
                    "partial"
                } else {
                    "failed"
                };
                return Ok(CodexSessionDeleteResult {
                    session_id: session_id.to_string(),
                    status: status.to_string(),
                    message: format!(
                        "Committed {committed_databases} database transactions; deletion failed for the remaining databases: {error}"
                    ),
                    backup_path: Some(backup_dir.to_string_lossy().to_string()),
                });
            }
        }
        let mut rollout_errors = Vec::new();
        let mut rollout_paths = plans
            .iter()
            .flat_map(|plan| plan.rollout_paths.iter().cloned())
            .collect::<Vec<_>>();
        rollout_paths.sort();
        rollout_paths.dedup();
        for path in rollout_paths {
            if !rollout_candidate(codex_home, &path).is_file() {
                continue;
            }
            match validated_rollout_path(codex_home, &path) {
                Ok(Some(path)) => {
                    if let Err(error) = fs::remove_file(&path) {
                        if error.kind() != std::io::ErrorKind::NotFound {
                            rollout_errors.push(format!("{}: {error}", path.display()));
                        }
                    }
                }
                Ok(None) => rollout_errors.push(format!(
                    "Preserved rollout outside the Codex session directory: {}",
                    path.display()
                )),
                Err(error) => rollout_errors.push(error),
            }
        }
        let (status, message) = if rollout_errors.is_empty() {
            ("deleted", "Deleted local database records and rollout files".to_string())
        } else {
            (
                "partial",
                format!(
                    "Database records were deleted, but some rollouts were not: {}",
                    rollout_errors.join("；")
                ),
            )
        };
        Ok(CodexSessionDeleteResult {
            session_id: session_id.to_string(),
            status: status.to_string(),
            message,
            backup_path: Some(backup_dir.to_string_lossy().to_string()),
        })
    })();
    outcome.unwrap_or_else(|message| CodexSessionDeleteResult {
        session_id: session_id.to_string(),
        status: "failed".to_string(),
        message,
        backup_path: None,
    })
}

fn build_database_delete_plan(
    path: &Path,
    session_id: &str,
) -> Result<Option<DatabaseDeletePlan>, String> {
    let connection = open_read_only(path)?;
    let mut tables = Map::new();
    let mut rollout_paths = Vec::new();
    let thread_rows =
        select_rows_if_supported(&connection, "threads", &["id"], "id = ?1", &[&session_id])?;
    let has_thread_row = !thread_rows.is_empty();
    if has_thread_row {
        for row in &thread_rows {
            if let Some(path) = row.get("rollout_path").and_then(Value::as_str) {
                if !path.trim().is_empty() {
                    rollout_paths.push(PathBuf::from(path));
                }
            }
        }
        tables.insert("threads".to_string(), Value::Array(thread_rows));
        for (table, column, clause) in [
            ("thread_dynamic_tools", "thread_id", "thread_id = ?1"),
            ("thread_goals", "thread_id", "thread_id = ?1"),
            ("stage1_outputs", "thread_id", "thread_id = ?1"),
            (
                "agent_job_items",
                "assigned_thread_id",
                "assigned_thread_id = ?1",
            ),
        ] {
            let rows =
                select_rows_if_supported(&connection, table, &[column], clause, &[&session_id])?;
            if !rows.is_empty() {
                tables.insert(table.to_string(), Value::Array(rows));
            }
        }
        if let Some(clause) = thread_spawn_edge_clause(&connection)? {
            let rows = select_rows_if_supported(
                &connection,
                "thread_spawn_edges",
                &[],
                clause,
                &[&session_id],
            )?;
            if !rows.is_empty() {
                tables.insert("thread_spawn_edges".to_string(), Value::Array(rows));
            }
        }
    }
    let automation_rows = select_rows_if_supported(
        &connection,
        "automation_runs",
        &["thread_id"],
        "thread_id = ?1",
        &[&session_id],
    )?;
    let inbox_rows = select_rows_if_supported(
        &connection,
        "inbox_items",
        &["thread_id"],
        "thread_id = ?1",
        &[&session_id],
    )?;
    let has_automation_rows = !automation_rows.is_empty() || !inbox_rows.is_empty();
    if !automation_rows.is_empty() {
        tables.insert("automation_runs".to_string(), Value::Array(automation_rows));
    }
    if !inbox_rows.is_empty() {
        tables.insert("inbox_items".to_string(), Value::Array(inbox_rows));
    }
    if !has_thread_row && !has_automation_rows {
        return Ok(None);
    }
    for (table, column, clause) in [
        ("local_thread_catalog", "thread_id", "thread_id = ?1"),
        ("thread_timeline_ledger", "thread_id", "thread_id = ?1"),
    ] {
        let rows = select_rows_if_supported(&connection, table, &[column], clause, &[&session_id])?;
        if !rows.is_empty() {
            tables.insert(table.to_string(), Value::Array(rows));
        }
    }
    Ok(Some(DatabaseDeletePlan {
        path: path.to_path_buf(),
        tables,
        rollout_paths,
        has_thread_row,
        has_automation_rows,
    }))
}

fn select_rows_if_supported(
    connection: &Connection,
    table: &str,
    required_columns: &[&str],
    where_clause: &str,
    params: &[&dyn ToSql],
) -> Result<Vec<Value>, String> {
    let columns = table_columns(connection, table)?;
    if columns.is_empty()
        || required_columns
            .iter()
            .any(|column| !columns.contains(*column))
    {
        return Ok(Vec::new());
    }
    select_json_rows(
        connection,
        &format!(
            "SELECT * FROM \"{}\" WHERE {where_clause}",
            table.replace('"', "\"\"")
        ),
        params,
    )
}

fn thread_spawn_edge_clause(connection: &Connection) -> Result<Option<&'static str>, String> {
    let columns = table_columns(connection, "thread_spawn_edges")?;
    Ok(
        match (
            columns.contains("parent_thread_id"),
            columns.contains("child_thread_id"),
        ) {
            (true, true) => Some("parent_thread_id = ?1 OR child_thread_id = ?1"),
            (true, false) => Some("parent_thread_id = ?1"),
            (false, true) => Some("child_thread_id = ?1"),
            (false, false) => None,
        },
    )
}

fn select_json_rows(
    connection: &Connection,
    sql: &str,
    params: &[&dyn ToSql],
) -> Result<Vec<Value>, String> {
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| format!("Failed to prepare session backup query: {error}"))?;
    let columns = statement
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    let rows = statement
        .query_map(params, |row| {
            let mut value = Map::new();
            for (index, column) in columns.iter().enumerate() {
                value.insert(column.clone(), sql_value_to_json(row.get_ref(index)?));
            }
            Ok(Value::Object(value))
        })
        .map_err(|error| format!("Failed to read session backup data: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed to read session backup data: {error}"))
}

fn sql_value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => json!(value),
        ValueRef::Real(value) => json!(value),
        ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).to_string()),
        ValueRef::Blob(value) => json!({
            "$base64": base64::engine::general_purpose::STANDARD.encode(value)
        }),
    }
}

fn create_operation_directory(
    codex_home: &Path,
    category: &str,
    label: &str,
) -> Result<PathBuf, String> {
    let safe_label = label
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(64)
        .collect::<String>();
    let sequence = BACKUP_SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed);
    let name = format!(
        "{}-{}-{sequence:020}-{}",
        Utc::now().format("%Y%m%d-%H%M%S%.3f"),
        std::process::id(),
        if safe_label.is_empty() {
            "operation"
        } else {
            &safe_label
        }
    );
    let directory = codex_home
        .join("backups_state")
        .join("easy-cli-proxy-api")
        .join(category)
        .join(name);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Failed to create session backup directory {}: {error}", directory.display()))?;
    Ok(directory)
}

fn create_delete_backup(
    codex_home: &Path,
    session_id: &str,
    plans: &[DatabaseDeletePlan],
) -> Result<PathBuf, String> {
    let directory = create_operation_directory(codex_home, "deletions", session_id)?;
    let mut sources = Vec::new();
    for (index, plan) in plans.iter().enumerate() {
        let file_name = format!("database-{index}-rows.json");
        let payload = json!({
            "databasePath": plan.path.to_string_lossy(),
            "tables": plan.tables,
        });
        write_json_atomically(&directory.join(&file_name), &payload)?;
        sources.push(json!({
            "databasePath": plan.path.to_string_lossy(),
            "rowsFile": file_name,
        }));
    }
    let rollout_directory = directory.join("rollouts");
    let mut rollout_files = Vec::new();
    let mut paths = plans
        .iter()
        .flat_map(|plan| plan.rollout_paths.iter().cloned())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    for (index, path) in paths.iter().enumerate() {
        if !rollout_candidate(codex_home, path).is_file() {
            continue;
        }
        let Some(validated) = validated_rollout_path(codex_home, path)? else {
            rollout_files.push(json!({
                "originalPath": path.to_string_lossy(),
                "backupPath": Value::Null,
                "skipped": "outside Codex session directories"
            }));
            continue;
        };
        if !validated.is_file() {
            continue;
        }
        fs::create_dir_all(&rollout_directory).map_err(|error| {
            format!(
                "Failed to create rollout backup directory {}: {error}",
                rollout_directory.display()
            )
        })?;
        let file_name = format!(
            "{index}-{}",
            validated
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("rollout.jsonl")
        );
        let target = rollout_directory.join(&file_name);
        fs::copy(&validated, &target)
            .map_err(|error| format!("Failed to back up rollout {}: {error}", validated.display()))?;
        rollout_files.push(json!({
            "originalPath": validated.to_string_lossy(),
            "backupPath": format!("rollouts/{file_name}")
        }));
    }
    write_json_atomically(
        &directory.join("metadata.json"),
        &json!({
            "schemaVersion": 1,
            "kind": "codex-session-deletion",
            "createdAt": Utc::now().to_rfc3339(),
            "sessionId": session_id,
            "codexHome": codex_home.to_string_lossy(),
            "sources": sources,
            "rolloutFiles": rollout_files,
            "managedBy": "EasyCLIProxyAPI"
        }),
    )?;
    Ok(directory)
}

fn rollout_candidate(codex_home: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        codex_home.join(path)
    }
}

fn validated_rollout_path(codex_home: &Path, path: &Path) -> Result<Option<PathBuf>, String> {
    let candidate = rollout_candidate(codex_home, path);
    if !candidate.is_file() {
        return Ok(None);
    }
    let canonical_home = fs::canonicalize(codex_home).map_err(|error| {
        format!(
            "Failed to resolve Codex home {}: {error}",
            codex_home.display()
        )
    })?;
    let canonical = fs::canonicalize(&candidate).map_err(|error| {
        format!(
            "Failed to resolve rollout path {}: {error}",
            candidate.display()
        )
    })?;
    for directory in SESSION_DIRS {
        let root = codex_home.join(directory);
        if let Ok(root) = fs::canonicalize(root) {
            if root.starts_with(&canonical_home)
                && canonical.starts_with(&root)
                && canonical.extension().and_then(OsStr::to_str) == Some("jsonl")
            {
                return Ok(Some(canonical));
            }
        }
    }
    Ok(None)
}

fn apply_database_delete_plan(plan: &DatabaseDeletePlan, session_id: &str) -> Result<(), String> {
    let mut connection = open_read_write(&plan.path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed to begin database transaction {}: {error}", plan.path.display()))?;
    if plan.has_thread_row {
        delete_where_if_supported(
            &transaction,
            "thread_dynamic_tools",
            &["thread_id"],
            "thread_id = ?1",
            &[&session_id],
        )?;
        delete_where_if_supported(
            &transaction,
            "thread_goals",
            &["thread_id"],
            "thread_id = ?1",
            &[&session_id],
        )?;
        if let Some(clause) = thread_spawn_edge_clause(&transaction)? {
            delete_where_if_supported(
                &transaction,
                "thread_spawn_edges",
                &[],
                clause,
                &[&session_id],
            )?;
        }
        delete_where_if_supported(
            &transaction,
            "stage1_outputs",
            &["thread_id"],
            "thread_id = ?1",
            &[&session_id],
        )?;
        let agent_columns = table_columns(&transaction, "agent_job_items")?;
        if agent_columns.contains("assigned_thread_id") {
            transaction
                .execute(
                    "UPDATE agent_job_items SET assigned_thread_id = NULL WHERE assigned_thread_id = ?1",
                    [session_id],
                )
                .map_err(|error| format!("Failed to update agent_job_items: {error}"))?;
        }
        delete_where_if_supported(&transaction, "threads", &["id"], "id = ?1", &[&session_id])?;
    }
    if plan.has_automation_rows {
        delete_where_if_supported(
            &transaction,
            "automation_runs",
            &["thread_id"],
            "thread_id = ?1",
            &[&session_id],
        )?;
        delete_where_if_supported(
            &transaction,
            "inbox_items",
            &["thread_id"],
            "thread_id = ?1",
            &[&session_id],
        )?;
    }
    delete_where_if_supported(
        &transaction,
        "local_thread_catalog",
        &["thread_id"],
        "thread_id = ?1",
        &[&session_id],
    )?;
    delete_where_if_supported(
        &transaction,
        "thread_timeline_ledger",
        &["thread_id"],
        "thread_id = ?1",
        &[&session_id],
    )?;
    transaction
        .commit()
        .map_err(|error| format!("Failed to commit database deletion {}: {error}", plan.path.display()))
}

fn delete_where_if_supported(
    connection: &Connection,
    table: &str,
    required_columns: &[&str],
    where_clause: &str,
    params: &[&dyn ToSql],
) -> Result<(), String> {
    let columns = table_columns(connection, table)?;
    if columns.is_empty()
        || required_columns
            .iter()
            .any(|column| !columns.contains(*column))
    {
        return Ok(());
    }
    connection
        .execute(
            &format!(
                "DELETE FROM \"{}\" WHERE {where_clause}",
                table.replace('"', "\"\"")
            ),
            params,
        )
        .map(|_| ())
        .map_err(|error| format!("Failed to delete related records from {table}: {error}"))
}

fn write_json_atomically(path: &Path, value: &Value) -> Result<(), String> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("Failed to serialize session backup: {error}"))?;
    crate::write_bytes_atomically(path, &bytes)
}

fn resolve_current_codex_provider(codex_home: &Path) -> Result<String, String> {
    let path = codex_home.join("config.toml");
    if !path.is_file() {
        return Ok("openai".to_string());
    }
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("Failed to read Codex configuration {}: {error}", path.display()))?;
    let document = text
        .parse::<toml::Value>()
        .map_err(|error| format!("Failed to parse Codex configuration: {error}"))?;
    Ok(document
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
        .unwrap_or("openai")
        .to_string())
}

fn repair_codex_session_metadata_from_home<F>(
    codex_home: &Path,
    target_provider: &str,
    mut progress: F,
) -> Result<CodexSessionRepairResult, String>
where
    F: FnMut(CodexSessionRepairProgress),
{
    let target_provider = target_provider.trim();
    if target_provider.is_empty() {
        return Err("Current Codex provider cannot be empty".to_string());
    }
    let _guard = CODEX_SESSION_WRITE_LOCK
        .lock()
        .map_err(|_| "Codex session write lock is poisoned".to_string())?;
    progress(CodexSessionRepairProgress {
        phase: "scanning".to_string(),
        percent: 10,
        processed: 0,
        total: 0,
    });
    let rollout_paths = collect_rollout_files(codex_home)?;
    let total = rollout_paths.len();
    let mut repairs = Vec::new();
    let mut skipped_locked_files = Vec::new();
    let mut warnings = Vec::new();
    for (index, path) in rollout_paths.iter().enumerate() {
        match build_rollout_repair(path, target_provider) {
            Ok(Some(repair)) => repairs.push(repair),
            Ok(None) => {}
            Err(error) if is_locked_error_message(&error) => {
                skipped_locked_files.push(path.to_string_lossy().to_string());
            }
            Err(error) => warnings.push(error),
        }
        if index % 25 == 0 || index + 1 == total {
            progress(CodexSessionRepairProgress {
                phase: "scanning".to_string(),
                percent: 10 + (((index + 1) * 25 / total.max(1)) as u8),
                processed: index + 1,
                total,
            });
        }
    }
    let projectless = load_projectless_thread_ids(codex_home)?;
    let (database_paths, database_warnings) = discover_database_paths(codex_home, false);
    warnings.extend(database_warnings);
    let (sqlite_updates_needed, database_count_warnings) =
        count_sqlite_repairs(&database_paths, &repairs, &projectless, target_provider)?;
    let sqlite_scan_incomplete = !database_count_warnings.is_empty();
    warnings.extend(database_count_warnings);
    let changed_repairs = repairs
        .iter()
        .filter(|repair| repair.changed)
        .cloned()
        .collect::<Vec<_>>();
    let backup_path =
        if changed_repairs.is_empty() && sqlite_updates_needed == 0 && !sqlite_scan_incomplete {
            None
        } else {
            progress(CodexSessionRepairProgress {
                phase: "backingUp".to_string(),
                percent: 45,
                processed: 0,
                total: changed_repairs.len(),
            });
            Some(create_repair_backup(
                codex_home,
                &database_paths,
                &changed_repairs,
                "provider-repair",
                target_provider,
            )?)
        };
    progress(CodexSessionRepairProgress {
        phase: "rewriting".to_string(),
        percent: 60,
        processed: 0,
        total: changed_repairs.len(),
    });
    let mut written = Vec::new();
    for (index, repair) in changed_repairs.iter().enumerate() {
        match crate::write_bytes_atomically(&repair.path, &repair.next) {
            Ok(()) => {
                restore_modified_time(&repair.path, repair.original_mtime);
                written.push(repair.clone());
            }
            Err(error) if is_locked_error_message(&error) => {
                let path = repair.path.to_string_lossy().to_string();
                if !skipped_locked_files.contains(&path) {
                    skipped_locked_files.push(path);
                }
            }
            Err(error) => {
                for previous in written.iter().rev() {
                    let _ = crate::write_bytes_atomically(&previous.path, &previous.original);
                    restore_modified_time(&previous.path, previous.original_mtime);
                }
                return Err(format!("Failed to write historical session; rollback attempted: {error}"));
            }
        }
        progress(CodexSessionRepairProgress {
            phase: "rewriting".to_string(),
            percent: 60 + (((index + 1) * 20 / changed_repairs.len().max(1)) as u8),
            processed: index + 1,
            total: changed_repairs.len(),
        });
    }
    progress(CodexSessionRepairProgress {
        phase: "updatingDatabase".to_string(),
        percent: 85,
        processed: 0,
        total: database_paths.len(),
    });
    let sqlite_rows_updated =
        match apply_sqlite_repairs(&database_paths, &repairs, &projectless, target_provider) {
            Ok((updated, database_lock_warnings)) => {
                warnings.extend(database_lock_warnings);
                updated
            }
            Err(error) => {
                for previous in written.iter().rev() {
                    let _ = crate::write_bytes_atomically(&previous.path, &previous.original);
                    restore_modified_time(&previous.path, previous.original_mtime);
                }
                return Err(format!(
                    "Failed to update session database; rollout rollback attempted and backup retained: {error}"
                ));
            }
        };
    if backup_path.is_some() {
        prune_repair_backups(codex_home)?;
    }
    let encrypted_content_warning = encrypted_content_warning(&repairs, target_provider);
    progress(CodexSessionRepairProgress {
        phase: "complete".to_string(),
        percent: 100,
        processed: total,
        total,
    });
    Ok(CodexSessionRepairResult {
        target_provider: target_provider.to_string(),
        changed_rollout_files: written.len(),
        sqlite_rows_updated,
        skipped_locked_files,
        backup_path: backup_path.map(|path| path.to_string_lossy().to_string()),
        encrypted_content_warning,
        warnings,
    })
}

fn collect_rollout_files(codex_home: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for directory in SESSION_DIRS {
        collect_rollout_files_recursive(&codex_home.join(directory), &mut files)?;
    }
    files.sort();
    Ok(files)
}

fn collect_rollout_files_recursive(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(root)
        .map_err(|error| format!("Failed to scan session directory {}: {error}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("Failed to read session directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to read session directory entry type: {error}"))?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_rollout_files_recursive(&path, files)?;
        } else if file_type.is_file() && path.extension().and_then(OsStr::to_str) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

fn build_rollout_repair(
    path: &Path,
    target_provider: &str,
) -> Result<Option<RolloutRepair>, String> {
    let original =
        fs::read(path).map_err(|error| format!("Failed to read rollout {}: {error}", path.display()))?;
    let text = String::from_utf8(original.clone())
        .map_err(|error| format!("Rollout is not UTF-8 {}: {error}", path.display()))?;
    let mut next = String::with_capacity(text.len());
    let mut changed = false;
    let mut session_meta_count = 0usize;
    let mut thread_id = None;
    let mut cwd = None;
    let mut providers = Vec::new();
    for segment in text.split_inclusive('\n') {
        let (body_with_cr, newline) = segment
            .strip_suffix('\n')
            .map_or((segment, ""), |body| (body, "\n"));
        let (body, carriage) = body_with_cr
            .strip_suffix('\r')
            .map_or((body_with_cr, ""), |body| (body, "\r"));
        let mut output = body.to_string();
        if let Ok(mut record) = serde_json::from_str::<Value>(body) {
            if record.get("type").and_then(Value::as_str) == Some("session_meta") {
                if let Some(payload) = record.get_mut("payload").and_then(Value::as_object_mut) {
                    session_meta_count += 1;
                    if thread_id.is_none() {
                        thread_id = payload
                            .get("id")
                            .and_then(Value::as_str)
                            .filter(|value| !value.trim().is_empty())
                            .map(ToString::to_string);
                    }
                    if cwd.is_none() {
                        cwd = payload
                            .get("cwd")
                            .and_then(Value::as_str)
                            .and_then(normalize_workspace_path);
                    }
                    if let Some(provider) = payload.get("model_provider").and_then(Value::as_str) {
                        providers.push(provider.to_string());
                    }
                    if payload.get("model_provider").and_then(Value::as_str)
                        != Some(target_provider)
                    {
                        payload.insert("model_provider".to_string(), json!(target_provider));
                        output = serde_json::to_string(&record)
                            .map_err(|error| format!("Failed to serialize session_meta: {error}"))?;
                        changed = true;
                    }
                }
            }
        }
        next.push_str(&output);
        next.push_str(carriage);
        next.push_str(newline);
    }
    if session_meta_count == 0 {
        return Ok(None);
    }
    let has_user_event = text.contains("\"user_message\"") || text.contains("\"user_input\"");
    Ok(Some(RolloutRepair {
        path: path.to_path_buf(),
        original,
        next: next.into_bytes(),
        original_mtime: fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok(),
        thread_id,
        cwd,
        has_user_event,
        providers,
        encrypted_content: text.contains("encrypted_content"),
        changed,
    }))
}

fn normalize_workspace_path(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if lower.starts_with(r"\\?\unc\") {
        return Some(format!(r"\\{}", value[8..].replace('/', r"\")));
    }
    if let Some(stripped) = value.strip_prefix(r"\\?\") {
        return Some(stripped.replace('\\', "/"));
    }
    Some(value.to_string())
}

fn is_locked_error_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("os error 32")
        || lower.contains("os error 33")
        || lower.contains("database is locked")
        || lower.contains("database is busy")
        || lower.contains("sharing violation")
        || lower.contains("being used by another process")
        || lower.contains("permission denied")
        || lower.contains("access is denied")
}

fn load_projectless_thread_ids(codex_home: &Path) -> Result<HashSet<String>, String> {
    let path = codex_home.join(".codex-global-state.json");
    if !path.is_file() {
        return Ok(HashSet::new());
    }
    let value = serde_json::from_str::<Value>(
        &fs::read_to_string(&path).map_err(|error| format!("Failed to read Codex global state: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse Codex global state: {error}"))?;
    Ok(value
        .get("projectless-thread-ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(ToString::to_string)
        .collect())
}

fn repair_maps(
    repairs: &[RolloutRepair],
    projectless: &HashSet<String>,
) -> (HashSet<String>, HashMap<String, String>) {
    let user_event_ids = repairs
        .iter()
        .filter(|repair| repair.has_user_event)
        .filter_map(|repair| repair.thread_id.clone())
        .collect::<HashSet<_>>();
    let cwd_by_id = repairs
        .iter()
        .filter_map(|repair| Some((repair.thread_id.clone()?, repair.cwd.clone()?)))
        .filter(|(id, _)| !projectless.contains(id))
        .collect::<HashMap<_, _>>();
    (user_event_ids, cwd_by_id)
}

fn count_sqlite_repairs(
    paths: &[PathBuf],
    repairs: &[RolloutRepair],
    projectless: &HashSet<String>,
    target_provider: &str,
) -> Result<(usize, Vec<String>), String> {
    let (user_event_ids, cwd_by_id) = repair_maps(repairs, projectless);
    let mut total = 0usize;
    let mut warnings = Vec::new();
    for path in paths {
        let count_result = (|| -> Result<usize, String> {
            let connection = open_read_only(path)?;
            let columns = table_columns(&connection, "threads")?;
            let mut count = 0usize;
            if columns.contains("model_provider") {
                count += connection
                    .query_row(
                        "SELECT COUNT(*) FROM threads WHERE COALESCE(model_provider, '') <> ?1",
                        [target_provider],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|error| format!("Failed to count provider migration rows: {error}"))?
                    as usize;
            }
            if columns.contains("has_user_event") {
                for id in &user_event_ids {
                    count += connection
                        .query_row(
                            "SELECT COUNT(*) FROM threads WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                            [id],
                            |row| row.get::<_, i64>(0),
                        )
                        .map_err(|error| format!("Failed to count session visibility update rows: {error}"))?
                        as usize;
                }
            }
            if columns.contains("cwd") {
                for (id, cwd) in &cwd_by_id {
                    count += connection
                        .query_row(
                            "SELECT COUNT(*) FROM threads WHERE id = ?1 AND COALESCE(cwd, '') <> ?2",
                            (id, cwd),
                            |row| row.get::<_, i64>(0),
                        )
                        .map_err(|error| format!("Failed to count project path update rows: {error}"))?
                        as usize;
                }
            }
            Ok(count)
        })();
        match count_result {
            Ok(count) => total += count,
            Err(error) if is_locked_error_message(&error) => warnings.push(format!(
                "Database is in use; cannot pre-count synchronization rows {}: {error}",
                path.display()
            )),
            Err(error) => return Err(error),
        }
    }
    Ok((total, warnings))
}

fn apply_sqlite_repairs(
    paths: &[PathBuf],
    repairs: &[RolloutRepair],
    projectless: &HashSet<String>,
    target_provider: &str,
) -> Result<(usize, Vec<String>), String> {
    let (user_event_ids, cwd_by_id) = repair_maps(repairs, projectless);
    let mut total = 0usize;
    let mut warnings = Vec::new();
    for path in paths {
        let update_result = (|| -> Result<usize, String> {
            let mut connection = open_read_write(path)?;
            let columns = table_columns(&connection, "threads")?;
            if columns.is_empty() {
                return Ok(0);
            }
            let transaction = connection
                .transaction()
                .map_err(|error| format!("Failed to begin historical session restoration transaction {}: {error}", path.display()))?;
            let mut updated = 0usize;
            if columns.contains("model_provider") {
                updated += transaction
                    .execute(
                        "UPDATE threads SET model_provider = ?1 WHERE COALESCE(model_provider, '') <> ?1",
                        [target_provider],
                    )
                    .map_err(|error| format!("Failed to update session provider: {error}"))?;
            }
            if columns.contains("has_user_event") {
                for id in &user_event_ids {
                    updated += transaction
                        .execute(
                            "UPDATE threads SET has_user_event = 1 WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                            [id],
                        )
                        .map_err(|error| format!("Failed to update session user-event flag: {error}"))?;
                }
            }
            if columns.contains("cwd") {
                for (id, cwd) in &cwd_by_id {
                    updated += transaction
                        .execute(
                            "UPDATE threads SET cwd = ?1 WHERE id = ?2 AND COALESCE(cwd, '') <> ?1",
                            (cwd, id),
                        )
                        .map_err(|error| format!("Failed to update session project path: {error}"))?;
                }
            }
            transaction
                .commit()
                .map_err(|error| format!("Failed to commit historical session restoration transaction {}: {error}", path.display()))?;
            Ok(updated)
        })();
        match update_result {
            Ok(updated) => total += updated,
            Err(error) if is_locked_error_message(&error) => warnings.push(format!(
                "Database is in use; skipped this synchronization {}: {error}",
                path.display()
            )),
            Err(error) => return Err(error),
        }
    }
    Ok((total, warnings))
}

fn create_repair_backup(
    codex_home: &Path,
    database_paths: &[PathBuf],
    repairs: &[RolloutRepair],
    label: &str,
    target_provider: &str,
) -> Result<PathBuf, String> {
    let directory = create_operation_directory(codex_home, "session-repair", label)?;
    for name in [
        "config.toml",
        ".codex-global-state.json",
        ".codex-global-state.json.bak",
    ] {
        let source = codex_home.join(name);
        if source.is_file() {
            fs::copy(&source, directory.join(name))
                .map_err(|error| format!("Failed to back up {name}: {error}"))?;
        }
    }
    let sqlite_home = resolve_sqlite_home(codex_home);
    let mut database_files = Vec::new();
    for database_path in database_paths {
        for source in sqlite_sidecar_paths(database_path) {
            if !source.is_file() {
                continue;
            }
            let relative = source
                .strip_prefix(&sqlite_home)
                .unwrap_or(source.as_path())
                .to_path_buf();
            let target = directory.join("databases").join(&relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Failed to create database backup directory: {error}"))?;
            }
            fs::copy(&source, &target)
                .map_err(|error| format!("Failed to back up database {}: {error}", source.display()))?;
            database_files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    let mut rollout_files = Vec::new();
    for (index, repair) in repairs.iter().enumerate() {
        let file_name = format!(
            "{index}-{}",
            repair
                .path
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("rollout.jsonl")
        );
        let target = directory.join("rollouts").join(&file_name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Failed to create rollout backup directory: {error}"))?;
        }
        fs::write(&target, &repair.original)
            .map_err(|error| format!("Failed to back up rollout {}: {error}", repair.path.display()))?;
        rollout_files.push(json!({
            "originalPath": repair.path.to_string_lossy(),
            "backupPath": format!("rollouts/{file_name}")
        }));
    }
    write_json_atomically(
        &directory.join("metadata.json"),
        &json!({
            "schemaVersion": 1,
            "kind": label,
            "createdAt": Utc::now().to_rfc3339(),
            "targetProvider": target_provider,
            "databaseFiles": database_files,
            "rolloutFiles": rollout_files,
            "managedBy": "EasyCLIProxyAPI"
        }),
    )?;
    Ok(directory)
}

fn sqlite_sidecar_paths(path: &Path) -> [PathBuf; 3] {
    [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.to_string_lossy())),
        PathBuf::from(format!("{}-shm", path.to_string_lossy())),
    ]
}

fn restore_modified_time(path: &Path, modified: Option<SystemTime>) {
    let Some(modified) = modified else {
        return;
    };
    if let Ok(file) = fs::File::options().write(true).open(path) {
        let _ = file.set_times(fs::FileTimes::new().set_modified(modified));
    }
}

fn encrypted_content_warning(repairs: &[RolloutRepair], target_provider: &str) -> Option<String> {
    let mut providers = repairs
        .iter()
        .filter(|repair| repair.encrypted_content)
        .flat_map(|repair| repair.providers.iter().cloned())
        .filter(|provider| provider != target_provider)
        .collect::<Vec<_>>();
    providers.sort();
    providers.dedup();
    if providers.is_empty() {
        return None;
    }
    Some(format!(
        "Some historical sessions contain encrypted_content from {}; metadata was synchronized to {}, but continuing or compacting these sessions may fail.",
        providers.join("、"),
        target_provider
    ))
}

fn prune_repair_backups(codex_home: &Path) -> Result<(), String> {
    let root = codex_home
        .join("backups_state")
        .join("easy-cli-proxy-api")
        .join("session-repair");
    if !root.is_dir() {
        return Ok(());
    }
    let mut directories = fs::read_dir(&root)
        .map_err(|error| format!("Failed to read historical session restoration backup: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect::<Vec<_>>();
    directories.sort_by_key(|entry| entry.file_name());
    let remove_count = directories.len().saturating_sub(REPAIR_BACKUP_KEEP_COUNT);
    for entry in directories.into_iter().take(remove_count) {
        let path = entry.path();
        if path.starts_with(&root) {
            fs::remove_dir_all(&path).map_err(|error| {
                format!("Failed to clean up old historical session restoration backup {}: {error}", path.display())
            })?;
        }
    }
    Ok(())
}

fn preview_session_index_cleanup_from_home(
    codex_home: &Path,
) -> Result<SessionIndexCleanupPreview, String> {
    let live_ids = collect_live_thread_ids(codex_home)?;
    let path = codex_home.join("session_index.jsonl");
    let Some(plan) = build_session_index_plan(&path, &live_ids)? else {
        return Ok(SessionIndexCleanupPreview {
            snapshot_sha256: sha256_hex(&[]),
            candidates: Vec::new(),
        });
    };
    Ok(SessionIndexCleanupPreview {
        snapshot_sha256: plan.snapshot_sha256,
        candidates: plan.candidates,
    })
}

fn collect_live_thread_ids(codex_home: &Path) -> Result<HashSet<String>, String> {
    let mut ids = HashSet::new();
    for path in collect_rollout_files(codex_home)? {
        if let Some(id) = rollout_thread_id_from_file_name(&path) {
            ids.insert(id);
        }
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if is_locked_io_error(&error) => continue,
            Err(error) => return Err(format!("Failed to read rollout {}: {error}", path.display())),
        };
        for line in text.lines() {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if record.get("type").and_then(Value::as_str) != Some("session_meta") {
                continue;
            }
            if let Some(id) = record
                .get("payload")
                .and_then(Value::as_object)
                .and_then(|payload| payload.get("id"))
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
            {
                ids.insert(id.to_string());
                ids.insert(normalize_thread_id(id));
            }
        }
    }
    let (database_paths, database_warnings) = discover_database_paths(codex_home, true);
    if !database_warnings.is_empty() {
        return Err(format!(
            "Unable to fully verify all Codex databases; invalid session record check aborted: {}",
            database_warnings.join("；")
        ));
    }
    for path in database_paths {
        let connection = open_read_only(&path)?;
        for (table, column) in REFERENCE_TABLE_COLUMNS {
            let columns = table_columns(&connection, table)?;
            if !columns.contains(column) {
                continue;
            }
            let sql = format!(
                "SELECT DISTINCT \"{}\" FROM \"{}\" WHERE COALESCE(\"{}\", '') <> ''",
                column, table, column
            );
            let mut statement = connection
                .prepare(&sql)
                .map_err(|error| format!("Failed to read session reference {}: {error}", path.display()))?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| format!("Failed to read session reference {}: {error}", path.display()))?;
            for id in rows
                .collect::<rusqlite::Result<HashSet<_>>>()
                .map_err(|error| format!("Failed to read session reference {}: {error}", path.display()))?
            {
                ids.insert(normalize_thread_id(&id));
                ids.insert(id);
            }
        }
    }
    Ok(ids)
}

fn rollout_thread_id_from_file_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    if stem.len() < 36 {
        return None;
    }
    let candidate = &stem[stem.len() - 36..];
    candidate
        .chars()
        .enumerate()
        .all(|(index, character)| match index {
            8 | 13 | 18 | 23 => character == '-',
            _ => character.is_ascii_hexdigit(),
        })
        .then(|| candidate.to_string())
}

fn build_session_index_plan(
    path: &Path,
    live_ids: &HashSet<String>,
) -> Result<Option<SessionIndexPlan>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let original =
        fs::read(path).map_err(|error| format!("Failed to read session_index.jsonl: {error}"))?;
    let original_text = String::from_utf8(original.clone())
        .map_err(|error| format!("session_index.jsonl is not UTF-8: {error}"))?;
    let mut seen_candidates = HashSet::new();
    let candidates = original_text
        .lines()
        .filter_map(known_session_index_candidate)
        .filter(|candidate| {
            !live_ids.contains(&candidate.id)
                && !live_ids.contains(&normalize_thread_id(&candidate.id))
        })
        .filter(|candidate| seen_candidates.insert(candidate.id.clone()))
        .collect();
    Ok(Some(SessionIndexPlan {
        path: path.to_path_buf(),
        snapshot_sha256: sha256_hex(&original),
        original,
        original_text,
        candidates,
    }))
}

fn known_session_index_candidate(line: &str) -> Option<SessionIndexCleanupCandidate> {
    let record = serde_json::from_str::<Value>(line).ok()?;
    let object = record.as_object()?;
    if object.len() != 3
        || !["id", "thread_name", "updated_at"]
            .iter()
            .all(|key| object.contains_key(*key))
    {
        return None;
    }
    let id = object.get("id")?.as_str()?.trim();
    let thread_name = object.get("thread_name")?.as_str()?;
    let updated_at = object.get("updated_at")?.as_str()?;
    if id.is_empty() || updated_at.trim().is_empty() {
        return None;
    }
    Some(SessionIndexCleanupCandidate {
        id: id.to_string(),
        thread_name: thread_name.to_string(),
        updated_at: updated_at.to_string(),
    })
}

fn apply_session_index_cleanup_from_home(
    codex_home: &Path,
    expected_snapshot: &str,
    confirmed_ids: &[String],
    require_stopped_app: bool,
) -> Result<SessionIndexCleanupResult, String> {
    let _guard = CODEX_SESSION_WRITE_LOCK
        .lock()
        .map_err(|_| "Codex session write lock is poisoned".to_string())?;
    if require_stopped_app {
        ensure_codex_apps_stopped()?;
    }
    let live_ids = collect_live_thread_ids(codex_home)?;
    let path = codex_home.join("session_index.jsonl");
    let plan = build_session_index_plan(&path, &live_ids)?
        .ok_or_else(|| "session_index.jsonl does not exist and cannot be cleaned".to_string())?;
    if plan.snapshot_sha256 != expected_snapshot {
        return Err("session_index.jsonl changed after preview. Preview it again".to_string());
    }
    let candidate_ids = plan
        .candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<HashSet<_>>();
    let selected = confirmed_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(ToString::to_string)
        .collect::<HashSet<_>>();
    if selected
        .iter()
        .any(|id| !candidate_ids.contains(id.as_str()))
    {
        return Err("The confirmation list has expired or contains non-candidate sessions. Preview again".to_string());
    }
    if selected.is_empty() {
        return Ok(SessionIndexCleanupResult {
            pruned_entries: 0,
            backup_path: None,
        });
    }
    let (next, removed) = filter_session_index(&plan.original_text, &selected);
    if removed == 0 {
        return Ok(SessionIndexCleanupResult {
            pruned_entries: 0,
            backup_path: None,
        });
    }
    let backup_dir =
        create_operation_directory(codex_home, "session-index-cleanup", "index-cleanup")?;
    fs::write(backup_dir.join("session_index.jsonl"), &plan.original)
        .map_err(|error| format!("Failed to back up session_index.jsonl: {error}"))?;
    write_json_atomically(
        &backup_dir.join("metadata.json"),
        &json!({
            "schemaVersion": 1,
            "kind": "session-index-cleanup",
            "createdAt": Utc::now().to_rfc3339(),
            "snapshotSha256": plan.snapshot_sha256,
            "removedEntries": removed,
            "managedBy": "EasyCLIProxyAPI"
        }),
    )?;
    let current = fs::read(&plan.path)
        .map_err(|error| format!("Failed to reread session_index.jsonl before writing: {error}"))?;
    if current != plan.original {
        return Err(format!(
            "session_index.jsonl changed again before writing; new content was not overwritten, and the backup is at {}",
            backup_dir.display()
        ));
    }
    if require_stopped_app {
        ensure_codex_apps_stopped()?;
    }
    let expected_after_sha256 = sha256_hex(next.as_bytes());
    crate::write_bytes_atomically(&plan.path, next.as_bytes())?;
    let written = fs::read(&plan.path)
        .map_err(|error| format!("Failed to verify session_index.jsonl after writing: {error}"))?;
    if sha256_hex(&written) != expected_after_sha256 {
        return Err(format!(
            "session_index.jsonl SHA-256 verification failed after writing; the original file backup is at {}",
            backup_dir.display()
        ));
    }
    Ok(SessionIndexCleanupResult {
        pruned_entries: removed,
        backup_path: Some(backup_dir.to_string_lossy().to_string()),
    })
}

fn filter_session_index(text: &str, selected: &HashSet<String>) -> (String, usize) {
    let mut next = String::with_capacity(text.len());
    let mut removed = 0usize;
    for segment in text.split_inclusive('\n') {
        let (body_with_cr, newline) = segment
            .strip_suffix('\n')
            .map_or((segment, ""), |body| (body, "\n"));
        let body = body_with_cr.strip_suffix('\r').unwrap_or(body_with_cr);
        let remove = known_session_index_candidate(body)
            .is_some_and(|candidate| selected.contains(&candidate.id));
        if remove {
            removed += 1;
        } else {
            next.push_str(body_with_cr);
            next.push_str(newline);
        }
    }
    (next, removed)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_locked_io_error(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
        || matches!(error.raw_os_error(), Some(32 | 33))
}

fn ensure_codex_apps_stopped() -> Result<(), String> {
    let running = codex_app_process_ids()?;
    if running.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Codex App / ChatGPT is still running (process: {}). Exit it completely, then preview and clean again",
        running
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join("、")
    ))
}

#[cfg(target_os = "windows")]
fn codex_app_process_ids() -> Result<Vec<u32>, String> {
    windows_app_process_ids(&["ChatGPT.exe", "Codex.exe"])
}

#[cfg(target_os = "windows")]
fn windows_app_process_ids(names: &[&str]) -> Result<Vec<u32>, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut ids = Vec::new();
    for name in names {
        let filter = format!("IMAGENAME eq {name}");
        let output = Command::new("tasklist")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|error| format!("Failed to check {name} process status: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Error checking {name} process status: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        ids.extend(parse_windows_tasklist_process_ids(&output.stdout));
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

#[cfg(target_os = "windows")]
fn parse_windows_tasklist_process_ids(output: &[u8]) -> Vec<u32> {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| {
            let columns = line
                .trim()
                .trim_matches('"')
                .split("\",\"")
                .collect::<Vec<_>>();
            columns.get(1)?.parse::<u32>().ok()
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn codex_app_process_ids() -> Result<Vec<u32>, String> {
    unix_app_process_ids(&["ChatGPT", "Codex"])
}

#[cfg(not(target_os = "windows"))]
fn unix_app_process_ids(names: &[&str]) -> Result<Vec<u32>, String> {
    let mut ids = Vec::new();
    for name in names {
        let output = Command::new("pgrep")
            .args(["-x", name])
            .output()
            .map_err(|error| format!("Failed to check {name} process status: {error}"))?;
        match output.status.code() {
            Some(0) => ids.extend(
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter_map(|line| line.trim().parse::<u32>().ok()),
            ),
            Some(1) => {}
            _ => {
                return Err(format!(
                    "Error checking {name} process status: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_tasklist_parser_accepts_csv_rows_and_ignores_status_messages() {
        let output = b"\"ChatGPT.exe\",\"1234\",\"Console\",\"1\",\"10,000 K\"\r\nINFO: No tasks are running which match the specified criteria.\r\n\"ChatGPT.exe\",\"5678\",\"Console\",\"1\",\"20,000 K\"\r\n";
        assert_eq!(parse_windows_tasklist_process_ids(output), vec![1234, 5678]);
    }

    pub(super) fn test_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "easy-cli-proxy-codex-sessions-{name}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn create_threads_db(path: &Path, rollout: &Path) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, cwd TEXT, model_provider TEXT, archived INTEGER, has_user_event INTEGER, updated_at_ms INTEGER);\n\
                 CREATE TABLE thread_dynamic_tools (thread_id TEXT, name TEXT);\n\
                 CREATE TABLE thread_spawn_edges (parent_thread_id TEXT, child_thread_id TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES ('thread-1', ?1, 'First', 'C:/old', 'openai', 0, 0, 200)",
                [rollout.to_string_lossy().as_ref()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO thread_dynamic_tools VALUES ('thread-1', 'tool')",
                [],
            )
            .unwrap();
    }

    #[test]
    fn list_merges_threads_and_automation_rows_in_update_order() {
        let root = test_root("list");
        let sqlite = root.join("sqlite");
        fs::create_dir_all(&sqlite).unwrap();
        let rollout = root.join("sessions/rollout.jsonl");
        fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        fs::write(&rollout, "{}\n").unwrap();
        create_threads_db(&root.join("state_5.sqlite"), &rollout);
        let automation = Connection::open(sqlite.join("codex-dev.db")).unwrap();
        automation
            .execute_batch(
                "CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY, status TEXT, thread_title TEXT, source_cwd TEXT, updated_at INTEGER);\n\
                 INSERT INTO automation_runs VALUES ('thread-1', 'running', 'First new', 'C:/newer', 4);\n\
                 INSERT INTO automation_runs VALUES ('thread-2', 'archived', 'Second', 'C:/new', 3);",
            )
            .unwrap();
        drop(automation);

        let page = list_codex_sessions_from_home(&root, 0, 50).unwrap();
        assert_eq!(page.sessions.len(), 2);
        assert_eq!(page.total_count, 2);
        assert_eq!(page.sessions[0].id, "thread-1");
        assert_eq!(page.sessions[0].title, "First new");
        assert_eq!(page.sessions[0].updated_at_ms, Some(4000));
        assert_eq!(page.sessions[1].id, "thread-2");
        assert!(page.sessions[1].archived);
        let second_page = list_codex_sessions_from_home(&root, 1, 1).unwrap();
        assert_eq!(second_page.sessions.len(), 1);
        assert_eq!(second_page.total_count, 2);
        assert_eq!(second_page.sessions[0].id, "thread-2");
        assert!(!second_page.has_more);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn list_includes_unindexed_sessions_without_rollout_metadata() {
        let root = test_root("list-all-providers");
        let api_id = "019f1234-abcd-7000-8000-000000000001";
        let oauth_id = "019f1234-abcd-7000-8000-000000000002";
        let api_rollout = root
            .join("sessions")
            .join(format!("rollout-2026-08-04T12-00-00-{api_id}.jsonl"));
        let oauth_rollout = root
            .join("archived_sessions")
            .join(format!("rollout-2026-08-03T12-00-00-{oauth_id}.jsonl"));
        fs::create_dir_all(api_rollout.parent().unwrap()).unwrap();
        fs::create_dir_all(oauth_rollout.parent().unwrap()).unwrap();
        fs::write(
            &api_rollout,
            "not parsed by the session list\n",
        )
        .unwrap();
        fs::write(
            &oauth_rollout,
            "not parsed by the session list\n",
        )
        .unwrap();
        fs::write(
            root.join("session_index.jsonl"),
            concat!(
                "{\"id\":\"019f1234-abcd-7000-8000-000000000001\",\"thread_name\":\"API session\",\"updated_at\":\"2026-08-04T12:00:00Z\"}\n",
                "{\"id\":\"019f1234-abcd-7000-8000-000000000002\",\"thread_name\":\"OAuth session\",\"updated_at\":\"2026-08-03T12:00:00Z\"}\n"
            ),
        )
        .unwrap();

        let page = list_codex_sessions_from_home(&root, 0, 50).unwrap();
        assert_eq!(page.sessions.len(), 2);
        assert_eq!(page.sessions[0].id, api_id);
        assert_eq!(page.sessions[0].title, "API session");
        assert!(!page.sessions[0].archived);
        assert_eq!(page.sessions[1].id, oauth_id);
        assert_eq!(page.sessions[1].title, "OAuth session");
        assert!(page.sessions[1].archived);
        assert!(page.sessions.iter().all(|session| session.cwd.is_empty() && session.model_provider.is_empty()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn listing_uses_filenames_and_index_without_parsing_rollouts() {
        let root = test_root("list-no-rollout-reads");
        fs::create_dir_all(root.join("sessions")).unwrap();
        fs::create_dir_all(root.join("archived_sessions")).unwrap();
        let mut index = String::new();
        for number in 0..6 {
            let id = format!("019f1234-abcd-7000-8000-{number:012}");
            let directory = if number % 2 == 0 {
                "sessions"
            } else {
                "archived_sessions"
            };
            let path = root
                .join(directory)
                .join(format!("rollout-2026-10-10T10-00-00-{id}.jsonl"));
            fs::write(
                path,
                b"not JSON; listing must not read or parse this file\xff",
            )
            .unwrap();
            index.push_str(&format!("{}\n", json!({"id":id,"thread_name":format!("Session {number}"),"updated_at":(1000 + number).to_string()})));
        }
        fs::write(root.join("session_index.jsonl"), index).unwrap();
        let page = list_codex_sessions_from_home(&root, 2, 2).unwrap();
        assert_eq!(page.total_count, 6);
        assert_eq!(page.sessions.len(), 2);
        assert!(page.has_more);
        assert!(page.warnings.is_empty(), "{:?}", page.warnings);
        assert_eq!(page.sessions[0].title, "Session 3");
        assert!(page.sessions[0].archived);
        assert_eq!(page.sessions[1].title, "Session 2");
        assert!(!page.sessions[1].archived);
        assert!(page
            .sessions
            .iter()
            .all(|session| session.cwd.is_empty() && session.model_provider.is_empty()));
        assert!(!root.join("backups_state").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn listing_succeeds_when_every_rollout_is_locked_against_reads() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = test_root("list-locked-rollouts");
        fs::create_dir_all(root.join("sessions")).unwrap();
        let mut guards = Vec::new();
        for number in 0..4 {
            let path = root.join("sessions").join(format!(
                "rollout-019f1234-abcd-7000-8000-{number:012}.jsonl"
            ));
            fs::write(&path, "secret message contents").unwrap();
            guards.push(
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(&path)
                    .unwrap(),
            );
            assert!(fs::File::open(&path).is_err());
        }
        for offset in [0, 2] {
            let page = list_codex_sessions_from_home(&root, offset, 2).unwrap();
            assert_eq!(page.total_count, 4);
            assert_eq!(page.sessions.len(), 2);
            assert!(page.warnings.is_empty(), "{:?}", page.warnings);
        }
        drop(guards);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn list_accepts_optional_thread_columns_and_warns_about_a_broken_database() {
        let root = test_root("list-optional");
        let sqlite = root.join("sqlite");
        fs::create_dir_all(&sqlite).unwrap();
        let connection = Connection::open(sqlite.join("optional.sqlite3")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT);\n\
                 CREATE TABLE inbox_items (id TEXT PRIMARY KEY);\n\
                 INSERT INTO threads VALUES ('optional-thread', 'Optional');",
            )
            .unwrap();
        drop(connection);
        fs::write(sqlite.join("broken.db"), b"not a sqlite database").unwrap();

        let page = list_codex_sessions_from_home(&root, 0, 0).unwrap();
        assert_eq!(page.limit, 1);
        assert_eq!(page.sessions.len(), 1);
        assert_eq!(page.sessions[0].id, "optional-thread");
        assert_eq!(page.sessions[0].cwd, "");
        assert_eq!(page.sessions[0].updated_at_ms, None);
        assert!(page
            .warnings
            .iter()
            .any(|warning| warning.contains("broken.db")));
        assert_eq!(
            list_codex_sessions_from_home(&root, 0, usize::MAX)
                .unwrap()
                .limit,
            MAX_PAGE_SIZE
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deletion_backs_up_rows_and_removes_only_in_home_rollout() {
        let root = test_root("delete");
        let rollout = root.join("sessions/2026/rollout.jsonl");
        fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        fs::write(&rollout, "session\n").unwrap();
        create_threads_db(&root.join("state_5.sqlite"), &rollout);

        let result = delete_codex_sessions_from_home(&root, vec!["thread-1".to_string()]).unwrap();
        assert_eq!(result.deleted_count, 1);
        assert_eq!(result.results[0].status, "deleted");
        assert!(!rollout.exists());
        let backup = PathBuf::from(result.results[0].backup_path.as_ref().unwrap());
        assert!(backup.join("metadata.json").is_file());
        assert!(backup.join("database-0-rows.json").is_file());
        let connection = Connection::open(root.join("state_5.sqlite")).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM threads", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deletion_cleans_known_relations_and_preserves_outside_rollouts() {
        let root = test_root("delete-relations");
        let outside_rollout = root.with_extension("outside.jsonl");
        fs::write(&outside_rollout, "outside\n").unwrap();
        let database = root.join("state_5.sqlite");
        create_threads_db(&database, &outside_rollout);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE thread_goals (thread_id TEXT, goal TEXT);\n\
                 CREATE TABLE stage1_outputs (thread_id TEXT, output TEXT);\n\
                 CREATE TABLE local_thread_catalog (thread_id TEXT, value TEXT);\n\
                 CREATE TABLE thread_timeline_ledger (thread_id TEXT, value TEXT);\n\
                 CREATE TABLE automation_runs (thread_id TEXT, title TEXT);\n\
                 CREATE TABLE inbox_items (id TEXT, thread_id TEXT);\n\
                 CREATE TABLE agent_job_items (id TEXT, assigned_thread_id TEXT);\n\
                 INSERT INTO thread_goals VALUES ('thread-1', 'goal');\n\
                 INSERT INTO thread_spawn_edges VALUES ('thread-1', 'child');\n\
                 INSERT INTO stage1_outputs VALUES ('thread-1', 'output');\n\
                 INSERT INTO local_thread_catalog VALUES ('thread-1', 'catalog');\n\
                 INSERT INTO thread_timeline_ledger VALUES ('thread-1', 'timeline');\n\
                 INSERT INTO automation_runs VALUES ('thread-1', 'automation');\n\
                 INSERT INTO inbox_items VALUES ('inbox-1', 'thread-1');\n\
                 INSERT INTO agent_job_items VALUES ('job-1', 'thread-1');",
            )
            .unwrap();
        drop(connection);

        let result = delete_codex_sessions_from_home(&root, vec!["thread-1".to_string()]).unwrap();
        assert_eq!(result.results[0].status, "partial");
        assert!(outside_rollout.is_file());
        let backup = PathBuf::from(result.results[0].backup_path.as_ref().unwrap());
        let metadata = fs::read_to_string(backup.join("metadata.json")).unwrap();
        assert!(metadata.contains("outside Codex session directories"));

        let connection = Connection::open(&database).unwrap();
        for table in [
            "threads",
            "thread_dynamic_tools",
            "thread_goals",
            "thread_spawn_edges",
            "stage1_outputs",
            "local_thread_catalog",
            "thread_timeline_ledger",
            "automation_runs",
            "inbox_items",
        ] {
            let count = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table} should be cleaned");
        }
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM agent_job_items WHERE assigned_thread_id IS NULL",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
        fs::remove_file(outside_rollout).unwrap();
    }

    #[test]
    fn deletion_skips_related_tables_that_lack_optional_thread_columns() {
        let root = test_root("delete-optional");
        let rollout = root.join("sessions/rollout.jsonl");
        fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        fs::write(&rollout, "session\n").unwrap();
        let database = root.join("state_5.sqlite");
        create_threads_db(&database, &rollout);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE thread_goals (id TEXT PRIMARY KEY);\n\
                 CREATE TABLE inbox_items (id TEXT PRIMARY KEY);\n\
                 INSERT INTO thread_goals VALUES ('goal-1');\n\
                 INSERT INTO inbox_items VALUES ('inbox-1');",
            )
            .unwrap();
        drop(connection);

        let result = delete_codex_sessions_from_home(&root, vec!["thread-1".to_string()]).unwrap();
        assert_eq!(result.results[0].status, "deleted");
        let connection = Connection::open(&database).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM thread_goals", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM inbox_items", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deletion_failure_keeps_transaction_rows_and_returns_the_backup() {
        let root = test_root("delete-failure");
        let rollout = root.join("sessions/rollout.jsonl");
        fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        fs::write(&rollout, "session\n").unwrap();
        let database = root.join("state_5.sqlite");
        create_threads_db(&database, &rollout);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER block_thread_delete BEFORE DELETE ON threads\n\
                 BEGIN SELECT RAISE(ABORT, 'blocked by test'); END;",
            )
            .unwrap();
        drop(connection);

        let result = delete_codex_sessions_from_home(&root, vec!["thread-1".to_string()]).unwrap();
        assert_eq!(result.results[0].status, "failed");
        let backup = PathBuf::from(result.results[0].backup_path.as_ref().unwrap());
        assert!(backup.join("database-0-rows.json").is_file());
        assert!(rollout.is_file());
        let connection = Connection::open(&database).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM threads", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM thread_dynamic_tools", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deletion_reports_partial_when_an_earlier_database_transaction_committed() {
        let root = test_root("delete-partial-database");
        let rollout = root.join("sessions/rollout.jsonl");
        fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        fs::write(&rollout, "session\n").unwrap();
        let legacy_path = root.join("state_5.sqlite");
        create_threads_db(&legacy_path, &rollout);
        let legacy = Connection::open(&legacy_path).unwrap();
        legacy
            .execute_batch(
                "CREATE TRIGGER block_thread_delete BEFORE DELETE ON threads\n\
                 BEGIN SELECT RAISE(ABORT, 'blocked by test'); END;",
            )
            .unwrap();
        drop(legacy);
        let sqlite = root.join("sqlite");
        fs::create_dir_all(&sqlite).unwrap();
        let automation_path = sqlite.join("codex-dev.db");
        let automation = Connection::open(&automation_path).unwrap();
        automation
            .execute_batch(
                "CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY);\n\
                 CREATE TABLE inbox_items (id TEXT PRIMARY KEY, thread_id TEXT);\n\
                 INSERT INTO automation_runs VALUES ('thread-1');\n\
                 INSERT INTO inbox_items VALUES ('inbox-1', 'thread-1');",
            )
            .unwrap();
        drop(automation);

        let result = delete_codex_sessions_from_home(&root, vec!["thread-1".to_string()]).unwrap();
        assert_eq!(result.results[0].status, "partial");
        assert_eq!(result.deleted_count, 1);
        assert!(result.results[0].backup_path.is_some());
        let automation = Connection::open(&automation_path).unwrap();
        assert_eq!(
            automation
                .query_row("SELECT COUNT(*) FROM automation_runs", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        let legacy = Connection::open(&legacy_path).unwrap();
        assert_eq!(
            legacy
                .query_row("SELECT COUNT(*) FROM threads", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(automation);
        drop(legacy);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn provider_repair_updates_rollout_and_sqlite_and_preserves_projectless_cwd() {
        let root = test_root("repair");
        fs::create_dir_all(root.join("sessions")).unwrap();
        fs::write(root.join("config.toml"), "model_provider = \"cpa-gui\"\n").unwrap();
        fs::write(
            root.join(".codex-global-state.json"),
            r#"{"projectless-thread-ids":["thread-1"]}"#,
        )
        .unwrap();
        let rollout = root.join("sessions/rollout.jsonl");
        fs::write(
            &rollout,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"C:/new\",\"model_provider\":\"openai\"}}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\"}}\n",
        )
        .unwrap();
        create_threads_db(&root.join("state_5.sqlite"), &rollout);

        let result = repair_codex_session_metadata_from_home(&root, "cpa-gui", |_| {}).unwrap();
        assert_eq!(result.changed_rollout_files, 1);
        assert_eq!(result.sqlite_rows_updated, 2);
        let backup = PathBuf::from(result.backup_path.as_ref().unwrap());
        assert!(backup.join("databases/state_5.sqlite").is_file());
        assert!(backup.join(".codex-global-state.json").is_file());
        assert!(fs::read_to_string(&rollout).unwrap().contains("cpa-gui"));
        let connection = Connection::open(root.join("state_5.sqlite")).unwrap();
        let row = connection
            .query_row(
                "SELECT model_provider, has_user_event, cwd FROM threads WHERE id = 'thread-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row, ("cpa-gui".to_string(), 1, "C:/old".to_string()));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn provider_repair_no_change_does_not_create_a_backup() {
        let root = test_root("repair-no-change");
        fs::create_dir_all(root.join("sessions")).unwrap();
        let rollout = root.join("sessions/rollout.jsonl");
        fs::write(
            &rollout,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"cpa-gui\"}}\n",
        )
        .unwrap();

        let result = repair_codex_session_metadata_from_home(&root, "cpa-gui", |_| {}).unwrap();
        assert_eq!(result.changed_rollout_files, 0);
        assert_eq!(result.sqlite_rows_updated, 0);
        assert_eq!(result.backup_path, None);
        assert!(!root.join("backups_state").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn provider_repair_updates_all_metadata_lines_and_preserves_mtime_and_crlf() {
        let root = test_root("repair-multiple-meta");
        fs::create_dir_all(root.join("archived_sessions/2026/07")).unwrap();
        let rollout = root.join("archived_sessions/2026/07/rollout.jsonl");
        let original = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"openai\"}}\r\n",
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"legacy\"}}\r\n",
            "{\"type\":\"response_item\",\"payload\":{\"encrypted_content\":\"ciphertext\"}}\r\n"
        );
        fs::write(&rollout, original).unwrap();
        let expected_mtime = std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        fs::File::options()
            .write(true)
            .open(&rollout)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(expected_mtime))
            .unwrap();
        let before_mtime = fs::metadata(&rollout).unwrap().modified().unwrap();

        let result = repair_codex_session_metadata_from_home(&root, "cpa-gui", |_| {}).unwrap();
        assert_eq!(result.changed_rollout_files, 1);
        assert!(result.encrypted_content_warning.is_some());
        let next = fs::read(&rollout).unwrap();
        let next_text = String::from_utf8(next.clone()).unwrap();
        assert_eq!(next_text.matches("cpa-gui").count(), 2);
        assert!(next
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .all(|(index, _)| index > 0 && next[index - 1] == b'\r'));
        assert_eq!(
            fs::metadata(&rollout).unwrap().modified().unwrap(),
            before_mtime
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repair_backup_copies_sqlite_sidecars_and_global_state() {
        let root = test_root("repair-sidecars");
        let database = root.join("state_5.sqlite");
        fs::write(&database, b"database").unwrap();
        fs::write(format!("{}-wal", database.to_string_lossy()), b"wal").unwrap();
        fs::write(format!("{}-shm", database.to_string_lossy()), b"shm").unwrap();
        fs::write(root.join(".codex-global-state.json"), b"{}").unwrap();

        let backup = create_repair_backup(
            &root,
            std::slice::from_ref(&database),
            &[],
            "provider-repair",
            "cpa-gui",
        )
        .unwrap();
        for name in ["state_5.sqlite", "state_5.sqlite-wal", "state_5.sqlite-shm"] {
            assert!(backup.join("databases").join(name).is_file());
        }
        assert!(backup.join(".codex-global-state.json").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repair_backup_rotation_keeps_only_the_latest_five_directories() {
        let root = test_root("repair-rotation");
        for index in 0..7 {
            let directory =
                create_operation_directory(&root, "session-repair", &format!("repair-{index}"))
                    .unwrap();
            fs::write(directory.join("marker"), index.to_string()).unwrap();
        }
        prune_repair_backups(&root).unwrap();
        let directories =
            fs::read_dir(root.join("backups_state/easy-cli-proxy-api/session-repair"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_dir())
                .count();
        assert_eq!(directories, REPAIR_BACKUP_KEEP_COUNT);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lock_error_detection_covers_rollout_and_sqlite_lock_messages() {
        assert!(is_locked_error_message("os error 32"));
        assert!(is_locked_error_message("database is locked"));
        assert!(is_locked_error_message(
            "The process cannot access the file because it is being used by another process"
        ));
        assert!(!is_locked_error_message("database disk image is malformed"));
    }

    #[test]
    fn session_index_cleanup_preserves_live_and_unknown_records_and_checks_snapshot() {
        let root = test_root("index");
        fs::create_dir_all(root.join("sessions")).unwrap();
        let live = "019f480d-bbc6-7b62-8a46-99597db8bde7";
        let database_live = "019f480d-bbc6-7b62-8a46-99597db8bde8";
        let stale = "019f4e36-490e-7ae0-8e78-a8b3ab33a428";
        fs::write(
            root.join("sessions")
                .join(format!("rollout-2026-{live}.jsonl")),
            "{}\n",
        )
        .unwrap();
        let database = Connection::open(root.join("state_5.sqlite")).unwrap();
        database
            .execute_batch(&format!(
                "CREATE TABLE threads (id TEXT PRIMARY KEY);\n\
                 INSERT INTO threads VALUES ('{database_live}');"
            ))
            .unwrap();
        drop(database);
        let line = |id: &str, title: &str| {
            json!({"id": id, "thread_name": title, "updated_at": "2026-07-25T00:00:00Z"})
                .to_string()
        };
        let original = format!(
            "{}\n{}\n{}\n{{\"id\":\"future-id\",\"thread_name\":\"Future\",\"updated_at\":\"now\",\"future\":true}}\n{{\"kind\":\"future\"}}\n",
            line(live, "Live"),
            line(database_live, "Database live"),
            line(stale, "Stale")
        );
        fs::write(root.join("session_index.jsonl"), &original).unwrap();

        let preview = preview_session_index_cleanup_from_home(&root).unwrap();
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].id, stale);
        assert!(apply_session_index_cleanup_from_home(
            &root,
            "bad-snapshot",
            &[stale.to_string()],
            false
        )
        .is_err());
        let result = apply_session_index_cleanup_from_home(
            &root,
            &preview.snapshot_sha256,
            &[stale.to_string()],
            false,
        )
        .unwrap();
        assert_eq!(result.pruned_entries, 1);
        assert!(PathBuf::from(result.backup_path.as_ref().unwrap())
            .join("session_index.jsonl")
            .is_file());
        let next = fs::read_to_string(root.join("session_index.jsonl")).unwrap();
        assert!(next.contains(live));
        assert!(next.contains(database_live));
        assert!(!next.contains(stale));
        assert!(next.contains("future"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_index_preview_refuses_to_guess_when_a_database_is_corrupt() {
        let root = test_root("index-corrupt-database");
        fs::create_dir_all(root.join("sqlite")).unwrap();
        fs::write(root.join("sqlite/broken.db"), b"not sqlite").unwrap();
        fs::write(
            root.join("session_index.jsonl"),
            "{\"id\":\"thread-1\",\"thread_name\":\"Maybe live\",\"updated_at\":\"now\"}\n",
        )
        .unwrap();

        let error = preview_session_index_cleanup_from_home(&root).unwrap_err();
        assert!(error.contains("invalid session record check aborted"));
        assert!(root.join("session_index.jsonl").is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
