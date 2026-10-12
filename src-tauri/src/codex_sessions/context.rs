use super::*;
use rusqlite::OptionalExtension;
use std::fs::OpenOptions;
use std::io::Write;

const MAX_RAW_JSONL_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetCodexSessionContextRequest {
    pub(crate) session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSessionMessageItem {
    pub(crate) id: String,
    pub(crate) line_number: usize,
    pub(crate) role: String,
    pub(crate) timestamp: Option<String>,
    pub(crate) content: String,
    pub(crate) raw_type: String,
    pub(crate) model: Option<String>,
    pub(crate) call_id: Option<String>,
    #[serde(default)]
    pub(crate) signature_stripped: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSessionContextStats {
    pub(crate) total_lines: usize,
    pub(crate) message_count: usize,
    pub(crate) user_message_count: usize,
    pub(crate) assistant_message_count: usize,
    pub(crate) tool_count: usize,
    #[serde(default)]
    pub(crate) reasoning_count: usize,
    pub(crate) file_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSessionContextDetail {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) cwd: String,
    pub(crate) model_provider: String,
    pub(crate) model: Option<String>,
    pub(crate) archived: bool,
    pub(crate) updated_at_ms: Option<i64>,
    pub(crate) rollout_path: Option<String>,
    pub(crate) database_path: Option<String>,
    pub(crate) messages: Vec<CodexSessionMessageItem>,
    pub(crate) stats: CodexSessionContextStats,
    pub(crate) raw_jsonl: Option<String>,
    pub(crate) raw_jsonl_available: bool,
    pub(crate) rollout_sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSessionMessageUpdate {
    pub(crate) line_number: usize,
    pub(crate) role: Option<String>,
    pub(crate) content: Option<String>,
    pub(crate) deleted: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSessionNewMessage {
    pub(crate) role: String,
    pub(crate) content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveCodexSessionContextRequest {
    pub(crate) session_id: String,
    pub(crate) expected_rollout_sha256: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) model_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) archived: Option<bool>,
    pub(crate) message_updates: Option<Vec<CodexSessionMessageUpdate>>,
    pub(crate) new_messages: Option<Vec<CodexSessionNewMessage>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveCodexSessionContextResult {
    pub(crate) success: bool,
    pub(crate) session_id: String,
    pub(crate) backup_path: Option<String>,
    pub(crate) message: String,
}
#[derive(Debug, Clone, Default)]
struct RolloutItemSync {
    item_id: String,
    turn_id: String,
    item_type: String,
    text: String,
}

#[derive(Debug, Clone, Default)]
struct RolloutSyncPlan {
    updated_items: Vec<RolloutItemSync>,
    deleted_item_keys: HashSet<(String, String)>,
    new_items: Vec<RolloutItemSync>,
    /// `None` when no rollout was rewritten, so an existing projection cursor is
    /// left untouched instead of being reset to zero.
    projection_cursor: Option<RolloutProjectionCursor>,
    original_projection_cursor: Option<RolloutProjectionCursor>,
    /// Fallback ordinal for a projected item without a canonical rollout copy.
    appended_ordinal: i64,
    /// Rollout ordinal each projected item already carries, so the local history
    /// rows keep the position the rollout stores instead of a re-sequenced line
    /// number.
    item_ordinals: HashMap<(String, String), i64>,
    turn_positions: HashMap<String, RolloutTurnPosition>,
}

#[derive(Debug, Clone)]
struct RolloutTurnPosition {
    ordinal: i64,
    byte_offset: i64,
    end_ordinal: Option<i64>,
    end_byte_offset: Option<i64>,
    status: &'static str,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    duration_ms: Option<i64>,
    error_json: Option<String>,
    root_turn_id: Option<String>,
}

/// Byte offset and next ordinal the local history projection must resume from
/// after a rollout rewrite.
#[derive(Debug, Clone, Copy)]
struct RolloutProjectionCursor {
    byte_offset: i64,
    next_ordinal: i64,
}

#[tauri::command]
pub(crate) async fn get_codex_session_context(
    app: tauri::AppHandle,
    request: GetCodexSessionContextRequest,
) -> Result<CodexSessionContextDetail, String> {
    let user_home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let codex_home = resolve_codex_home(&user_home);
    tauri::async_runtime::spawn_blocking(move || {
        get_codex_session_context_from_home(&codex_home, &request.session_id)
    })
    .await
    .map_err(|error| format!("Failed to read Codex session context: {error}"))?
}

#[tauri::command]
pub(crate) async fn save_codex_session_context(
    app: tauri::AppHandle,
    request: SaveCodexSessionContextRequest,
) -> Result<SaveCodexSessionContextResult, String> {
    let user_home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let codex_home = resolve_codex_home(&user_home);
    tauri::async_runtime::spawn_blocking(move || {
        save_codex_session_context_from_home(&codex_home, request)
    })
    .await
    .map_err(|error| format!("Failed to save Codex session context: {error}"))?
}

#[tauri::command]
pub(crate) async fn open_codex_session_rollout(
    app: tauri::AppHandle,
    request: GetCodexSessionContextRequest,
) -> Result<(), String> {
    let user_home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let codex_home = resolve_codex_home(&user_home);
    let session_id = validate_session_id(&normalize_thread_id(&request.session_id))?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        find_rollout_for_session(&codex_home, &session_id)?
            .ok_or_else(|| "This session has no rollout file to open".to_string())
    })
    .await
    .map_err(|error| format!("Failed to locate session rollout: {error}"))??;
    tauri_plugin_opener::open_path(path, None::<&str>)
        .map_err(|error| format!("Failed to open rollout with the default application: {error}"))
}

fn discover_thread_history_databases(codex_home: &Path) -> Vec<PathBuf> {
    let sqlite_home = resolve_sqlite_home(codex_home);
    let mut candidates = Vec::new();
    let scan_directories = [
        codex_home.to_path_buf(),
        sqlite_home.clone(),
        sqlite_home.join("sqlite"),
    ];
    for directory in scan_directories {
        if let Ok(entries) = fs::read_dir(&directory) {
            for entry in entries.flatten() {
                let candidate_path = entry.path();
                if candidate_path.is_file() && sqlite_candidate(&candidate_path) {
                    if let Some(file_name) = candidate_path.file_name().and_then(OsStr::to_str) {
                        if file_name.starts_with("thread_history")
                            && !candidates.contains(&candidate_path)
                        {
                            candidates.push(candidate_path);
                        }
                    }
                }
            }
        }
    }
    candidates.sort();
    candidates
}

fn acquire_session_writer_lock(codex_home: &Path, session_id: &str) -> Result<fs::File, String> {
    let lock_directory = codex_home.join("thread-writer-locks");
    ensure_context_path_in_home(codex_home, &lock_directory)?;
    fs::create_dir_all(&lock_directory).map_err(|error| {
        format!(
            "Failed to create session writer lock directory {}: {error}",
            lock_directory.display()
        )
    })?;
    // Codex coordinates lock-file cleanup and rollout publication before opening
    // the per-thread lock. Honor that same order to avoid locking a stale inode.
    let _coordination = acquire_context_lock_file(
        codex_home,
        &lock_directory.join(".coordination.lock"),
        "Codex writer coordination is busy. Retry after current writes finish",
    )?;
    let lock_path = lock_directory.join(format!("{session_id}.lock"));
    acquire_context_lock_file(
        codex_home, &lock_path,
        &format!("Session {session_id} is currently active. Close the session before editing its context"),
    )
}

fn acquire_context_lock_file(
    codex_home: &Path,
    lock_path: &Path,
    busy_message: &str,
) -> Result<fs::File, String> {
    ensure_context_path_in_home(codex_home, lock_path)?;
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
    {
        Ok(file) => file,
        Err(error) if is_locked_io_error(&error) => {
            return Err(busy_message.to_string());
        }
        Err(error) => {
            return Err(format!(
                "Failed to verify the session writer lock {}: {error}",
                lock_path.display()
            ));
        }
    };
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(busy_message.to_string()),
        Err(std::fs::TryLockError::Error(error)) if is_locked_io_error(&error) => {
            Err(busy_message.to_string())
        }
        Err(std::fs::TryLockError::Error(error)) => Err(format!(
            "Failed to verify the session writer lock {}: {error}",
            lock_path.display()
        )),
    }
}

fn validate_session_id(id: &str) -> Result<String, String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("Session ID cannot be empty".to_string());
    }
    if trimmed.len() > 128 {
        return Err("Session ID exceeds maximum length".to_string());
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || trimmed.contains(':')
    {
        return Err("Session ID contains invalid path characters".to_string());
    }
    if !trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
    {
        return Err("Session ID contains disallowed characters".to_string());
    }
    Ok(trimmed.to_string())
}

fn extract_content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(arr) => {
            let mut parts = Vec::new();
            for item in arr {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(text);
                } else if let Some(text) = item.as_str() {
                    parts.push(text);
                }
            }
            parts.join("\n")
        }
        Value::Object(obj) => obj
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

fn extract_reasoning_text(payload: &Value) -> String {
    if let Some(text) = payload.get("thinking").and_then(Value::as_str) {
        return text.to_string();
    }
    for field in ["summary_text", "raw_content", "summary", "content"] {
        if let Some(value) = payload.get(field) {
            let text = extract_content_text(value);
            if !text.is_empty() {
                return text;
            }
        }
    }
    if payload.get("encrypted_content").is_some() {
        return String::from("[Encrypted reasoning block]");
    }
    String::new()
}

fn message_content_type(role: Option<&str>) -> &'static str {
    if role == Some("assistant") {
        "output_text"
    } else {
        "input_text"
    }
}

fn is_tool_response(payload_type: &str) -> bool {
    matches!(payload_type, "function_call" | "function_call_output")
        || payload_type.contains("tool")
}

fn update_content_text(content: &mut Value, new_text: &str, content_type: &str) {
    match content {
        Value::String(_) => {
            *content = Value::String(new_text.to_string());
        }
        Value::Array(arr) => {
            let original = std::mem::take(arr);
            let mut replacement = Vec::with_capacity(original.len());
            let mut updated = false;
            for mut item in original {
                if item.is_string() {
                    if updated {
                        continue;
                    }
                    replacement.push(json!(new_text));
                    updated = true;
                    continue;
                }
                if let Some(obj) = item.as_object_mut() {
                    if obj.contains_key("text") {
                        if updated {
                            continue;
                        }
                        obj.insert("text".to_string(), json!(new_text));
                        if obj.contains_key("text_elements") {
                            obj.insert("text_elements".to_string(), json!([]));
                        }
                        if obj.contains_key("type") {
                            obj.insert("type".to_string(), json!(content_type));
                        }
                        updated = true;
                    }
                }
                replacement.push(item);
            }
            if updated {
                *arr = replacement;
            } else {
                replacement.push(json!({"type": content_type, "text": new_text}));
                *arr = replacement;
            }
        }
        Value::Object(object) if object.contains_key("text") => {
            object.insert("text".to_string(), json!(new_text));
            if object.contains_key("text_elements") {
                object.insert("text_elements".to_string(), json!([]));
            }
            if object.contains_key("type") {
                object.insert("type".to_string(), json!(content_type));
            }
        }
        _ => {
            *content = json!([{"type": content_type, "text": new_text}]);
        }
    }
}

fn validate_raw_jsonl(raw_jsonl: &str, session_id: &str) -> Result<Vec<(usize, Value)>, String> {
    let mut records = Vec::new();
    let mut matching_session_meta = false;
    for (line_idx, line) in raw_jsonl.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let record = serde_json::from_str::<Value>(trimmed)
            .map_err(|error| format!("Validation error at line {}: {error}", line_idx + 1))?;
        if !record.is_object() {
            return Err(format!(
                "Validation error at line {}: Each JSONL record must be an object",
                line_idx + 1
            ));
        }
        if record.get("type").and_then(Value::as_str) == Some("session_meta") {
            let record_id = record
                .get("payload")
                .and_then(|payload| payload.get("id"))
                .and_then(Value::as_str);
            if record_id != Some(session_id) {
                return Err(format!(
                    "Validation error at line {}: session_meta belongs to a different session",
                    line_idx + 1
                ));
            }
            matching_session_meta = true;
        }
        records.push((line_idx, record));
    }
    if records.is_empty() {
        return Err("Raw JSONL cannot be empty".to_string());
    }
    if !matching_session_meta {
        return Err(format!(
            "Raw JSONL does not contain session_meta for session {session_id}"
        ));
    }
    Ok(records)
}

fn apply_rollout_metadata(record: &mut Value, request: &SaveCodexSessionContextRequest) {
    let record_type = record.get("type").and_then(Value::as_str);
    if record_type == Some("session_meta") {
        if let Some(payload) = record.get_mut("payload").and_then(Value::as_object_mut) {
            if let Some(title) = request.title.as_ref() {
                payload.insert("title".to_string(), json!(title));
            }
            if let Some(cwd) = request.cwd.as_ref() {
                payload.insert("cwd".to_string(), json!(cwd));
            }
            if let Some(provider) = request.model_provider.as_ref() {
                payload.insert("model_provider".to_string(), json!(provider));
            }
        }
    } else if record_type == Some("turn_context") {
        if let Some(payload) = record.get_mut("payload").and_then(Value::as_object_mut) {
            if let Some(cwd) = request.cwd.as_ref() {
                payload.insert("cwd".to_string(), json!(cwd));
            }
            if let Some(model) = request.model.as_ref() {
                payload.insert("model".to_string(), json!(model));
            }
        }
    }
}

fn serialize_jsonl(records: &[Value], line_ending: &str) -> Result<Vec<u8>, String> {
    let mut output = String::new();
    for record in records {
        output.push_str(
            &serde_json::to_string(record)
                .map_err(|error| format!("Failed to serialize rollout record: {error}"))?,
        );
        output.push_str(line_ending);
    }
    Ok(output.into_bytes())
}

fn find_rollout_for_session(
    codex_home: &Path,
    session_id: &str,
) -> Result<Option<PathBuf>, String> {
    let (database_paths, _) = discover_database_paths(codex_home, false);
    for db_path in &database_paths {
        if let Ok(conn) = open_read_only(db_path) {
            if let Ok(true) = table_exists(&conn, "threads") {
                if let Ok(mut stmt) =
                    conn.prepare("SELECT rollout_path FROM threads WHERE id = ?1 LIMIT 1")
                {
                    if let Ok(mut rows) = stmt.query([session_id]) {
                        if let Ok(Some(row)) = rows.next() {
                            if let Ok(Some(path_str)) = row.get::<_, Option<String>>(0) {
                                let path = PathBuf::from(&path_str);
                                if let Some(validated) = validated_rollout_path(codex_home, &path)?
                                {
                                    return Ok(Some(validated));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let Ok(rollout_files) = collect_rollout_files(codex_home) {
        for path in &rollout_files {
            if let Some(id) = rollout_thread_id_from_file_name(path) {
                if id == session_id {
                    return validated_rollout_path(codex_home, path);
                }
            }
        }
    }
    Ok(None)
}

fn prune_context_edit_backups(codex_home: &Path) -> Result<(), String> {
    let root = codex_home
        .join("backups_state")
        .join("easy-cli-proxy-api")
        .join("session-context-edits");
    if !root.is_dir() {
        return Ok(());
    }
    ensure_context_path_in_home(codex_home, &root)?;
    let mut directories = fs::read_dir(&root)
        .map_err(|error| format!("Failed to read context edit backup directory: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_dir()))
        .collect::<Vec<_>>();
    directories.sort_by_key(|entry| entry.file_name());
    let keep_count = 10;
    let remove_count = directories.len().saturating_sub(keep_count);
    for entry in directories.into_iter().take(remove_count) {
        let _ = fs::remove_dir_all(entry.path());
    }
    Ok(())
}

fn find_session_database_path(
    codex_home: &Path,
    session_id: &str,
) -> Result<Option<PathBuf>, String> {
    let (database_paths, warnings) = discover_database_paths(codex_home, false);
    if !warnings.is_empty() {
        return Err(format!(
            "Unable to safely inspect all Codex session databases; saving aborted: {}",
            warnings.join("; ")
        ));
    }
    let mut matches = Vec::new();
    for path in database_paths {
        let connection = open_read_only(&path)?;
        if !table_columns(&connection, "threads")?.contains("id") {
            continue;
        }
        let exists = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM threads WHERE id = ?1)",
                [session_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| {
                format!(
                    "Failed to locate session {session_id} in {}: {error}",
                    path.display()
                )
            })?
            != 0;
        if exists {
            matches.push(path);
        }
    }
    if matches.len() > 1 {
        return Err(format!(
            "Session {session_id} exists in multiple Codex databases; refusing an ambiguous update"
        ));
    }
    Ok(matches.pop())
}

fn rollout_archive_target(
    codex_home: &Path,
    current_path: &Path,
    archived: Option<bool>,
) -> Result<PathBuf, String> {
    let Some(archived) = archived else {
        return Ok(current_path.to_path_buf());
    };
    let archived_root = validated_session_root(codex_home, "archived_sessions")?;
    let sessions_root = validated_session_root(codex_home, "sessions")?;
    let current_is_archived = is_archived_rollout_path(codex_home, current_path);
    if archived == current_is_archived {
        return Ok(current_path.to_path_buf());
    }
    let file_name = current_path
        .file_name()
        .ok_or_else(|| format!("Rollout path has no file name: {}", current_path.display()))?;
    if archived {
        return Ok(archived_root.join(file_name));
    }
    let name = file_name.to_string_lossy();
    let date = name
        .strip_prefix("rollout-")
        .and_then(|value| value.get(..10))
        .filter(|value| {
            value
                .chars()
                .enumerate()
                .all(|(index, character)| match index {
                    4 | 7 => character == '-',
                    _ => character.is_ascii_digit(),
                })
        });
    let target_dir = date.map_or(sessions_root.clone(), |date| {
        sessions_root
            .join(&date[0..4])
            .join(&date[5..7])
            .join(&date[8..10])
    });
    Ok(target_dir.join(file_name))
}

fn is_archived_rollout_path(codex_home: &Path, path: &Path) -> bool {
    let Ok(archived_root) = fs::canonicalize(codex_home.join("archived_sessions")) else {
        return false;
    };
    path.starts_with(archived_root)
}

fn validated_session_root(codex_home: &Path, directory: &str) -> Result<PathBuf, String> {
    let home = fs::canonicalize(codex_home).map_err(|error| {
        format!(
            "Failed to resolve Codex home {}: {error}",
            codex_home.display()
        )
    })?;
    let root = codex_home.join(directory);
    match fs::symlink_metadata(&root) {
        Ok(_) => {
            let canonical = fs::canonicalize(&root).map_err(|error| {
                format!(
                    "Failed to resolve session directory {}: {error}",
                    root.display()
                )
            })?;
            if !canonical.starts_with(&home) {
                return Err(format!(
                    "Session directory {} resolves outside Codex home",
                    root.display()
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Failed to inspect session directory {}: {error}",
                root.display()
            ));
        }
    }
    Ok(root)
}

fn ensure_context_path_in_home(codex_home: &Path, path: &Path) -> Result<(), String> {
    let home = fs::canonicalize(codex_home)
        .map_err(|error| format!("Failed to resolve Codex home: {error}"))?;
    let mut ancestor = path;
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let canonical = fs::canonicalize(ancestor).map_err(|error| {
                    format!(
                        "Failed to resolve context path {}: {error}",
                        ancestor.display()
                    )
                })?;
                if !canonical.starts_with(&home) {
                    return Err(format!(
                        "Context path {} resolves outside Codex home",
                        path.display()
                    ));
                }
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| "Context path has no existing parent".to_string())?;
            }
            Err(error) => {
                return Err(format!(
                    "Failed to inspect context path {}: {error}",
                    ancestor.display()
                ))
            }
        }
    }
}

fn create_context_edit_backup(
    codex_home: &Path,
    session_id: &str,
    rollout_path: Option<&Path>,
    rollout_bytes: Option<&[u8]>,
    database_path: Option<&Path>,
    session_index_bytes: Option<&[u8]>,
    thread_history_databases: &[PathBuf],
) -> Result<PathBuf, String> {
    ensure_context_path_in_home(
        codex_home,
        &codex_home.join("backups_state/easy-cli-proxy-api/session-context-edits"),
    )?;
    let directory = create_operation_directory(codex_home, "session-context-edits", session_id)?;
    let mut rollout_backup = Value::Null;
    if let (Some(path), Some(bytes)) = (rollout_path, rollout_bytes) {
        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("rollout.jsonl");
        let relative = PathBuf::from("rollouts").join(file_name);
        let target = directory.join(&relative);
        fs::create_dir_all(target.parent().unwrap_or(&directory))
            .map_err(|error| format!("Failed to create context backup directory: {error}"))?;
        crate::write_bytes_atomically(&target, bytes)?;
        rollout_backup = json!({
            "originalPath": path.to_string_lossy(),
            "backupPath": relative.to_string_lossy().replace('\\', "/"),
            "sha256": sha256_hex(bytes)
        });
    }
    let mut database_backup = Value::Null;
    if let Some(path) = database_path {
        let connection = open_read_only(path)?;
        let mut tables = Map::new();
        for (table, key) in [("threads", "id"), ("local_thread_catalog", "thread_id")] {
            let rows = select_rows_if_supported(
                &connection,
                table,
                &[key],
                &format!("{key} = ?1"),
                &[&session_id],
            )?;
            if !rows.is_empty() {
                tables.insert(table.to_string(), Value::Array(rows));
            }
        }
        database_backup = json!({
            "databasePath": path.to_string_lossy(),
            "tables": tables
        });
        write_json_atomically(&directory.join("database-rows.json"), &database_backup)?;
    }
    if !thread_history_databases.is_empty() {
        let mut history_backups = Vec::new();
        for database_path in thread_history_databases {
            let connection = open_read_only(database_path)?;
            let mut tables_map = Map::new();
            for (table_name, column_name) in [
                ("thread_items", "thread_id"),
                ("thread_turns", "thread_id"),
                ("thread_history_projection_state", "thread_id"),
            ] {
                let rows = select_rows_if_supported(
                    &connection,
                    table_name,
                    &[column_name],
                    &format!("{column_name} = ?1"),
                    &[&session_id],
                )
                .map_err(|error| {
                    format!(
                        "Failed to back up {table_name} from {}: {error}",
                        database_path.display()
                    )
                })?;
                if !rows.is_empty() {
                    tables_map.insert(table_name.to_string(), Value::Array(rows));
                }
            }
            history_backups.push(json!({
                "databasePath": database_path.to_string_lossy(),
                "tables": tables_map
            }));
        }
        if !history_backups.is_empty() {
            write_json_atomically(
                &directory.join("thread-history-rows.json"),
                &Value::Array(history_backups),
            )?;
        }
    }
    if let Some(bytes) = session_index_bytes {
        crate::write_bytes_atomically(&directory.join("session_index.jsonl.bak"), bytes)?;
    }
    write_json_atomically(
        &directory.join("metadata.json"),
        &json!({
            "schemaVersion": 1,
            "kind": "session-context-edit",
            "createdAt": Utc::now().to_rfc3339(),
            "sessionId": session_id,
            "rollout": rollout_backup,
            "database": database_backup,
            "threadHistoryBackedUp": !thread_history_databases.is_empty(),
            "sessionIndexBackedUp": session_index_bytes.is_some(),
            "managedBy": "EasyCLIProxyAPI"
        }),
    )?;
    Ok(directory)
}

fn sqlite_param_for_json(value: Option<&Value>) -> Box<dyn ToSql> {
    match value {
        Some(Value::String(text)) => Box::new(text.clone()),
        Some(Value::Number(number)) => {
            if let Some(integer) = number.as_i64() {
                Box::new(integer)
            } else if let Some(float) = number.as_f64() {
                Box::new(float)
            } else {
                Box::new(None::<String>)
            }
        }
        Some(Value::Bool(flag)) => Box::new(*flag),
        _ => Box::new(None::<String>),
    }
}

/// Restores the `threads` / `local_thread_catalog` rows captured before the
/// save. `update_session_database` commits before the later rollout-index and
/// thread-history steps run, so a late failure must roll this back as well.
fn restore_session_database_backup(backup_dir: &Path) -> Result<(), String> {
    let backup_file = backup_dir.join("database-rows.json");
    if !backup_file.is_file() {
        return Ok(());
    }
    let content = fs::read_to_string(&backup_file).map_err(|error| {
        format!(
            "Failed to read database backup {}: {error}",
            backup_file.display()
        )
    })?;
    let entry: Value = serde_json::from_str(&content).map_err(|error| {
        format!(
            "Failed to parse database backup {}: {error}",
            backup_file.display()
        )
    })?;
    let Some(db_path) = entry.get("databasePath").and_then(Value::as_str) else {
        return Ok(());
    };
    let db_path = PathBuf::from(db_path);
    if !db_path.exists() {
        return Ok(());
    }
    let mut connection = open_read_write(&db_path)?;
    let transaction = connection.transaction().map_err(|error| {
        format!(
            "Failed to start the session database rollback for {}: {error}",
            db_path.display()
        )
    })?;
    if let Some(tables) = entry.get("tables").and_then(Value::as_object) {
        for (table_name, rows_value) in tables {
            if !table_exists(&transaction, table_name)? {
                continue;
            }
            let Some(rows) = rows_value.as_array() else {
                continue;
            };
            for row in rows {
                let Some(row_map) = row.as_object() else {
                    continue;
                };
                let columns: Vec<&str> = row_map.keys().map(String::as_str).collect();
                let placeholders: Vec<String> =
                    (1..=columns.len()).map(|idx| format!("?{idx}")).collect();
                let sql = format!(
                    "INSERT OR REPLACE INTO \"{}\" ({}) VALUES ({})",
                    table_name.replace('"', "\"\""),
                    columns.join(", "),
                    placeholders.join(", ")
                );
                let params: Vec<Box<dyn ToSql>> = columns
                    .iter()
                    .map(|column| sqlite_param_for_json(row_map.get(*column)))
                    .collect();
                let refs: Vec<&dyn ToSql> = params.iter().map(|item| item.as_ref()).collect();
                transaction
                    .execute(&sql, refs.as_slice())
                    .map_err(|error| {
                        format!(
                            "Failed to restore {table_name} in {} during rollback: {error}",
                            db_path.display()
                        )
                    })?;
            }
        }
    }
    transaction.commit().map_err(|error| {
        format!(
            "Failed to commit the session database rollback for {}: {error}",
            db_path.display()
        )
    })
}

fn restore_thread_history_backup(backup_dir: &Path, session_id: &str) -> Result<(), String> {
    let backup_file = backup_dir.join("thread-history-rows.json");
    if !backup_file.is_file() {
        return Ok(());
    }
    let content = fs::read_to_string(&backup_file).map_err(|error| {
        format!(
            "Failed to read thread history backup {}: {error}",
            backup_file.display()
        )
    })?;
    let entries: Vec<Value> = serde_json::from_str(&content).map_err(|error| {
        format!(
            "Failed to parse thread history backup {}: {error}",
            backup_file.display()
        )
    })?;

    for entry in entries {
        let db_path_str = match entry.get("databasePath").and_then(Value::as_str) {
            Some(path) => path,
            None => continue,
        };
        let db_path = PathBuf::from(db_path_str);
        if !db_path.exists() {
            continue;
        }
        let mut connection = open_read_write(&db_path)?;
        let transaction = connection.transaction().map_err(|error| {
            format!(
                "Failed to start rollback transaction for {}: {error}",
                db_path.display()
            )
        })?;

        for table_name in [
            "thread_items",
            "thread_turns",
            "thread_history_projection_state",
        ] {
            if !table_exists(&transaction, table_name)? {
                continue;
            }
            transaction
                .execute(
                    &format!("DELETE FROM \"{table_name}\" WHERE thread_id = ?1"),
                    [session_id],
                )
                .map_err(|error| {
                    format!(
                        "Failed to clear {table_name} in {} during rollback: {error}",
                        db_path.display()
                    )
                })?;
        }

        if let Some(tables) = entry.get("tables").and_then(Value::as_object) {
            for (table_name, rows_val) in tables {
                if let Some(rows) = rows_val.as_array() {
                    for row in rows {
                        if let Some(row_map) = row.as_object() {
                            if !table_exists(&transaction, table_name)? {
                                continue;
                            }
                            let columns: Vec<&str> = row_map.keys().map(String::as_str).collect();
                            let placeholders: Vec<String> =
                                (1..=columns.len()).map(|idx| format!("?{idx}")).collect();
                            let sql = format!(
                                "INSERT OR REPLACE INTO \"{}\" ({}) VALUES ({})",
                                table_name.replace('"', "\"\""),
                                columns.join(", "),
                                placeholders.join(", ")
                            );
                            let params_vec: Vec<Box<dyn ToSql>> = columns
                                .iter()
                                .map(|col| sqlite_param_for_json(row_map.get(*col)))
                                .collect();
                            let refs: Vec<&dyn ToSql> =
                                params_vec.iter().map(|item| item.as_ref()).collect();
                            transaction.execute(&sql, refs.as_slice()).map_err(|error| {
                                format!(
                                    "Failed to restore {table_name} rows in {} during rollback: {error}",
                                    db_path.display()
                                )
                            })?;
                        }
                    }
                }
            }
        }
        transaction.commit().map_err(|error| {
            format!(
                "Failed to commit rollback transaction for {}: {error}",
                db_path.display()
            )
        })?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackedItemKind {
    UserMessage,
    AgentMessage,
    Reasoning,
}

fn response_item_retained_id(record: &Value) -> Option<String> {
    record
        .get("metadata")
        .and_then(|metadata| metadata.get("retained_source"))
        .and_then(|source| source.get("id"))
        .and_then(|id_object| id_object.get("message_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn tracked_item_text(item: &Value, kind: TrackedItemKind) -> String {
    if kind == TrackedItemKind::Reasoning {
        return extract_reasoning_text(item);
    }
    if let Some(text) = item.get("text").and_then(Value::as_str) {
        if !text.is_empty() {
            return text.to_string();
        }
    }
    extract_content_text(item.get("content").unwrap_or(&Value::Null))
}

fn response_item_text(record: &Value) -> Option<String> {
    if record.get("type").and_then(Value::as_str) != Some("response_item") {
        return None;
    }
    let payload = record.get("payload")?;
    match payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "message" => Some(extract_content_text(
            payload.get("content").unwrap_or(&Value::Null),
        )),
        "reasoning" | "thinking" => Some(extract_reasoning_text(payload)),
        _ => None,
    }
}

/// Writes reasoning text back into the field shape the record already used so a
/// re-read returns the edited value instead of the stale original.
fn write_reasoning_text(payload: &mut Map<String, Value>, text: &str) {
    if payload.get("thinking").and_then(Value::as_str).is_some() {
        payload.insert("thinking".to_string(), json!(text));
        return;
    }
    if payload.contains_key("summary") {
        payload.insert(
            "summary".to_string(),
            json!([{ "type": "summary_text", "text": text }]),
        );
        if payload.contains_key("content") {
            payload.insert("content".to_string(), json!([]));
        }
        return;
    }
    if payload.contains_key("content") {
        payload.insert(
            "content".to_string(),
            json!([{ "type": "text", "text": text }]),
        );
        return;
    }
    payload.insert(
        "summary".to_string(),
        json!([{ "type": "summary_text", "text": text }]),
    );
}

/// One physical copy of a projected item inside a rollout. Codex stores every
/// message twice, as a `response_item` plus an `event_msg/item_completed`
/// mirror, so one logical item can own several copies.
#[derive(Debug, Clone)]
struct TrackedCopy {
    line_index: usize,
    kind: TrackedItemKind,
    text: String,
    is_event_copy: bool,
    is_legacy_copy: bool,
    turn_id: String,
    explicit_id: Option<String>,
}

/// Every copy of one logical item, keyed by the stable item id when the rollout
/// exposes one. Codex records without `retained_source` metadata inherit the id
/// of the duplicated copy they belong to.
#[derive(Debug, Clone)]
struct TrackedGroup {
    id: Option<String>,
    kind: TrackedItemKind,
    copies: Vec<TrackedCopy>,
}

impl TrackedGroup {
    fn first_line_index(&self) -> usize {
        self.copies
            .iter()
            .map(|copy| copy.line_index)
            .min()
            .unwrap_or_default()
    }
}

fn non_empty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn tracked_kind_from_item_type(item_type: &str) -> Option<TrackedItemKind> {
    match item_type {
        "UserMessage" => Some(TrackedItemKind::UserMessage),
        "AgentMessage" => Some(TrackedItemKind::AgentMessage),
        "Reasoning" | "reasoning" => Some(TrackedItemKind::Reasoning),
        _ => None,
    }
}

fn tracked_kind_from_role(role: &str) -> Option<TrackedItemKind> {
    match role {
        "user" => Some(TrackedItemKind::UserMessage),
        "assistant" => Some(TrackedItemKind::AgentMessage),
        _ => None,
    }
}

fn tracked_copies(records: &[Value]) -> Vec<TrackedCopy> {
    let mut copies = Vec::new();
    let mut current_turn_id = String::new();
    for (line_index, record) in records.iter().enumerate() {
        let record_type = record
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(payload) = record.get("payload") else {
            continue;
        };
        if record_type == "event_msg" {
            let event_type = payload
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if event_type == "task_started" {
                if let Some(turn_id) = payload.get("turn_id").and_then(Value::as_str) {
                    if !turn_id.is_empty() {
                        current_turn_id = turn_id.to_string();
                    }
                }
                continue;
            }
            let turn_id = payload
                .get("turn_id")
                .and_then(Value::as_str)
                .filter(|turn_id| !turn_id.is_empty())
                .unwrap_or(current_turn_id.as_str())
                .to_string();
            if matches!(event_type, "user_message" | "agent_message") {
                copies.push(TrackedCopy {
                    line_index,
                    kind: if event_type == "user_message" {
                        TrackedItemKind::UserMessage
                    } else {
                        TrackedItemKind::AgentMessage
                    },
                    text: payload["message"].as_str().unwrap_or_default().to_string(),
                    is_event_copy: true,
                    is_legacy_copy: true,
                    turn_id,
                    explicit_id: None,
                });
                continue;
            }
            if event_type != "item_completed" {
                continue;
            }
            let Some(item) = payload.get("item") else {
                continue;
            };
            let Some(kind) = tracked_kind_from_item_type(
                item.get("type").and_then(Value::as_str).unwrap_or_default(),
            ) else {
                continue;
            };
            copies.push(TrackedCopy {
                line_index,
                kind,
                text: tracked_item_text(item, kind),
                is_event_copy: true,
                is_legacy_copy: false,
                turn_id,
                explicit_id: non_empty_string(item.get("id")),
            });
            continue;
        }
        if record_type != "response_item" {
            continue;
        }
        let retained_turn_id = record
            .get("metadata")
            .and_then(|metadata| metadata.get("retained_source"))
            .and_then(|source| source.get("id"))
            .and_then(|id_object| id_object.get("turn_id"))
            .and_then(Value::as_str)
            .filter(|turn_id| !turn_id.is_empty());
        let turn_id = retained_turn_id
            .unwrap_or(current_turn_id.as_str())
            .to_string();
        match payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "message" => {
                let Some(kind) = tracked_kind_from_role(
                    payload
                        .get("role")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ) else {
                    continue;
                };
                copies.push(TrackedCopy {
                    line_index,
                    kind,
                    text: tracked_item_text(payload, kind),
                    is_event_copy: false,
                    is_legacy_copy: false,
                    turn_id,
                    explicit_id: response_item_retained_id(record),
                });
            }
            "reasoning" | "thinking" => copies.push(TrackedCopy {
                line_index,
                kind: TrackedItemKind::Reasoning,
                text: tracked_item_text(payload, TrackedItemKind::Reasoning),
                is_event_copy: false,
                is_legacy_copy: false,
                turn_id,
                explicit_id: non_empty_string(payload.get("id"))
                    .or_else(|| response_item_retained_id(record)),
            }),
            _ => {}
        }
    }
    copies
}

/// Groups identified copies by (item id, turn); id-less mirrors still match by
/// text and then order within the turn.
fn group_tracked_copies(copies: Vec<TrackedCopy>) -> Vec<TrackedGroup> {
    let mut groups: Vec<TrackedGroup> = Vec::new();
    let mut group_index_by_key: HashMap<(String, String), usize> = HashMap::new();
    let mut orphans: Vec<TrackedCopy> = Vec::new();

    for copy in copies {
        match copy.explicit_id.clone() {
            Some(id) => {
                let key = (id.clone(), copy.turn_id.clone());
                match group_index_by_key.get(&key).copied() {
                    Some(index) => groups[index].copies.push(copy),
                    None => {
                        group_index_by_key.insert(key, groups.len());
                        groups.push(TrackedGroup {
                            id: Some(id),
                            kind: copy.kind,
                            copies: vec![copy],
                        });
                    }
                }
            }
            None => orphans.push(copy),
        }
    }

    for orphan in orphans {
        let mut text_match: Option<usize> = None;
        let mut order_match: Option<usize> = None;
        for (index, group) in groups.iter().enumerate() {
            if group.kind != orphan.kind {
                continue;
            }
            if group.copies.iter().any(|copy| {
                copy.is_event_copy == orphan.is_event_copy
                    && copy.is_legacy_copy == orphan.is_legacy_copy
            }) {
                continue;
            }
            if !orphan.turn_id.is_empty()
                && group
                    .copies
                    .iter()
                    .all(|copy| copy.turn_id != orphan.turn_id)
            {
                continue;
            }
            if group.copies.iter().any(|copy| copy.text == orphan.text) {
                text_match = Some(index);
                break;
            }
            if order_match.is_none() {
                order_match = Some(index);
            }
        }
        match text_match.or(order_match) {
            Some(index) => groups[index].copies.push(orphan),
            None => {
                let kind = orphan.kind;
                groups.push(TrackedGroup {
                    id: None,
                    kind,
                    copies: vec![orphan],
                });
            }
        }
    }

    for group in groups.iter_mut() {
        group.copies.sort_by_key(|copy| copy.line_index);
    }
    groups.sort_by_key(TrackedGroup::first_line_index);
    groups
}

fn group_turn_id(group: &TrackedGroup, fallback: &str) -> String {
    group
        .copies
        .iter()
        .map(|copy| copy.turn_id.as_str())
        .find(|turn| !turn.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn group_ordinal(records: &[Value], group: &TrackedGroup) -> Option<i64> {
    let copy = group
        .copies
        .iter()
        .find(|copy| copy.is_event_copy && !copy.is_legacy_copy)
        .or_else(|| group.copies.iter().find(|copy| !copy.is_event_copy))
        .or_else(|| group.copies.first())?;
    Some(
        records
            .get(copy.line_index)?
            .get("ordinal")
            .and_then(Value::as_i64)
            .unwrap_or(copy.line_index as i64),
    )
}

/// Resolve item positions after missing ordinals have been stamped.
fn collect_projection_positions(records: &[Value], serialized: &[u8], plan: &mut RolloutSyncPlan) {
    let fallback_turn_id = rollout_last_turn_id(records, "");
    for group in group_tracked_copies(tracked_copies(records)) {
        if let (Some(id), Some(ordinal)) = (group.id.as_ref(), group_ordinal(records, &group)) {
            let turn_id = group_turn_id(&group, &fallback_turn_id);
            plan.item_ordinals.insert((id.clone(), turn_id), ordinal);
        }
    }
    let mut byte_offset = 0;
    let mut current_turn = String::new();
    for (index, (record, line)) in records
        .iter()
        .zip(serialized.split_inclusive(|byte| *byte == b'\n'))
        .enumerate()
    {
        let end_offset = byte_offset + line.len() as i64;
        let ordinal = record
            .get("ordinal")
            .and_then(Value::as_i64)
            .unwrap_or(index as i64);
        if record.get("type").and_then(Value::as_str) == Some("event_msg") {
            if let Some(payload) = record.get("payload") {
                let event_type = payload
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let turn_id = payload
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .unwrap_or(&current_turn)
                    .to_string();
                if event_type == "task_started" {
                    current_turn = turn_id.clone();
                }
                if !turn_id.is_empty()
                    && matches!(
                        event_type,
                        "task_started" | "item_completed" | "task_complete" | "turn_aborted"
                    )
                {
                    let position =
                        plan.turn_positions
                            .entry(turn_id.clone())
                            .or_insert(RolloutTurnPosition {
                                ordinal,
                                byte_offset,
                                end_ordinal: None,
                                end_byte_offset: None,
                                status: "inProgress",
                                started_at: None,
                                completed_at: None,
                                duration_ms: None,
                                error_json: None,
                                root_turn_id: None,
                            });
                    position.started_at = payload["started_at"].as_i64().or(position.started_at);
                    position.root_turn_id = non_empty_string(payload.get("root_turn_id"))
                        .or_else(|| position.root_turn_id.clone());
                    if matches!(event_type, "task_complete" | "turn_aborted") {
                        position.end_ordinal = Some(ordinal);
                        position.end_byte_offset = Some(end_offset);
                        position.error_json = payload
                            .get("error")
                            .filter(|error| !error.is_null())
                            .map(Value::to_string);
                        position.status = if event_type == "turn_aborted" {
                            "interrupted"
                        } else if position.error_json.is_some() {
                            "failed"
                        } else {
                            "completed"
                        };
                        position.completed_at = payload["completed_at"].as_i64();
                        position.duration_ms = payload["duration_ms"].as_i64();
                    }
                    // Tool items also have canonical completion records.
                    if event_type == "item_completed" {
                        if let Some(id) = payload
                            .get("item")
                            .and_then(|item| item.get("id"))
                            .and_then(Value::as_str)
                        {
                            plan.item_ordinals
                                .insert((id.to_string(), turn_id), ordinal);
                        }
                    }
                }
            }
        }
        byte_offset = end_offset;
    }
}

/// Each completion summarizes only the retained assistants preceding it in its turn.
fn refresh_task_complete_messages(
    records: &mut [Value],
    turns: &HashSet<String>,
    session_id: &str,
) {
    let groups = group_tracked_copies(tracked_copies(records));
    let mut agents_by_line = HashMap::new();
    for group in &groups {
        if group.kind != TrackedItemKind::AgentMessage {
            continue;
        }
        if let Some(copy) = group
            .copies
            .iter()
            .find(|copy| copy.is_event_copy && !copy.is_legacy_copy)
            .or_else(|| group.copies.iter().find(|copy| !copy.is_event_copy))
            .or_else(|| group.copies.first())
        {
            agents_by_line.insert(
                copy.line_index,
                (group_turn_id(group, session_id), copy.text.clone()),
            );
        }
    }
    let mut last_by_turn = HashMap::new();
    let mut current_turn = session_id.to_string();
    for (line_index, record) in records.iter_mut().enumerate() {
        if let Some((turn, text)) = agents_by_line.get(&line_index) {
            last_by_turn.insert(turn.clone(), text.clone());
        }
        if record.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let Some(payload) = record.get_mut("payload").and_then(Value::as_object_mut) else {
            continue;
        };
        if payload.get("type").and_then(Value::as_str) == Some("task_started") {
            if let Some(turn) = non_empty_string(payload.get("turn_id")) {
                current_turn = turn;
            }
        } else if payload.get("type").and_then(Value::as_str) == Some("task_complete") {
            let turn =
                non_empty_string(payload.get("turn_id")).unwrap_or_else(|| current_turn.clone());
            if turns.contains(&turn) {
                payload.insert(
                    "last_agent_message".to_string(),
                    last_by_turn
                        .get(&turn)
                        .cloned()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                );
            }
        }
    }
}

fn strip_reasoning_signature(object: &mut Map<String, Value>) {
    object.remove("signature");
    object.remove("encrypted_content");
    object.insert("signature_stripped".to_string(), json!(true));
}

fn rollout_last_turn_id(records: &[Value], session_id: &str) -> String {
    let mut last_turn_id = String::new();
    for record in records {
        if record.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let Some(payload) = record.get("payload") else {
            continue;
        };
        match payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "task_started" | "item_completed" | "task_complete" | "turn_aborted" => {
                if let Some(turn_id) = payload.get("turn_id").and_then(Value::as_str) {
                    if !turn_id.is_empty() {
                        last_turn_id = turn_id.to_string();
                    }
                }
            }
            _ => {}
        }
    }
    if last_turn_id.is_empty() {
        session_id.to_string()
    } else {
        last_turn_id
    }
}

fn rewrite_event_item_text(
    item_object: &mut Map<String, Value>,
    kind: TrackedItemKind,
    text: &str,
) {
    if kind == TrackedItemKind::Reasoning {
        item_object.insert("summary_text".to_string(), json!([text]));
        item_object.insert("raw_content".to_string(), json!([]));
        strip_reasoning_signature(item_object);
        return;
    }
    if item_object.contains_key("text") {
        item_object.insert("text".to_string(), json!(text));
    }
    let content = item_object.entry("content").or_insert_with(|| json!([]));
    update_content_text(
        content,
        text,
        if kind == TrackedItemKind::UserMessage {
            "text"
        } else {
            "Text"
        },
    );
}

/// Keeps the ordinals a rollout already stores and only stamps the next
/// sequential values onto records that lack one, so ordinal gaps, unknown
/// records and existing history bases keep their meaning. Returns the ordinal
/// that follows the last record.
fn stamp_rollout_ordinals(mut records: Vec<Value>) -> (Vec<Value>, i64) {
    let highest = records
        .iter()
        .filter_map(|record| record.get("ordinal").and_then(Value::as_i64))
        .max();
    let Some(highest) = highest else {
        let line_count = records.len() as i64;
        return (records, line_count);
    };
    let mut next_ordinal = highest + 1;
    for record in records.iter_mut() {
        let Some(record_map) = record.as_object_mut() else {
            continue;
        };
        if !record_map.contains_key("ordinal") {
            record_map.insert("ordinal".to_string(), json!(next_ordinal));
            next_ordinal += 1;
        }
    }
    (records, next_ordinal)
}

fn build_updated_rollout(
    original: &[u8],
    session_id: &str,
    request: &SaveCodexSessionContextRequest,
) -> Result<(Vec<u8>, RolloutSyncPlan), String> {
    let source = std::str::from_utf8(original)
        .map_err(|error| format!("Rollout JSONL is not UTF-8: {error}"))?;
    let line_ending = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let parsed = validate_raw_jsonl(source, session_id)?;

    let mut sync_plan = RolloutSyncPlan::default();
    let updates = request
        .message_updates
        .as_ref()
        .map(|items| {
            items
                .iter()
                .map(|item| (item.line_number, item))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    let removed_call_ids = parsed
        .iter()
        .filter(|(line_number, _)| {
            updates
                .get(line_number)
                .is_some_and(|update| update.deleted == Some(true))
        })
        .filter_map(|(_, record)| {
            let payload = record.get("payload")?;
            let payload_type = payload.get("type")?.as_str()?;
            if record.get("type").and_then(Value::as_str) != Some("response_item")
                || !is_tool_response(payload_type)
            {
                return None;
            }
            payload
                .get("call_id")?
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .collect::<HashSet<_>>();

    let original_records = parsed
        .iter()
        .map(|(_, record)| record.clone())
        .collect::<Vec<_>>();
    sync_plan.original_projection_cursor = Some(RolloutProjectionCursor {
        byte_offset: original.len() as i64,
        next_ordinal: original_records
            .iter()
            .filter_map(|record| record["ordinal"].as_i64())
            .max()
            .map_or(original_records.len() as i64, |ordinal| ordinal + 1),
    });
    let groups = group_tracked_copies(tracked_copies(&original_records));
    let last_turn_id = rollout_last_turn_id(&original_records, session_id);
    let mut response_to_sync_info = HashMap::new();
    for group in &groups {
        let turn = group_turn_id(group, &last_turn_id);
        let role = match group.kind {
            TrackedItemKind::UserMessage => "user",
            TrackedItemKind::AgentMessage => "assistant",
            TrackedItemKind::Reasoning => "reasoning",
        };
        let events = group
            .copies
            .iter()
            .filter(|copy| copy.is_event_copy)
            .map(|copy| parsed[copy.line_index].0)
            .collect::<Vec<_>>();
        for copy in group.copies.iter().filter(|copy| !copy.is_event_copy) {
            response_to_sync_info.insert(
                parsed[copy.line_index].0,
                (
                    turn.clone(),
                    role.to_string(),
                    group.id.clone(),
                    events.clone(),
                ),
            );
        }
    }

    let mut deleted_lines = HashSet::new();
    let mut updated_event_content = HashMap::new();
    let mut touched_agent_turns = HashSet::new();
    let mut original_line_text: HashMap<usize, String> = HashMap::new();
    for (line_idx, record) in &parsed {
        if let Some(text) = response_item_text(record) {
            original_line_text.insert(*line_idx, text);
        }
    }

    let mut update_line_numbers = updates.keys().copied().collect::<Vec<_>>();
    update_line_numbers.sort_unstable();
    for line_number in update_line_numbers {
        let update = updates[&line_number];
        if update.deleted == Some(true) {
            deleted_lines.insert(line_number);
            if let Some((turn_id, role, item_id, event_lines)) =
                response_to_sync_info.get(&line_number)
            {
                deleted_lines.extend(event_lines);
                if let Some(item_id) = item_id {
                    sync_plan
                        .deleted_item_keys
                        .insert((item_id.clone(), turn_id.clone()));
                }
                if role == "assistant" {
                    touched_agent_turns.insert(turn_id.clone());
                }
            }
        } else if update.content.is_some() || update.role.is_some() {
            if let Some((turn_id, original_role, item_id, event_lines)) =
                response_to_sync_info.get(&line_number)
            {
                if let Some(requested_role) = update.role.as_deref() {
                    if requested_role != original_role {
                        return Err(format!(
                            "Line {} cannot change the message role from {original_role} to {requested_role}; remove the message and add a new one instead",
                            line_number + 1
                        ));
                    }
                }
                let new_text = update
                    .content
                    .clone()
                    .or_else(|| original_line_text.get(&line_number).cloned())
                    .ok_or_else(|| format!("Line {} has no editable text", line_number + 1))?;
                for event_line in event_lines {
                    updated_event_content.insert(*event_line, new_text.clone());
                }
                if let Some(item_id) = item_id {
                    sync_plan.updated_items.push(RolloutItemSync {
                        item_id: item_id.clone(),
                        turn_id: turn_id.clone(),
                        item_type: match original_role.as_str() {
                            "user" => "userMessage",
                            "reasoning" => "reasoning",
                            _ => "agentMessage",
                        }
                        .to_string(),
                        text: new_text.clone(),
                    });
                }
                if original_role == "assistant" {
                    touched_agent_turns.insert(turn_id.clone());
                }
            }
        }
    }

    let mut tool_turn_id = String::new();
    for (line_idx, record) in &parsed {
        if record.get("type").and_then(Value::as_str) == Some("response_item") {
            if let Some(call_id) = record
                .get("payload")
                .and_then(|payload| payload.get("call_id"))
                .and_then(Value::as_str)
            {
                if removed_call_ids.contains(call_id) && !tool_turn_id.is_empty() {
                    sync_plan
                        .deleted_item_keys
                        .insert((call_id.to_string(), tool_turn_id.clone()));
                }
            }
        }
        if record.get("type").and_then(Value::as_str) == Some("event_msg") {
            if let Some(payload) = record.get("payload") {
                if payload.get("type").and_then(Value::as_str) == Some("task_started") {
                    tool_turn_id = payload
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
                if payload.get("type").and_then(Value::as_str) == Some("item_completed") {
                    if let Some(item) = payload.get("item") {
                        if let Some(item_id) = item.get("id").and_then(Value::as_str) {
                            if removed_call_ids.contains(item_id) {
                                deleted_lines.insert(*line_idx);
                                let turn_id = payload
                                    .get("turn_id")
                                    .and_then(Value::as_str)
                                    .unwrap_or(&tool_turn_id);
                                if !turn_id.is_empty() {
                                    sync_plan
                                        .deleted_item_keys
                                        .insert((item_id.to_string(), turn_id.to_string()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut applied_updates = HashSet::new();
    let mut records = Vec::with_capacity(parsed.len());
    for (line_number, mut record) in parsed {
        if let Some(update) = updates.get(&line_number) {
            let payload_type = record
                .get("payload")
                .and_then(|payload| payload.get("type"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let editable = record.get("type").and_then(Value::as_str) == Some("response_item")
                && (payload_type == "message"
                    || is_tool_response(payload_type)
                    || payload_type == "reasoning"
                    || payload_type == "thinking");
            if !editable {
                return Err(format!(
                    "Line {} is not an editable session message",
                    line_number + 1
                ));
            }
            applied_updates.insert(line_number);
            if update.deleted == Some(true) {
                continue;
            }
            if update.content.is_some() || update.role.is_some() {
                if payload_type == "message" {
                    let payload = record
                        .get_mut("payload")
                        .and_then(Value::as_object_mut)
                        .ok_or_else(|| {
                            format!("Line {} has no message payload", line_number + 1)
                        })?;
                    if let Some(role) = update.role.as_ref() {
                        if !matches!(role.as_str(), "user" | "assistant" | "system" | "developer") {
                            return Err(format!("Unsupported message role: {role}"));
                        }
                        payload.insert("role".to_string(), json!(role));
                    }
                    let role = payload
                        .get("role")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let content_value = payload.get_mut("content").ok_or_else(|| {
                        format!("Line {} has no message content", line_number + 1)
                    })?;
                    let text = update
                        .content
                        .clone()
                        .unwrap_or_else(|| extract_content_text(content_value));
                    update_content_text(
                        content_value,
                        &text,
                        message_content_type(role.as_deref()),
                    );
                } else if payload_type == "reasoning" || payload_type == "thinking" {
                    let payload = record
                        .get_mut("payload")
                        .and_then(Value::as_object_mut)
                        .ok_or_else(|| {
                            format!("Line {} has no reasoning payload", line_number + 1)
                        })?;
                    let existing_text = extract_reasoning_text(&Value::Object(payload.clone()));
                    let text = update.content.clone().unwrap_or(existing_text);
                    strip_reasoning_signature(payload);
                    write_reasoning_text(payload, &text);
                } else {
                    return Err(format!(
                        "Line {} only supports removal, not message editing",
                        line_number + 1
                    ));
                }
            }
        }

        if deleted_lines.contains(&line_number) {
            continue;
        }

        if record.get("type").and_then(Value::as_str) == Some("response_item") {
            if let Some(payload) = record.get("payload") {
                if is_tool_response(
                    payload
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ) && payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .is_some_and(|call_id| removed_call_ids.contains(call_id))
                {
                    continue;
                }
            }
        } else if record.get("type").and_then(Value::as_str) == Some("event_msg") {
            if let Some(new_text) = updated_event_content.get(&line_number) {
                if matches!(
                    record["payload"]["type"].as_str(),
                    Some("user_message" | "agent_message")
                ) {
                    record["payload"]["message"] = json!(new_text);
                }
                if let Some(item_obj) = record
                    .get_mut("payload")
                    .and_then(|p| p.get_mut("item"))
                    .and_then(Value::as_object_mut)
                {
                    let item_type = item_obj
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let kind = tracked_kind_from_item_type(item_type)
                        .unwrap_or(TrackedItemKind::AgentMessage);
                    rewrite_event_item_text(item_obj, kind, new_text);
                }
            }
        }

        apply_rollout_metadata(&mut record, request);
        records.push(record);
    }

    if applied_updates.len() != updates.len() {
        return Err(
            "One or more edited message lines no longer exist; reload the session".to_string(),
        );
    }

    let mut added_records = Vec::new();
    for (message_index, message) in request
        .new_messages
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        if message.content.trim().is_empty() {
            continue;
        }
        if !matches!(message.role.as_str(), "user" | "assistant") {
            return Err(format!("Unsupported new message role: {}", message.role));
        }
        let now_ts = Utc::now().to_rfc3339();
        let now_ms = Utc::now().timestamp_millis();
        let unique_item_id = format!("msg-added-{:x}-{:x}", now_ms, message_index);
        let content_type = message_content_type(Some(&message.role));

        let response = json!({
            "timestamp": now_ts, "type": "response_item",
            "payload": { "type": "message", "role": message.role,
                "content": [{ "type": content_type, "text": message.content }] }
        });
        let event = json!({
            "timestamp": now_ts, "type": "event_msg",
            "payload": { "type": "item_completed", "thread_id": session_id,
                "turn_id": last_turn_id, "completed_at_ms": now_ms, "started_at_ms": now_ms,
                "item": { "id": unique_item_id,
                    "type": if message.role == "user" { "UserMessage" } else { "AgentMessage" },
                    "content": [{ "type": if message.role == "user" { "text" } else { "Text" }, "text": message.content }] } }
        });
        if message.role == "user" {
            added_records.extend([response, event]);
        } else {
            added_records.extend([event, response]);
            touched_agent_turns.insert(last_turn_id.clone());
        }
        sync_plan.new_items.push(RolloutItemSync {
            item_id: unique_item_id,
            turn_id: last_turn_id.clone(),
            item_type: if message.role == "user" {
                "userMessage"
            } else {
                "agentMessage"
            }
            .to_string(),
            text: message.content.clone(),
        });
    }

    if !added_records.is_empty() {
        let has_turn = original_records.iter().any(|record| {
            record["type"] == "event_msg"
                && matches!(
                    record["payload"]["type"].as_str(),
                    Some("task_started" | "item_completed" | "task_complete" | "turn_aborted")
                )
                && record["payload"]["turn_id"].as_str() == Some(last_turn_id.as_str())
        });
        if !has_turn {
            let timestamp = Utc::now();
            added_records.insert(
                0,
                json!({
                    "timestamp": timestamp.to_rfc3339(), "type": "event_msg",
                    "payload": { "type": "task_started", "turn_id": last_turn_id,
                        "started_at": timestamp.timestamp() }
                }),
            );
            added_records.push(json!({
                "timestamp": timestamp.to_rfc3339(), "type": "event_msg",
                "payload": { "type": "task_complete", "turn_id": last_turn_id,
                    "started_at": timestamp.timestamp(), "completed_at": timestamp.timestamp(),
                    "duration_ms": 0, "last_agent_message": null }
            }));
        }
        // Add within the last turn, before its existing terminal record. Keep
        // completed/aborted status and timing; an open turn stays open.
        let mut insertion = records.len();
        let mut current_turn = session_id.to_string();
        for (index, record) in records.iter().enumerate() {
            if record["type"] != "event_msg" {
                continue;
            }
            let payload = &record["payload"];
            if payload["type"] == "task_started" {
                if let Some(turn) = non_empty_string(payload.get("turn_id")) {
                    current_turn = turn;
                }
            }
            let turn = payload["turn_id"]
                .as_str()
                .filter(|turn| !turn.is_empty())
                .unwrap_or(&current_turn);
            if turn == last_turn_id
                && matches!(
                    payload["type"].as_str(),
                    Some("task_complete" | "turn_aborted")
                )
            {
                insertion = index;
            }
        }
        // Explicit ordinals must remain increasing across an insertion. Shift
        // only the suffix; preserve earlier positions and existing gaps.
        let (mut stamped, _) = stamp_rollout_ordinals(records);
        if let Some(first) = stamped
            .get(insertion)
            .and_then(|record| record["ordinal"].as_i64())
        {
            for (index, record) in added_records.iter_mut().enumerate() {
                record["ordinal"] = json!(first + index as i64);
            }
            for record in &mut stamped[insertion..] {
                if let Some(ordinal) = record["ordinal"].as_i64() {
                    record["ordinal"] = json!(ordinal + added_records.len() as i64);
                }
            }
        }
        stamped.splice(insertion..insertion, added_records);
        records = stamped;
    }
    refresh_task_complete_messages(&mut records, &touched_agent_turns, session_id);

    let (records, next_ordinal) = stamp_rollout_ordinals(records);
    let serialized = serialize_jsonl(&records, line_ending)?;
    collect_projection_positions(&records, &serialized, &mut sync_plan);
    sync_plan.appended_ordinal = next_ordinal;
    sync_plan.projection_cursor = Some(RolloutProjectionCursor {
        byte_offset: serialized.len() as i64,
        next_ordinal,
    });
    Ok((serialized, sync_plan))
}

fn update_session_database(
    path: &Path,
    session_id: &str,
    request: &SaveCodexSessionContextRequest,
    rollout_path: Option<&Path>,
) -> Result<(), String> {
    let mut connection = open_read_write(path)?;
    let transaction = connection.transaction().map_err(|error| {
        format!(
            "Failed to begin session context transaction {}: {error}",
            path.display()
        )
    })?;
    let columns = table_columns(&transaction, "threads")?;
    let mut updates = Vec::new();
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    let title_column = if columns.contains("title") {
        "title"
    } else {
        "name"
    };
    for (column, value) in [
        (title_column, request.title.as_ref()),
        ("cwd", request.cwd.as_ref()),
        ("model", request.model.as_ref()),
        ("model_provider", request.model_provider.as_ref()),
    ] {
        if columns.contains(column) {
            if let Some(value) = value {
                updates.push(format!("{column} = ?"));
                params.push(Box::new(value.clone()));
            }
        }
    }
    if let Some(archived) = request.archived {
        if columns.contains("archived") {
            updates.push("archived = ?".to_string());
            params.push(Box::new(i64::from(archived)));
        }
        if columns.contains("archived_at") {
            updates.push("archived_at = ?".to_string());
            params.push(Box::new(if archived {
                Some(Utc::now().timestamp())
            } else {
                None::<i64>
            }));
        }
    }
    if let Some(rollout_path) = rollout_path {
        if columns.contains("rollout_path") {
            updates.push("rollout_path = ?".to_string());
            params.push(Box::new(rollout_path.to_string_lossy().to_string()));
        }
    }
    let now_ts = Utc::now().timestamp();
    let now_ms = Utc::now().timestamp_millis();
    if columns.contains("updated_at") {
        updates.push("updated_at = ?".to_string());
        params.push(Box::new(now_ts));
    }
    if columns.contains("updated_at_ms") {
        updates.push("updated_at_ms = ?".to_string());
        params.push(Box::new(now_ms));
    }
    if updates.is_empty() {
        return Err(format!(
            "Database {} has no supported thread columns",
            path.display()
        ));
    }
    params.push(Box::new(session_id.to_string()));
    let sql = format!("UPDATE threads SET {} WHERE id = ?", updates.join(", "));
    let refs = params
        .iter()
        .map(|value| value.as_ref())
        .collect::<Vec<_>>();
    let changed = transaction
        .execute(&sql, refs.as_slice())
        .map_err(|error| {
            format!(
                "Failed to update session database {}: {error}",
                path.display()
            )
        })?;
    if changed != 1 {
        return Err(format!(
            "Session {session_id} disappeared from database {} during the update",
            path.display()
        ));
    }
    if table_exists(&transaction, "local_thread_catalog")? {
        let columns = table_columns(&transaction, "local_thread_catalog")?;
        let mut updates = Vec::new();
        let mut params: Vec<Box<dyn ToSql>> = Vec::new();
        for (column, value) in [
            ("display_title", request.title.as_ref()),
            ("cwd", request.cwd.as_ref()),
            ("model_provider", request.model_provider.as_ref()),
        ] {
            if columns.contains(column) {
                if let Some(value) = value {
                    updates.push(format!("{column} = ?"));
                    params.push(Box::new(value.clone()));
                }
            }
        }
        if columns.contains("source_updated_at") {
            updates.push("source_updated_at = ?".to_string());
            params.push(Box::new(now_ts as f64));
        }
        if !updates.is_empty() {
            params.push(Box::new(session_id.to_string()));
            let sql = format!(
                "UPDATE local_thread_catalog SET {} WHERE thread_id = ?",
                updates.join(", ")
            );
            let refs = params
                .iter()
                .map(|value| value.as_ref())
                .collect::<Vec<_>>();
            transaction
                .execute(&sql, refs.as_slice())
                .map_err(|error| {
                    format!(
                        "Failed to update local session catalog {}: {error}",
                        path.display()
                    )
                })?;
        }
    }
    transaction.commit().map_err(|error| {
        format!(
            "Failed to commit session context transaction {}: {error}",
            path.display()
        )
    })
}

struct SessionIndexTitleUpdate {
    path: PathBuf,
    original: Vec<u8>,
    previous: Value,
    next: Value,
}

fn latest_session_index_entry(bytes: &[u8], session_id: &str) -> Result<Option<Value>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("session_index.jsonl is not UTF-8: {error}"))?;
    let target = normalize_thread_id(session_id);
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| {
            record
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| normalize_thread_id(id) == target)
        })
        .last())
}

fn build_session_index_title_update(
    codex_home: &Path,
    session_id: &str,
    new_title: &str,
) -> Result<Option<SessionIndexTitleUpdate>, String> {
    let path = codex_home.join("session_index.jsonl");
    if !path.is_file() {
        return Ok(None);
    }
    let canonical_home = fs::canonicalize(codex_home)
        .map_err(|error| format!("Failed to resolve Codex home: {error}"))?;
    let path = fs::canonicalize(&path)
        .map_err(|error| format!("Failed to resolve session_index.jsonl: {error}"))?;
    if !path.starts_with(&canonical_home) {
        return Err("session_index.jsonl resolves outside Codex home".to_string());
    }
    let original =
        fs::read(&path).map_err(|error| format!("Failed to read session_index.jsonl: {error}"))?;
    let Some(previous) = latest_session_index_entry(&original, session_id)? else {
        return Ok(None);
    };
    if previous["thread_name"].as_str() == Some(new_title) {
        return Ok(None);
    }
    let mut next = previous.clone();
    next["thread_name"] = json!(new_title);
    next["updated_at"] = json!(Utc::now().to_rfc3339());
    Ok(Some(SessionIndexTitleUpdate {
        path,
        original,
        previous,
        next,
    }))
}

fn append_session_index_entry(path: &Path, current: &[u8], entry: &Value) -> Result<(), String> {
    // Preserve the file identity: Codex may already hold an append handle.
    if !current.is_empty() && !current.ends_with(b"\n") {
        return Err(
            "session_index.jsonl has an unterminated record; reload after its writer finishes"
                .to_string(),
        );
    }
    let newline = if current.ends_with(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let bytes = format!("{entry}{newline}");
    OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|mut file| {
            file.write_all(bytes.as_bytes())?;
            file.sync_all()
        })
        .map_err(|error| format!("Failed to append session_index.jsonl: {error}"))
}

fn apply_session_index_title_update(update: &SessionIndexTitleUpdate) -> Result<(), String> {
    let current = fs::read(&update.path)
        .map_err(|error| format!("Failed to reread session_index.jsonl before saving: {error}"))?;
    let id = update.previous["id"].as_str().unwrap_or_default();
    if latest_session_index_entry(&current, id)?.as_ref() != Some(&update.previous) {
        return Err(
            "The session index entry changed during saving; reload before editing".to_string(),
        );
    }
    append_session_index_entry(&update.path, &current, &update.next)
}

fn restore_session_index(update: &SessionIndexTitleUpdate) -> Result<(), String> {
    let current = fs::read(&update.path).map_err(|error| {
        format!("Failed to reread session_index.jsonl before rollback: {error}")
    })?;
    let id = update.previous["id"].as_str().unwrap_or_default();
    let latest = latest_session_index_entry(&current, id)?;
    if latest.as_ref() == Some(&update.previous) {
        return Ok(());
    }
    if latest.as_ref() != Some(&update.next) {
        return Err(
            "session_index.jsonl changed externally; refusing to overwrite newer index records"
                .to_string(),
        );
    }
    let mut restored = update.previous.clone();
    restored["updated_at"] = json!(Utc::now().to_rfc3339());
    append_session_index_entry(&update.path, &current, &restored)
}

fn sync_thread_history_projection_databases(
    database_paths: &[PathBuf],
    session_id: &str,
    plan: &RolloutSyncPlan,
) -> Result<usize, String> {
    let mut total_rows_affected = 0;
    for db_path in database_paths {
        let mut connection = open_read_write(db_path)?;
        let transaction = connection.transaction().map_err(|error| {
            format!(
                "Failed to start transaction for {}: {error}",
                db_path.display()
            )
        })?;

        validate_projection_cursor(&transaction, session_id, plan.original_projection_cursor)?;

        let has_items = table_exists(&transaction, "thread_items")?;
        if !has_items {
            continue;
        }

        let turn_columns = table_columns(&transaction, "thread_turns")?;

        for item in &plan.updated_items {
            let existing_json: Option<String> = transaction
                .query_row(
                    "SELECT item_json FROM thread_items WHERE thread_id = ?1 AND turn_id = ?2 AND item_id = ?3",
                    rusqlite::params![session_id, item.turn_id, item.item_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| format!("Failed to read thread item {}: {error}", item.item_id))?;

            if let Some(raw_json) = existing_json {
                if let Ok(mut val) = serde_json::from_str::<Value>(&raw_json) {
                    let mut changed = false;
                    if item.item_type == "agentMessage" {
                        if let Some(obj) = val.as_object_mut() {
                            obj.insert("text".to_string(), json!(item.text));
                            changed = true;
                        }
                    } else if item.item_type == "userMessage" {
                        if let Some(obj) = val.as_object_mut() {
                            let content = obj.entry("content").or_insert_with(|| json!([]));
                            update_content_text(content, &item.text, "text");
                            changed = true;
                        }
                    } else if item.item_type == "reasoning" {
                        if let Some(obj) = val.as_object_mut() {
                            obj.remove("signature");
                            obj.remove("encrypted_content");
                            obj.insert("signature_stripped".to_string(), json!(true));
                            obj.insert("summary".to_string(), json!([item.text]));
                            obj.insert("content".to_string(), json!([]));
                            changed = true;
                        }
                    }
                    if changed {
                        if let Ok(new_json_str) = serde_json::to_string(&val) {
                            let updated = transaction
                                .execute(
                                    "UPDATE thread_items SET item_json = ?1 WHERE thread_id = ?2 AND turn_id = ?3 AND item_id = ?4",
                                    rusqlite::params![new_json_str, session_id, item.turn_id, item.item_id],
                                )
                                .map_err(|error| format!("Failed to update thread item {}: {error}", item.item_id))?;
                            total_rows_affected += updated;
                        }
                    }
                }
            }
        }

        for (deleted_id, turn_id) in &plan.deleted_item_keys {
            let deleted = transaction.execute(
                "DELETE FROM thread_items WHERE thread_id = ?1 AND turn_id = ?2 AND item_id = ?3",
                rusqlite::params![session_id, turn_id, deleted_id],
            ).map_err(|error| format!("Failed to delete thread item {deleted_id}: {error}"))?;
            total_rows_affected += deleted;
        }

        let mut new_turns = HashSet::new();
        for new_item in &plan.new_items {
            if !new_turns.insert(new_item.turn_id.as_str()) || turn_columns.is_empty() {
                continue;
            }
            let position = plan.turn_positions.get(&new_item.turn_id).ok_or_else(|| {
                format!("Missing rollout position for new turn {}", new_item.turn_id)
            })?;
            let mut columns = vec!["thread_id", "turn_id"];
            let mut values: Vec<Box<dyn ToSql>> = vec![
                Box::new(session_id.to_string()),
                Box::new(new_item.turn_id.clone()),
            ];
            for (column, value) in [
                ("rollout_ordinal", json!(position.ordinal)),
                ("rollout_byte_offset", json!(position.byte_offset)),
                ("rollout_end_ordinal", json!(position.end_ordinal)),
                ("rollout_end_byte_offset", json!(position.end_byte_offset)),
                ("status", json!(position.status)),
                ("started_at", json!(position.started_at)),
                ("completed_at", json!(position.completed_at)),
                ("duration_ms", json!(position.duration_ms)),
                ("error_json", json!(position.error_json)),
                ("root_turn_id", json!(position.root_turn_id)),
            ] {
                if turn_columns.contains(column) {
                    columns.push(column);
                    values.push(sqlite_param_for_json(Some(&value)));
                }
            }
            let placeholders = vec!["?"; columns.len()].join(", ");
            let refs = values
                .iter()
                .map(|value| value.as_ref())
                .collect::<Vec<_>>();
            total_rows_affected += transaction.execute(
                &format!("INSERT INTO thread_turns ({}) SELECT {placeholders} WHERE NOT EXISTS (SELECT 1 FROM thread_turns WHERE thread_id = ? AND turn_id = ?)", columns.join(", ")),
                rusqlite::params_from_iter(refs.into_iter().chain([
                    &session_id as &dyn ToSql, &new_item.turn_id as &dyn ToSql,
                ])),
            ).map_err(|error| format!("Failed to create thread turn {}: {error}", new_item.turn_id))?;
        }

        for (index, new_item) in plan.new_items.iter().enumerate() {
            let item_json = if new_item.item_type == "agentMessage" {
                json!({
                    "type": "agentMessage",
                    "id": new_item.item_id,
                    "text": new_item.text,
                    "phase": null,
                    "memoryCitation": null
                })
            } else {
                json!({
                    "type": "userMessage",
                    "id": new_item.item_id,
                    "clientId": null,
                    "content": [{"type": "text", "text": new_item.text}]
                })
            };
            let now_ms = Utc::now().timestamp_millis();
            let inserted = transaction
                .execute(
                    "INSERT OR REPLACE INTO thread_items (thread_id, turn_id, item_id, rollout_ordinal, created_at_ms, item_json, item_type, updated_at_ordinal, started_at_ms, completed_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        session_id,
                        new_item.turn_id,
                        new_item.item_id,
                        plan.item_ordinals
                            .get(&(new_item.item_id.clone(), new_item.turn_id.clone()))
                            .copied()
                            .unwrap_or(plan.appended_ordinal + index as i64),
                        now_ms,
                        item_json.to_string(),
                        new_item.item_type,
                        0i64,
                        now_ms,
                        now_ms
                    ],
                )
                .map_err(|error| format!("Failed to insert thread item {}: {error}", new_item.item_id))?;
            total_rows_affected += inserted;
        }

        for ((item_id, turn_id), ordinal) in &plan.item_ordinals {
            total_rows_affected += transaction
                .execute(
                    "UPDATE thread_items SET rollout_ordinal = ?1 WHERE thread_id = ?2 AND turn_id = ?3 AND item_id = ?4",
                    rusqlite::params![ordinal, session_id, turn_id, item_id],
                )
                .map_err(|error| {
                    format!("Failed to update rollout ordinal for thread item {item_id}: {error}")
                })?;
        }

        let affected_turns: HashSet<&str> = plan
            .deleted_item_keys
            .iter()
            .map(|(_, turn)| turn.as_str())
            .chain(plan.new_items.iter().map(|item| item.turn_id.as_str()))
            .collect();
        for turn_id in affected_turns {
            let user_type = "(item_type = 'userMessage' OR (item_type = '' AND json_extract(item_json, '$.type') = 'userMessage'))";
            let agent_type = "(item_type = 'agentMessage' OR (item_type = '' AND json_extract(item_json, '$.type') = 'agentMessage'))";
            if turn_columns.contains("first_user_item_id") {
                total_rows_affected += transaction.execute(
                    &format!("UPDATE thread_turns SET first_user_item_id = (SELECT item_id FROM thread_items WHERE thread_id = ?1 AND turn_id = ?2 AND {user_type} ORDER BY rollout_ordinal LIMIT 1) WHERE thread_id = ?1 AND turn_id = ?2"),
                    rusqlite::params![session_id, turn_id],
                ).map_err(|error| format!("Failed to refresh the first user item for {turn_id}: {error}"))?;
            }
            if turn_columns.contains("final_agent_item_id") {
                let terminal = if turn_columns.contains("status") {
                    "status IN ('completed', 'interrupted', 'failed')"
                } else {
                    "1"
                };
                total_rows_affected += transaction.execute(
                    &format!("UPDATE thread_turns SET final_agent_item_id = COALESCE((SELECT item_id FROM thread_items WHERE thread_id = ?1 AND turn_id = ?2 AND {agent_type} AND json_extract(item_json, '$.phase') = 'final_answer' ORDER BY rollout_ordinal DESC LIMIT 1), CASE WHEN {terminal} THEN (SELECT item_id FROM thread_items WHERE thread_id = ?1 AND turn_id = ?2 AND {agent_type} AND json_extract(item_json, '$.phase') IS NULL ORDER BY rollout_ordinal DESC LIMIT 1) END) WHERE thread_id = ?1 AND turn_id = ?2"),
                    rusqlite::params![session_id, turn_id],
                ).map_err(|error| format!("Failed to refresh the final agent item for {turn_id}: {error}"))?;
            }
        }

        for (turn_id, position) in &plan.turn_positions {
            for (column, value) in [
                ("rollout_ordinal", Some(position.ordinal)),
                ("rollout_byte_offset", Some(position.byte_offset)),
                ("rollout_end_ordinal", position.end_ordinal),
                ("rollout_end_byte_offset", position.end_byte_offset),
            ] {
                if turn_columns.contains(column) {
                    total_rows_affected += transaction.execute(
                        &format!("UPDATE thread_turns SET {column} = ?1 WHERE thread_id = ?2 AND turn_id = ?3"),
                        rusqlite::params![value, session_id, turn_id],
                    ).map_err(|error| format!("Failed to update {column} for turn {turn_id}: {error}"))?;
                }
            }
        }

        if table_exists(&transaction, "thread_history_projection_state")? {
            if let Some(cursor) = plan.projection_cursor {
                let has_state: bool = transaction
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM thread_history_projection_state WHERE thread_id = ?1)",
                        [session_id],
                        |row| row.get(0),
                    )
                    .map_err(|error| {
                        format!(
                            "Failed to read the thread history projection cursor for {session_id}: {error}"
                        )
                    })?;
                if has_state {
                    transaction.execute(
                        "UPDATE thread_history_projection_state SET next_rollout_byte_offset = ?1, next_rollout_ordinal = ?2 WHERE thread_id = ?3",
                        rusqlite::params![cursor.byte_offset, cursor.next_ordinal, session_id],
                    ).map_err(|error| {
                        format!("Failed to update the thread history projection cursor for {session_id}: {error}")
                    })?;
                }
            }
        }

        transaction.commit().map_err(|error| {
            format!(
                "Failed to commit thread history transaction {}: {error}",
                db_path.display()
            )
        })?;
    }
    Ok(total_rows_affected)
}

/// Advancing a cursor is safe only when the original durable prefix was fully
/// materialized. Refuse incomplete/stale projections rather than skipping rows.
fn validate_projection_cursor(
    connection: &Connection,
    session_id: &str,
    expected: Option<RolloutProjectionCursor>,
) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    if !table_exists(connection, "thread_history_projection_state")? {
        return Ok(());
    }
    let current: Option<(i64, i64)> = connection.query_row(
        "SELECT next_rollout_byte_offset, next_rollout_ordinal FROM thread_history_projection_state WHERE thread_id = ?1",
        [session_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(|error| format!("Failed to inspect the projection cursor: {error}"))?;
    if current.is_some_and(|cursor| cursor != (expected.byte_offset, expected.next_ordinal)) {
        return Err("The thread history projection is incomplete or stale. Let Codex finish indexing this session, close it, and reload before editing".to_string());
    }
    Ok(())
}

fn format_save_rollback_error(
    base_error: String,
    rollout_rollback: Option<Result<(), String>>,
    index_rollback: Option<Result<(), String>>,
    database_rollback: Option<Result<(), String>>,
    history_rollback: Option<Result<(), String>>,
    backup_dir: &Path,
) -> String {
    let mut details = Vec::new();
    for (name, result) in [
        ("rollout", rollout_rollback),
        ("session index", index_rollback),
        ("thread history", history_rollback),
        ("session database", database_rollback),
    ] {
        if let Some(result) = result {
            details.push(match result {
                Ok(()) => format!("{name} restored"),
                Err(error) => format!("{name} rollback failed: {error}"),
            });
        }
    }
    if details.is_empty() {
        format!("{base_error}. Backup: {}", backup_dir.display())
    } else {
        format!(
            "{base_error}; {}. Backup: {}",
            details.join("; "),
            backup_dir.display()
        )
    }
}

fn rollback_rollout(
    original_path: &Path,
    current_path: &Path,
    original_bytes: &[u8],
) -> Result<(), String> {
    if current_path != original_path && current_path.exists() {
        if let Some(parent) = original_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("Failed to recreate rollout directory during rollback: {error}")
            })?;
        }
        fs::rename(current_path, original_path).map_err(|error| {
            format!(
                "Failed to restore rollout path {}: {error}",
                original_path.display()
            )
        })?;
    }
    crate::write_bytes_atomically(original_path, original_bytes)
}

fn get_codex_session_context_from_home(
    codex_home: &Path,
    raw_session_id: &str,
) -> Result<CodexSessionContextDetail, String> {
    let session_id = validate_session_id(&normalize_thread_id(raw_session_id))?;
    let (database_paths, _) = discover_database_paths(codex_home, false);

    let mut title = String::new();
    let mut cwd = String::new();
    let mut model_provider = String::new();
    let mut model = None;
    let mut archived = false;
    let mut updated_at_ms = None;
    let mut matched_database_path = None;
    let mut rollout_path_buf = None;

    for db_path in &database_paths {
        if let Ok(conn) = open_read_only(db_path) {
            if let Ok(true) = table_exists(&conn, "threads") {
                let columns = table_columns(&conn, "threads").unwrap_or_default();
                let title_col =
                    optional_column(&columns, "title", optional_column(&columns, "name", "id"));
                let cwd_col = optional_column(&columns, "cwd", "''");
                let provider_col = optional_column(&columns, "model_provider", "''");
                let model_col = optional_column(&columns, "model", "''");
                let archived_col = optional_column(&columns, "archived", "0");
                let rollout_col = optional_column(&columns, "rollout_path", "''");
                let time_col = optional_column(
                    &columns,
                    "updated_at_ms",
                    optional_column(&columns, "updated_at", "NULL"),
                );

                let query = format!(
                    "SELECT {title_col}, {cwd_col}, {provider_col}, {model_col}, {archived_col}, {rollout_col}, {time_col} FROM threads WHERE id = ?1 LIMIT 1"
                );
                if let Ok(mut stmt) = conn.prepare(&query) {
                    if let Ok(mut rows) = stmt.query([&session_id]) {
                        if let Ok(Some(row)) = rows.next() {
                            matched_database_path = Some(db_path.to_string_lossy().to_string());
                            for (index, field) in
                                [(0, &mut title), (1, &mut cwd), (2, &mut model_provider)]
                            {
                                if let Ok(Some(value)) = row.get::<_, Option<String>>(index) {
                                    if !value.trim().is_empty() {
                                        *field = value;
                                    }
                                }
                            }
                            if let Ok(Some(val)) = row.get::<_, Option<String>>(3) {
                                if !val.trim().is_empty() {
                                    model = Some(val);
                                }
                            }
                            if let Ok(Some(val)) = row.get::<_, Option<i64>>(4) {
                                archived = val != 0;
                            }
                            if let Ok(Some(val)) = row.get::<_, Option<String>>(5) {
                                if !val.trim().is_empty() {
                                    let path = PathBuf::from(val);
                                    rollout_path_buf = validated_rollout_path(codex_home, &path)?;
                                }
                            }
                            if let Ok(Some(val)) = row.get::<_, Option<i64>>(6) {
                                updated_at_ms = Some(if val < 100_000_000_000 {
                                    val.saturating_mul(1000)
                                } else {
                                    val
                                });
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    if rollout_path_buf.is_none() {
        rollout_path_buf = find_rollout_for_session(codex_home, &session_id)?;
    }
    if title.is_empty() {
        if let Some(metadata) = read_session_index_metadata(codex_home)?.get(&session_id) {
            title = metadata.title.clone();
        }
    }
    if let Some(path) = rollout_path_buf.as_deref() {
        archived = archived || is_archived_rollout_path(codex_home, path);
    }

    let mut messages = Vec::new();
    let mut rollout_model = None;
    let mut total_lines = 0usize;
    let mut user_count = 0usize;
    let mut assistant_count = 0usize;
    let mut tool_count = 0usize;
    let mut reasoning_count = 0usize;
    let mut file_size_bytes = 0u64;
    let mut raw_jsonl = None;
    let mut rollout_sha256 = None;

    if let Some(ref path) = rollout_path_buf {
        let file = fs::File::open(path)
            .map_err(|error| format!("Failed to read rollout {}: {error}", path.display()))?;
        let mut hasher = Sha256::new();
        let mut raw_bytes = Some(Vec::new());
        {
            let mut reader = BufReader::new(file);
            let mut line = Vec::new();
            loop {
                line.clear();
                let read = reader.read_until(b'\n', &mut line).map_err(|error| {
                    format!("Failed to read rollout {}: {error}", path.display())
                })?;
                if read == 0 {
                    break;
                }
                hasher.update(&line);
                file_size_bytes += read as u64;
                if file_size_bytes > MAX_RAW_JSONL_BYTES {
                    raw_bytes = None;
                } else if let Some(bytes) = raw_bytes.as_mut() {
                    bytes.extend_from_slice(&line);
                }
                let idx = total_lines;
                total_lines += 1;
                let Ok(record) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                let rec_type = record
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let payload = record.get("payload");

                if rec_type == "session_meta" {
                    if let Some(payload) = payload {
                        if title.is_empty() {
                            title = payload
                                .get("title")
                                .or_else(|| payload.get("name"))
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                        }
                        if cwd.is_empty() {
                            cwd = payload
                                .get("cwd")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                        }
                        if model_provider.is_empty() {
                            model_provider = payload
                                .get("model_provider")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                        }
                    }
                } else if rec_type == "turn_context" {
                    if let Some(payload) = payload {
                        if let Some(turn_model) = payload.get("model").and_then(Value::as_str) {
                            rollout_model = Some(turn_model.to_string());
                        }
                        if cwd.is_empty() {
                            cwd = payload
                                .get("cwd")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                        }
                    }
                } else if rec_type == "response_item" {
                    let Some(payload) = payload else { continue };
                    let p_type = payload["type"].as_str().unwrap_or_default();
                    let (prefix, role, content) = match p_type {
                        "message" => {
                            let role = payload["role"].as_str().unwrap_or("unknown");
                            match role {
                                "user" => user_count += 1,
                                "assistant" => assistant_count += 1,
                                _ => {}
                            }
                            ("msg", role, extract_content_text(&payload["content"]))
                        }
                        "reasoning" | "thinking" => {
                            reasoning_count += 1;
                            ("reasoning", "reasoning", extract_reasoning_text(payload))
                        }
                        tool if is_tool_response(tool) => {
                            tool_count += 1;
                            let content = if tool.ends_with("_output") {
                                payload
                                    .get("output")
                                    .map(|output| {
                                        output
                                            .as_str()
                                            .map(str::to_string)
                                            .unwrap_or_else(|| output.to_string())
                                    })
                                    .unwrap_or_default()
                            } else {
                                let name = payload["name"].as_str().unwrap_or("unknown_tool");
                                let args = payload
                                    .get("arguments")
                                    .or_else(|| payload.get("input"))
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                format!("{name}({args})")
                            };
                            ("tool", "tool", content)
                        }
                        _ => continue,
                    };
                    messages.push(CodexSessionMessageItem {
                        id: format!("{prefix}-{idx}"),
                        line_number: idx,
                        role: role.to_string(),
                        timestamp: record["timestamp"].as_str().map(str::to_string),
                        content,
                        raw_type: rec_type.to_string(),
                        model: rollout_model.clone().or_else(|| model.clone()),
                        call_id: if role == "tool" {
                            payload["call_id"].as_str().map(str::to_string)
                        } else {
                            None
                        },
                        signature_stripped: if role == "reasoning" {
                            payload["signature_stripped"].as_bool()
                        } else {
                            None
                        },
                    });
                }
            }
        }
        raw_jsonl = raw_bytes.and_then(|bytes| String::from_utf8(bytes).ok());
        rollout_sha256 = Some(format!("{:x}", hasher.finalize()));
    } else if matched_database_path.is_none() {
        return Err(format!(
            "Session {session_id} not found in database or rollout files"
        ));
    }

    if model.is_none() {
        model = rollout_model;
    }
    Ok(CodexSessionContextDetail {
        id: session_id,
        title,
        cwd,
        model_provider,
        model,
        archived,
        updated_at_ms,
        rollout_path: rollout_path_buf.map(|path| path.to_string_lossy().to_string()),
        database_path: matched_database_path,
        messages,
        stats: CodexSessionContextStats {
            total_lines,
            message_count: user_count + assistant_count + reasoning_count,
            user_message_count: user_count,
            assistant_message_count: assistant_count,
            tool_count,
            reasoning_count,
            file_size_bytes,
        },
        raw_jsonl_available: raw_jsonl.is_some(),
        raw_jsonl,
        rollout_sha256,
    })
}

fn save_codex_session_context_from_home(
    codex_home: &Path,
    request: SaveCodexSessionContextRequest,
) -> Result<SaveCodexSessionContextResult, String> {
    let session_id = validate_session_id(&normalize_thread_id(&request.session_id))?;
    let _guard = CODEX_SESSION_WRITE_LOCK
        .lock()
        .map_err(|_| "Codex session write lock is poisoned".to_string())?;
    let rollout_path = find_rollout_for_session(codex_home, &session_id)?;
    let _writer_lock = rollout_path
        .as_ref()
        .map(|_| acquire_session_writer_lock(codex_home, &session_id))
        .transpose()?;
    let database_path = find_session_database_path(codex_home, &session_id)?;
    let thread_history_paths = discover_thread_history_databases(codex_home);
    if rollout_path.is_none() && database_path.is_none() && thread_history_paths.is_empty() {
        return Err(format!("Session {session_id} was not found"));
    }
    let has_structured_changes = request
        .message_updates
        .as_ref()
        .is_some_and(|updates| !updates.is_empty())
        || request
            .new_messages
            .as_ref()
            .is_some_and(|messages| !messages.is_empty());
    if rollout_path.is_none() && has_structured_changes {
        return Err("This session has no rollout file to edit".to_string());
    }
    if rollout_path.is_none() && request.archived.is_some() {
        return Err("This session has no rollout file to archive or restore".to_string());
    }

    let mut original_rollout: Option<Vec<u8>> = None;
    let mut next_rollout: Option<Vec<u8>> = None;
    let mut target_rollout: Option<PathBuf> = None;
    let mut sync_plan = RolloutSyncPlan::default();
    if let Some(path) = rollout_path.as_deref() {
        let original = fs::read(path)
            .map_err(|error| format!("Failed to read rollout {}: {error}", path.display()))?;
        let current_sha256 = sha256_hex(&original);
        let expected_sha256 = request
            .expected_rollout_sha256
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                "The rollout snapshot is missing. Reload the session before saving".to_string()
            })?;
        if current_sha256 != expected_sha256 {
            return Err(
                "The rollout changed after it was loaded. Reload the session before saving"
                    .to_string(),
            );
        }
        let (next, plan) = build_updated_rollout(&original, &session_id, &request)?;
        let target = rollout_archive_target(codex_home, path, request.archived)?;
        ensure_context_path_in_home(codex_home, &target)?;
        if target != path && target.exists() {
            return Err(format!(
                "The archive destination already exists: {}",
                target.display()
            ));
        }
        original_rollout = Some(original);
        next_rollout = Some(next);
        target_rollout = Some(target);
        sync_plan = plan;
    }

    let session_index_update = request
        .title
        .as_deref()
        .map(|title| build_session_index_title_update(codex_home, &session_id, title))
        .transpose()?
        .flatten();

    for path in &thread_history_paths {
        validate_projection_cursor(
            &open_read_only(path)?,
            &session_id,
            sync_plan.original_projection_cursor,
        )
        .map_err(|error| {
            format!("Failed to inspect thread history projection before saving: {error}")
        })?;
    }

    let backup_dir = create_context_edit_backup(
        codex_home,
        &session_id,
        rollout_path.as_deref(),
        original_rollout.as_deref(),
        database_path.as_deref(),
        session_index_update
            .as_ref()
            .map(|update| update.original.as_slice()),
        &thread_history_paths,
    )?;

    let mut written_rollout = rollout_path.clone();
    if let (Some(source), Some(original), Some(next), Some(target)) = (
        rollout_path.as_deref(),
        original_rollout.as_deref(),
        next_rollout.as_deref(),
        target_rollout.as_deref(),
    ) {
        let current = fs::read(source).map_err(|error| {
            format!(
                "Failed to reread rollout {} before saving: {error}",
                source.display()
            )
        })?;
        if current != original {
            return Err(format!(
                "The rollout changed during the save operation; nothing was overwritten. Backup: {}",
                backup_dir.display()
            ));
        }
        if next != original {
            crate::write_bytes_atomically(source, next)?;
        }
        if target != source {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    let _ = rollback_rollout(source, source, original);
                    format!(
                        "Failed to create archive directory {}: {error}",
                        parent.display()
                    )
                })?;
            }
            if let Err(error) = fs::rename(source, target) {
                let rollback = rollback_rollout(source, source, original);
                return Err(match rollback {
                    Ok(()) => format!("Failed to move rollout to {}: {error}", target.display()),
                    Err(rollback_error) => format!(
                        "Failed to move rollout to {}: {error}; rollback also failed: {rollback_error}. Backup: {}",
                        target.display(),
                        backup_dir.display()
                    ),
                });
            }
            written_rollout = Some(target.to_path_buf());
        }
    }

    if let Some(update) = session_index_update.as_ref() {
        if let Err(error) = apply_session_index_title_update(update) {
            let rollout_rollback =
                if let (Some(original_path), Some(current_path), Some(original)) = (
                    rollout_path.as_deref(),
                    written_rollout.as_deref(),
                    original_rollout.as_deref(),
                ) {
                    Some(rollback_rollout(original_path, current_path, original))
                } else {
                    None
                };
            let index_rollback = Some(restore_session_index(update));
            let history_rollback = Some(restore_thread_history_backup(&backup_dir, &session_id));
            return Err(format_save_rollback_error(
                format!("Failed to update session index: {error}"),
                rollout_rollback,
                index_rollback,
                None,
                history_rollback,
                &backup_dir,
            ));
        }
    }

    if let Some(path) = database_path.as_deref() {
        if let Err(error) =
            update_session_database(path, &session_id, &request, written_rollout.as_deref())
        {
            let rollout_rollback =
                if let (Some(original_path), Some(current_path), Some(original)) = (
                    rollout_path.as_deref(),
                    written_rollout.as_deref(),
                    original_rollout.as_deref(),
                ) {
                    Some(rollback_rollout(original_path, current_path, original))
                } else {
                    None
                };
            let index_rollback = session_index_update.as_ref().map(restore_session_index);
            let history_rollback = Some(restore_thread_history_backup(&backup_dir, &session_id));
            return Err(format_save_rollback_error(
                format!("Database update failed: {error}"),
                rollout_rollback,
                index_rollback,
                None,
                history_rollback,
                &backup_dir,
            ));
        }
    }

    if !thread_history_paths.is_empty() {
        if let Err(error) =
            sync_thread_history_projection_databases(&thread_history_paths, &session_id, &sync_plan)
        {
            let rollout_rollback =
                if let (Some(original_path), Some(current_path), Some(original)) = (
                    rollout_path.as_deref(),
                    written_rollout.as_deref(),
                    original_rollout.as_deref(),
                ) {
                    Some(rollback_rollout(original_path, current_path, original))
                } else {
                    None
                };
            let index_rollback = session_index_update.as_ref().map(restore_session_index);
            let history_rollback = Some(restore_thread_history_backup(&backup_dir, &session_id));
            let database_rollback = Some(restore_session_database_backup(&backup_dir));
            return Err(format_save_rollback_error(
                format!("Thread history sync failed: {error}"),
                rollout_rollback,
                index_rollback,
                database_rollback,
                history_rollback,
                &backup_dir,
            ));
        }
    }

    let _ = prune_context_edit_backups(codex_home);

    Ok(SaveCodexSessionContextResult {
        success: true,
        session_id,
        backup_path: Some(backup_dir.to_string_lossy().to_string()),
        message: "Session context successfully updated".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_sessions::tests::test_root;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn dynamic_session_id() -> String {
        format!(
            "019f1234-abcd-7000-8000-{:012x}",
            TEST_SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed)
        )
    }
    #[test]
    fn legacy_message_copies_follow_edits_and_deletions_in_their_turn() {
        for canonical in [false, true] {
            for deleted in [false, true] {
                for role in ["user", "assistant"] {
                    let root = test_root("legacy-message-mirrors");
                    let session_id = dynamic_session_id();
                    let fixture = seed_context_edit_fixture(&root, &session_id);
                    let mut records =
                        vec![json!({"type":"session_meta","payload":{"id":session_id}})];
                    for turn in ["edited-turn", "untouched-turn"] {
                        records.push(json!({"type":"event_msg","payload":{"type":"task_started","turn_id":turn}}));
                        records.push(json!({"type":"event_msg","payload":{"type":"user_message","message":"Repeated user","images":[],"local_images":[]}}));
                        records.push(json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Repeated user"}]}}));
                        if canonical {
                            records.push(json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":turn,"item":{"type":"UserMessage","id":"user-id","content":[{"type":"text","text":"Repeated user"}]}}}));
                        }
                        records.push(json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Repeated answer"}]}}));
                        records.push(json!({"type":"event_msg","payload":{"type":"agent_message","message":"Repeated answer","phase":"final_answer"}}));
                        if canonical {
                            records.push(json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":turn,"item":{"type":"AgentMessage","id":"agent-id","content":[{"type":"Text","text":"Repeated answer"}]}}}));
                        }
                        records.push(json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":turn,"last_agent_message":"Repeated answer"}}));
                    }
                    let original = serialize_jsonl(&records, "\r\n").unwrap();
                    fs::write(&fixture.rollout, &original).unwrap();
                    let connection = Connection::open(&fixture.history_db).unwrap();
                    connection.execute("UPDATE thread_history_projection_state SET next_rollout_byte_offset=?1,next_rollout_ordinal=?2", rusqlite::params![original.len() as i64, records.len() as i64]).unwrap();
                    drop(connection);
                    let context = get_codex_session_context_from_home(&root, &session_id).unwrap();
                    let line = context
                        .messages
                        .iter()
                        .find(|message| message.role == role)
                        .unwrap()
                        .line_number;
                    let mut request = context_save_request(&context);
                    request.message_updates = Some(vec![CodexSessionMessageUpdate {
                        line_number: line,
                        role: None,
                        content: Some("Changed message".into()),
                        deleted: Some(deleted),
                    }]);
                    save_codex_session_context_from_home(&root, request).unwrap();
                    let saved = fs::read_to_string(&fixture.rollout).unwrap();
                    let parsed = validate_raw_jsonl(&saved, &session_id).unwrap();
                    let event_type = if role == "user" {
                        "user_message"
                    } else {
                        "agent_message"
                    };
                    let messages = parsed
                        .iter()
                        .filter(|(_, record)| record["payload"]["type"] == event_type)
                        .map(|(_, record)| record["payload"]["message"].as_str().unwrap())
                        .collect::<Vec<_>>();
                    let untouched = if role == "user" {
                        "Repeated user"
                    } else {
                        "Repeated answer"
                    };
                    assert_eq!(
                        messages,
                        if deleted {
                            vec![untouched]
                        } else {
                            vec!["Changed message", untouched]
                        }
                    );
                    assert!(saved.contains("\r\n"));
                    fs::remove_dir_all(root).unwrap();
                }
            }
        }
    }

    #[test]
    fn appending_to_an_empty_fork_creates_a_readable_turn_and_rolls_back_on_failure() {
        for fail_summary in [false, true] {
            let root = test_root("context-empty-fork-turn");
            let session_id = dynamic_session_id();
            let fixture = seed_context_edit_fixture(&root, &session_id);
            let original = serialize_jsonl(&[json!({"type":"session_meta","payload":{"id":session_id,"forked_from_id":"parent-thread"}})], "\n").unwrap();
            fs::write(&fixture.rollout, &original).unwrap();
            let connection = Connection::open(&fixture.history_db).unwrap();
            connection.execute_batch("DELETE FROM thread_items; ALTER TABLE thread_turns ADD COLUMN rollout_byte_offset INTEGER; ALTER TABLE thread_turns ADD COLUMN rollout_end_ordinal INTEGER; ALTER TABLE thread_turns ADD COLUMN rollout_end_byte_offset INTEGER; CREATE UNIQUE INDEX turns_page ON thread_turns(thread_id, rollout_ordinal);").unwrap();
            connection.execute("UPDATE thread_history_projection_state SET next_rollout_byte_offset=?1,next_rollout_ordinal=1", [original.len() as i64]).unwrap();
            if fail_summary {
                connection.execute_batch("CREATE TRIGGER fail_summary BEFORE UPDATE OF first_user_item_id ON thread_turns BEGIN SELECT RAISE(ABORT, 'summary failure'); END;").unwrap();
            }
            drop(connection);
            let context = get_codex_session_context_from_home(&root, &session_id).unwrap();
            let mut request = context_save_request(&context);
            request.new_messages = Some(vec![
                CodexSessionNewMessage {
                    role: "user".into(),
                    content: "New question".into(),
                },
                CodexSessionNewMessage {
                    role: "assistant".into(),
                    content: "New answer".into(),
                },
            ]);
            let result = save_codex_session_context_from_home(&root, request);
            let connection = Connection::open(&fixture.history_db).unwrap();
            if fail_summary {
                assert!(result.unwrap_err().contains("summary failure"));
                assert_eq!(fs::read(&fixture.rollout).unwrap(), original);
                let counts: (i64,i64) = connection.query_row("SELECT (SELECT COUNT(*) FROM thread_items), (SELECT COUNT(*) FROM thread_turns)", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
                assert_eq!(counts, (0, 0));
            } else {
                result.unwrap();
                let summary: (String, String, String, i64, i64) = connection.query_row(
                    "SELECT turns.status, user.item_json, agent.item_json, turns.rollout_ordinal, turns.rollout_end_byte_offset FROM thread_turns turns JOIN thread_items user ON user.thread_id=turns.thread_id AND user.turn_id=turns.turn_id AND user.item_id=turns.first_user_item_id JOIN thread_items agent ON agent.thread_id=turns.thread_id AND agent.turn_id=turns.turn_id AND agent.item_id=turns.final_agent_item_id WHERE turns.thread_id=?1",
                    [&session_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).unwrap();
                assert_eq!(summary.0, "completed");
                assert!(summary.1.contains("New question"));
                assert!(summary.2.contains("New answer"));
                assert_eq!(summary.3, 1);
                assert_eq!(
                    summary.4,
                    fs::metadata(&fixture.rollout).unwrap().len() as i64
                );
                let saved = fs::read_to_string(&fixture.rollout).unwrap();
                assert!(saved.contains("\"type\":\"task_started\""));
                assert!(saved.contains("\"last_agent_message\":\"New answer\""));
            }
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn session_context_validation_rejects_path_traversal() {
        assert!(validate_session_id("../etc/passwd").is_err());
        assert!(validate_session_id("..\\windows\\system32").is_err());
        assert!(validate_session_id("test/session").is_err());
        assert!(validate_session_id("test:session").is_err());
        assert!(validate_session_id("").is_err());
        assert!(validate_session_id("   ").is_err());
        assert_eq!(
            validate_session_id("01a0ba19-e1ac-7d00-9136-563b25bbdb86").unwrap(),
            "01a0ba19-e1ac-7d00-9136-563b25bbdb86"
        );
    }

    #[test]
    fn structured_changes_reject_nonmessage_and_missing_lines() {
        let source = b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"s\"}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[]}}\n";
        for line_number in [0, 2] {
            for deleted in [true, false] {
                let mut request = empty_context_request("s".to_string(), None);
                request.message_updates = Some(vec![CodexSessionMessageUpdate {
                    line_number,
                    role: None,
                    content: Some("invalid update".to_string()),
                    deleted: Some(deleted),
                }]);
                assert!(build_updated_rollout(source, "s", &request).is_err());
            }
        }
    }

    #[test]
    fn context_lookup_does_not_open_unrelated_rollouts() {
        let root = test_root("context-no-unrelated-read");
        fs::create_dir_all(root.join("sessions")).unwrap();
        let session_id = dynamic_session_id();
        let other_id = dynamic_session_id();
        let content = format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":session_id}})
        );
        fs::write(
            root.join("sessions")
                .join(format!("rollout-{other_id}.jsonl")),
            content,
        )
        .unwrap();
        assert!(find_rollout_for_session(&root, &session_id)
            .unwrap()
            .is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_jsonl_limit_keeps_structured_messages_and_original_path() {
        let root = test_root("context-export-limit");
        fs::create_dir_all(root.join("sessions")).unwrap();
        let session_id = dynamic_session_id();
        let path = root
            .join("sessions")
            .join(format!("rollout-{session_id}.jsonl"));
        let record = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"still readable"}]}}).to_string();
        for size in [MAX_RAW_JSONL_BYTES, MAX_RAW_JSONL_BYTES + 1] {
            let mut bytes = format!("{record}\n").into_bytes();
            bytes.resize(size as usize, b' ');
            fs::write(&path, &bytes).unwrap();
            let detail = get_codex_session_context_from_home(&root, &session_id).unwrap();
            assert_eq!(detail.raw_jsonl_available, size <= MAX_RAW_JSONL_BYTES);
            assert_eq!(detail.raw_jsonl.is_some(), detail.raw_jsonl_available);
            assert_eq!(
                detail.rollout_sha256.as_deref(),
                Some(sha256_hex(&bytes).as_str())
            );
            assert_eq!(detail.stats.file_size_bytes, bytes.len() as u64);
            if let Some(raw) = detail.raw_jsonl.as_ref() {
                assert_eq!(raw.as_bytes(), bytes);
            }
            assert_eq!(detail.messages[0].content, "still readable");
            assert_eq!(
                find_rollout_for_session(&root, &session_id)
                    .unwrap()
                    .unwrap(),
                fs::canonicalize(&path).unwrap()
            );
        }
        fs::write(&path, [record.as_bytes(), b"\n\xff"].concat()).unwrap();
        let detail = get_codex_session_context_from_home(&root, &session_id).unwrap();
        assert!(!detail.raw_jsonl_available);
        assert_eq!(detail.messages[0].content, "still readable");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_get_and_save_workflow() {
        let root = test_root("context-workflow");
        let session_id = "019f1234-abcd-7000-8000-000000000001";
        let session_dir = root.join("sessions").join("2026").join("10").join("05");
        fs::create_dir_all(&session_dir).unwrap();

        let rollout_file =
            session_dir.join(format!("rollout-2026-10-05T12-00-00-{session_id}.jsonl"));
        let meta_line = json!({"type":"session_meta","payload":{"id":session_id,"title":"Initial Title","cwd":"C:\\Repo","model_provider":"openai"}}).to_string();
        let turn_line =
            json!({"type":"turn_context","payload":{"model":"gpt-5.4","cwd":"C:\\Repo"}})
                .to_string();
        let user_line = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"First prompt"}]}}).to_string();
        let asst_line = json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"First answer"}]}}).to_string();
        let initial_jsonl = format!("{meta_line}\n{turn_line}\n{user_line}\n{asst_line}\n");
        fs::write(&rollout_file, &initial_jsonl).unwrap();
        let database_path = root.join("state_5.sqlite");
        let connection = Connection::open(&database_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL, title TEXT, cwd TEXT, model_provider TEXT, model TEXT, archived INTEGER NOT NULL DEFAULT 0, archived_at INTEGER, updated_at INTEGER, updated_at_ms INTEGER);\n\
                 CREATE TABLE local_thread_catalog (thread_id TEXT PRIMARY KEY, display_title TEXT, cwd TEXT, model_provider TEXT, source_updated_at REAL);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, title, cwd, model_provider, model, archived, updated_at, updated_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 1, 1000)",
                rusqlite::params![
                    session_id,
                    rollout_file.to_string_lossy().to_string(),
                    "Initial Title",
                    "C:\\Repo",
                    "openai",
                    "gpt-5.4"
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO local_thread_catalog VALUES (?1, ?2, ?3, ?4, 1)",
                rusqlite::params![session_id, "Initial Title", "C:\\Repo", "openai"],
            )
            .unwrap();
        drop(connection);

        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        assert_eq!(detail.id, session_id);
        assert_eq!(detail.title, "Initial Title");
        assert_eq!(detail.cwd, "C:\\Repo");
        assert_eq!(detail.model_provider, "openai");
        assert_eq!(detail.stats.user_message_count, 1);
        assert_eq!(detail.stats.assistant_message_count, 1);
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(detail.messages[0].content, "First prompt");
        assert_eq!(detail.messages[1].content, "First answer");
        assert!(detail.rollout_sha256.is_some());

        let save_req = SaveCodexSessionContextRequest {
            title: Some("Updated Title".to_string()),
            cwd: Some("C:\\NewRepo".to_string()),
            model_provider: Some("cpa-gui".to_string()),
            model: Some("deepseek-v4.1".to_string()),
            archived: Some(true),
            message_updates: Some(vec![
                CodexSessionMessageUpdate {
                    line_number: 2,
                    role: None,
                    content: Some("Modified prompt".to_string()),
                    deleted: None,
                },
                CodexSessionMessageUpdate {
                    line_number: 3,
                    role: None,
                    content: None,
                    deleted: Some(true),
                },
            ]),
            new_messages: Some(vec![CodexSessionNewMessage {
                role: "assistant".to_string(),
                content: "Brand new response".to_string(),
            }]),
            ..empty_context_request(session_id.to_string(), detail.rollout_sha256.clone())
        };
        let save_res = save_codex_session_context_from_home(&root, save_req).unwrap();
        assert!(save_res.success);
        assert!(save_res.backup_path.is_some());
        let backup = PathBuf::from(save_res.backup_path.as_ref().unwrap());
        assert!(backup.join("database-rows.json").is_file());
        let archived_rollout = root
            .join("archived_sessions")
            .join(rollout_file.file_name().unwrap());
        assert!(!rollout_file.exists());
        assert!(archived_rollout.is_file());
        let saved_jsonl = fs::read_to_string(&archived_rollout).unwrap();
        assert!(saved_jsonl.contains("\"model\":\"deepseek-v4.1\""));
        assert!(saved_jsonl.contains("\"type\":\"output_text\""));

        let connection = Connection::open(&database_path).unwrap();
        let database_state = connection
            .query_row(
                "SELECT archived, archived_at, rollout_path, model FROM threads WHERE id = ?1",
                [session_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(database_state.0, 1);
        assert!(database_state.1.is_some());
        assert_eq!(PathBuf::from(database_state.2), archived_rollout);
        assert_eq!(database_state.3, "deepseek-v4.1");
        drop(connection);

        let updated_detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        assert_eq!(updated_detail.title, "Updated Title");
        assert_eq!(updated_detail.cwd, "C:\\NewRepo");
        assert_eq!(updated_detail.model_provider, "cpa-gui");
        assert_eq!(updated_detail.messages.len(), 2);
        assert_eq!(updated_detail.messages[0].content, "Modified prompt");
        assert_eq!(updated_detail.messages[1].content, "Brand new response");

        let restore_detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        let restore_req = SaveCodexSessionContextRequest {
            archived: Some(false),
            ..empty_context_request(session_id.to_string(), restore_detail.rollout_sha256)
        };
        save_codex_session_context_from_home(&root, restore_req).unwrap();
        assert!(rollout_file.is_file());
        assert!(!archived_rollout.exists());
        let connection = Connection::open(&database_path).unwrap();
        let restored_state = connection
            .query_row(
                "SELECT archived, archived_at, rollout_path FROM threads WHERE id = ?1",
                [session_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(restored_state.0, 0);
        assert!(restored_state.1.is_none());
        assert_eq!(PathBuf::from(restored_state.2), rollout_file);
        drop(connection);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_refuses_database_rollout_outside_codex_home() {
        let root = test_root("context-outside-rollout");
        let session_id = "019f1234-abcd-7000-8000-000000000010";
        let outside = root.parent().unwrap().join(format!(
            "{}-outside.jsonl",
            root.file_name().unwrap().to_string_lossy()
        ));
        let outside_contents = format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":session_id}})
        );
        fs::write(&outside, &outside_contents).unwrap();
        let connection = Connection::open(root.join("state_5.sqlite")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, 'Outside')",
                rusqlite::params![session_id, outside.to_string_lossy().to_string()],
            )
            .unwrap();
        drop(connection);

        assert!(find_rollout_for_session(&root, session_id)
            .unwrap()
            .is_none());
        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        assert!(detail.rollout_path.is_none());
        let request = SaveCodexSessionContextRequest {
            ..empty_context_request(session_id.to_string(), None)
        };
        assert!(save_codex_session_context_from_home(&root, request).is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), outside_contents);

        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_does_not_match_parent_id_inside_child_filename() {
        let root = test_root("context-parent-child");
        let parent_id = "019f1234-abcd-7000-8000-000000000020";
        let child_id = "019f1234-abcd-7000-8000-000000000021";
        let directory = root.join("sessions/2026/10/05");
        fs::create_dir_all(&directory).unwrap();
        let rollout = directory.join(format!(
            "rollout-2026-10-05T12-00-00-{parent_id}_{child_id}.jsonl"
        ));
        fs::write(
            &rollout,
            format!(
                "{}\n",
                json!({"type":"session_meta","payload":{"id":child_id}})
            ),
        )
        .unwrap();

        assert_eq!(
            find_rollout_for_session(&root, child_id).unwrap(),
            Some(fs::canonicalize(&rollout).unwrap())
        );
        assert!(find_rollout_for_session(&root, parent_id)
            .unwrap()
            .is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_supports_legacy_thread_schema_without_optional_columns() {
        let root = test_root("context-legacy-schema");
        let session_id = "019f1234-abcd-7000-8000-000000000002";
        let sessions = root.join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let rollout = sessions.join(format!("rollout-{session_id}.jsonl"));
        fs::write(
            &rollout,
            format!(
                "{}\n",
                json!({"type":"session_meta","payload":{"id":session_id,"title":"Legacy"}})
            ),
        )
        .unwrap();
        let database = root.join("state_5.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, cwd TEXT, model_provider TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    session_id,
                    rollout.to_string_lossy().to_string(),
                    "Legacy",
                    "C:\\Repo",
                    "openai"
                ],
            )
            .unwrap();
        drop(connection);

        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        assert_eq!(detail.model, None);
        assert_eq!(detail.updated_at_ms, None);
        assert_eq!(detail.title, "Legacy");

        let request = SaveCodexSessionContextRequest {
            title: Some("Updated legacy".to_string()),
            ..empty_context_request(session_id.to_string(), detail.rollout_sha256)
        };
        save_codex_session_context_from_home(&root, request).unwrap();

        let connection = Connection::open(&database).unwrap();
        let title = connection
            .query_row(
                "SELECT title FROM threads WHERE id = ?1",
                [session_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(title, "Updated legacy");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_updates_legacy_name_column() {
        let root = test_root("context-name-column");
        let session_id = "019f1234-abcd-7000-8000-000000000094";
        fs::create_dir_all(root.join("sessions")).unwrap();
        let rollout = root
            .join("sessions")
            .join(format!("rollout-{session_id}.jsonl"));
        fs::write(
            &rollout,
            format!(
                "{}\n",
                json!({"type":"session_meta","payload":{"id":session_id,"title":"Legacy"}})
            ),
        )
        .unwrap();
        let database = root.join("state_5.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, name TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, ?3)",
                rusqlite::params![session_id, rollout.to_string_lossy().to_string(), "Legacy"],
            )
            .unwrap();
        drop(connection);

        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        assert_eq!(detail.title, "Legacy");
        let mut request = context_save_request(&detail);
        request.title = Some("Updated legacy name".to_string());
        save_codex_session_context_from_home(&root, request).unwrap();
        let updated = get_codex_session_context_from_home(&root, session_id).unwrap();
        assert_eq!(updated.title, "Updated legacy name");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_uses_latest_model_and_retains_message_turn_models() {
        let root = test_root("context-latest-model");
        let session_id = "019f1234-abcd-7000-8000-000000000095";
        fs::create_dir_all(root.join("sessions")).unwrap();
        let rollout = root
            .join("sessions")
            .join(format!("rollout-{session_id}.jsonl"));
        let records = vec![
            json!({"type":"session_meta","payload":{"id":session_id}}),
            json!({"type":"turn_context","payload":{"model":"first-model"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"First turn"}]}}),
            json!({"type":"turn_context","payload":{"model":"latest-model"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Latest turn"}]}}),
        ];
        fs::write(&rollout, serialize_jsonl(&records, "\n").unwrap()).unwrap();
        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        assert_eq!(detail.model.as_deref(), Some("latest-model"));
        assert_eq!(detail.messages[0].model.as_deref(), Some("first-model"));
        assert_eq!(detail.messages[1].model.as_deref(), Some("latest-model"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_rejects_stale_rollout_snapshot() {
        let root = test_root("context-stale-snapshot");
        let session_id = "019f1234-abcd-7000-8000-000000000040";
        let directory = root.join("sessions");
        fs::create_dir_all(&directory).unwrap();
        let rollout = directory.join(format!("rollout-{session_id}.jsonl"));
        let initial = format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":session_id,"title":"Initial"}})
        );
        fs::write(&rollout, &initial).unwrap();
        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        let changed = format!(
            "{initial}{}\n",
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Concurrent append"}]}})
        );
        fs::write(&rollout, &changed).unwrap();
        let request = SaveCodexSessionContextRequest {
            title: Some("Should not be written".to_string()),
            ..empty_context_request(session_id.to_string(), detail.rollout_sha256)
        };
        let error = save_codex_session_context_from_home(&root, request).unwrap_err();
        assert!(error.contains("changed after it was loaded"));
        assert_eq!(fs::read_to_string(&rollout).unwrap(), changed);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_rolls_back_rollout_when_database_update_fails() {
        let root = test_root("context-database-rollback");
        let session_id = "019f1234-abcd-7000-8000-000000000050";
        let directory = root.join("sessions");
        fs::create_dir_all(&directory).unwrap();
        let rollout = directory.join(format!("rollout-{session_id}.jsonl"));
        let initial = format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":session_id,"title":"Initial"}})
        );
        fs::write(&rollout, &initial).unwrap();
        let database = root.join("state_5.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT);\n\
                 CREATE TRIGGER reject_context_update BEFORE UPDATE ON threads BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, 'Initial')",
                rusqlite::params![session_id, rollout.to_string_lossy().to_string()],
            )
            .unwrap();
        drop(connection);
        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        let request = SaveCodexSessionContextRequest {
            title: Some("Updated".to_string()),
            ..empty_context_request(session_id.to_string(), detail.rollout_sha256)
        };
        let error = save_codex_session_context_from_home(&root, request).unwrap_err();
        assert!(error.contains("rollout restored"));
        assert_eq!(fs::read_to_string(&rollout).unwrap(), initial);
        let connection = Connection::open(&database).unwrap();
        let title = connection
            .query_row(
                "SELECT title FROM threads WHERE id = ?1",
                [session_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(title, "Initial");
        let backup_root = root
            .join("backups_state")
            .join("easy-cli-proxy-api")
            .join("session-context-edits");
        let backup = fs::read_dir(backup_root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(backup.join("database-rows.json").is_file());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_rolls_back_session_index_when_database_update_fails() {
        let root = test_root("context-index-database-rollback");
        let session_id = "019f1234-abcd-7000-8000-000000000051";
        let directory = root.join("sessions");
        fs::create_dir_all(&directory).unwrap();
        let rollout = directory.join(format!("rollout-{session_id}.jsonl"));
        let initial = format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":session_id,"title":"Initial"}})
        );
        fs::write(&rollout, &initial).unwrap();
        let index_path = root.join("session_index.jsonl");
        let initial_index = format!(
            "{}\r\n",
            json!({"id":session_id,"thread_name":"Initial","updated_at":"2026-10-05T00:00:00Z"})
        );
        fs::write(&index_path, &initial_index).unwrap();

        let database = root.join("state_5.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT);\n\
                 CREATE TRIGGER reject_context_update BEFORE UPDATE ON threads BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, 'Initial')",
                rusqlite::params![session_id, rollout.to_string_lossy().to_string()],
            )
            .unwrap();
        drop(connection);

        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        let request = SaveCodexSessionContextRequest {
            title: Some("Updated".to_string()),
            ..empty_context_request(session_id.to_string(), detail.rollout_sha256)
        };
        let error = save_codex_session_context_from_home(&root, request).unwrap_err();
        assert!(error.contains("session index restored"));
        assert_eq!(fs::read_to_string(&rollout).unwrap(), initial);
        let saved_index = fs::read(&index_path).unwrap();
        assert!(saved_index.starts_with(initial_index.as_bytes()));
        assert_eq!(
            latest_session_index_entry(&saved_index, session_id)
                .unwrap()
                .unwrap()["thread_name"],
            "Initial"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_rejects_active_writer_lock() {
        let root = test_root("context-writer-lock");
        let session_id = "019f1234-abcd-7000-8000-000000000060";
        let directory = root.join("sessions");
        fs::create_dir_all(&directory).unwrap();
        let rollout = directory.join(format!("rollout-{session_id}.jsonl"));
        fs::write(
            &rollout,
            format!(
                "{}\n",
                json!({"type":"session_meta","payload":{"id":session_id}})
            ),
        )
        .unwrap();
        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        let lock_directory = root.join("thread-writer-locks");
        fs::create_dir_all(&lock_directory).unwrap();
        let lock_file =
            fs::File::create(lock_directory.join(format!("{session_id}.lock"))).unwrap();
        lock_file.lock().unwrap();
        let request = SaveCodexSessionContextRequest {
            title: Some("Blocked".to_string()),
            ..empty_context_request(session_id.to_string(), detail.rollout_sha256)
        };
        let error = save_codex_session_context_from_home(&root, request).unwrap_err();
        assert!(error.contains("currently active"));
        lock_file.unlock().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_honors_writer_coordination_lock() {
        let root = test_root("context-writer-coordination");
        let directory = root.join("thread-writer-locks");
        fs::create_dir_all(&directory).unwrap();
        let coordination = fs::File::create(directory.join(".coordination.lock")).unwrap();
        coordination.lock().unwrap();
        let session_id = dynamic_session_id();
        assert!(acquire_session_writer_lock(&root, &session_id).is_err());
        assert!(!directory.join(format!("{session_id}.lock")).exists());
        coordination.unlock().unwrap();
        drop(acquire_session_writer_lock(&root, &session_id).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_rejects_overlong_session_id() {
        let long_id = "a".repeat(129);
        assert!(validate_session_id(&long_id).is_err());
        let valid_id = "a".repeat(128);
        assert!(validate_session_id(&valid_id).is_ok());
    }

    #[test]
    fn session_context_syncs_session_index_title_and_preserves_crlf() {
        let root = test_root("context-sync-index");
        let session_id = "019f1234-abcd-7000-8000-000000000088";
        let directory = root.join("sessions");
        fs::create_dir_all(&directory).unwrap();
        let rollout = directory.join(format!("rollout-{session_id}.jsonl"));
        let meta = json!({"type":"session_meta","payload":{"id":session_id,"title":"Old Title"}})
            .to_string();
        let user = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Hi"}]}}).to_string();
        let crlf_data = format!("{meta}\r\n{user}\r\n");
        fs::write(&rollout, crlf_data.as_bytes()).unwrap();

        let index_file = root.join("session_index.jsonl");
        let index_entry =
            json!({"id":session_id,"thread_name":"Old Title","updated_at":"2026-10-05T00:00:00Z"})
                .to_string();
        fs::write(&index_file, format!("{index_entry}\r\n")).unwrap();

        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        assert_eq!(detail.title, "Old Title");

        let request = SaveCodexSessionContextRequest {
            title: Some("Renamed Thread".to_string()),
            ..empty_context_request(session_id.to_string(), detail.rollout_sha256)
        };
        let save_res = save_codex_session_context_from_home(&root, request).unwrap();
        assert!(save_res.success);

        let saved_rollout = fs::read(&rollout).unwrap();
        assert!(saved_rollout.windows(2).any(|window| window == b"\r\n"));

        let updated_index = fs::read_to_string(&index_file).unwrap();
        assert!(updated_index.starts_with(&format!("{index_entry}\r\n")));
        assert!(updated_index.ends_with("\r\n"));
        assert_eq!(
            latest_session_index_entry(updated_index.as_bytes(), session_id)
                .unwrap()
                .unwrap()["thread_name"],
            "Renamed Thread"
        );

        let backup = PathBuf::from(save_res.backup_path.unwrap());
        assert!(backup.join("session_index.jsonl.bak").is_file());

        fs::remove_dir_all(root).unwrap();
    }

    fn empty_context_request(
        session_id: String,
        expected_rollout_sha256: Option<String>,
    ) -> SaveCodexSessionContextRequest {
        SaveCodexSessionContextRequest {
            session_id,
            expected_rollout_sha256,
            title: None,
            cwd: None,
            model_provider: None,
            model: None,
            archived: None,
            message_updates: None,
            new_messages: None,
        }
    }

    fn context_save_request(detail: &CodexSessionContextDetail) -> SaveCodexSessionContextRequest {
        SaveCodexSessionContextRequest {
            session_id: detail.id.clone(),
            expected_rollout_sha256: detail.rollout_sha256.clone(),
            title: None,
            cwd: None,
            model_provider: None,
            model: None,
            archived: None,
            message_updates: None,
            new_messages: None,
        }
    }

    #[test]
    fn session_context_replaces_all_text_parts_and_keeps_images() {
        let image = json!({"type":"input_image","image_url":"data:image/png;base64,fixture"});
        let mut content = json!([
            {"type":"input_text","text":"First part"},
            image.clone(),
            {"type":"input_text","text":"Second part"},
            "Third part"
        ]);
        update_content_text(&mut content, "Replacement", "output_text");
        assert_eq!(extract_content_text(&content), "Replacement");
        assert_eq!(
            content,
            json!([{"type":"output_text","text":"Replacement"}, image.clone()])
        );

        let mut image_only = json!([image.clone()]);
        update_content_text(&mut image_only, "New caption", "input_text");
        assert_eq!(
            image_only,
            json!([image, {"type":"input_text","text":"New caption"}])
        );
    }

    #[test]
    fn session_context_removes_tool_calls_and_outputs_together() {
        let root = test_root("context-tool-pairs");
        let session_id = "019f1234-abcd-7000-8000-000000000091";
        fs::create_dir_all(root.join("sessions")).unwrap();
        let rollout = root
            .join("sessions")
            .join(format!("rollout-{session_id}.jsonl"));
        let records = vec![
            json!({"type":"session_meta","payload":{"id":session_id}}),
            json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{}","call_id":"call-1"}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","output":"Output 1","call_id":"call-1"}}),
            json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"patch","call_id":"call-2"}}),
            json!({"type":"response_item","payload":{"type":"custom_tool_call_output","output":"Output 2","call_id":"call-2"}}),
        ];
        fs::write(&rollout, serialize_jsonl(&records, "\n").unwrap()).unwrap();
        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        assert_eq!(detail.stats.tool_count, 4);
        assert_eq!(detail.messages[1].content, "Output 1");
        assert_eq!(detail.messages[1].call_id.as_deref(), Some("call-1"));
        assert_eq!(detail.messages[2].content, "apply_patch(patch)");

        let mut edit_request = context_save_request(&detail);
        edit_request.message_updates = Some(vec![CodexSessionMessageUpdate {
            line_number: 2,
            role: None,
            content: Some("Blocked edit".to_string()),
            deleted: None,
        }]);
        assert!(save_codex_session_context_from_home(&root, edit_request)
            .unwrap_err()
            .contains("only supports removal"));

        let mut remove_request = context_save_request(&detail);
        remove_request.message_updates = Some(vec![CodexSessionMessageUpdate {
            line_number: 2,
            role: None,
            content: None,
            deleted: Some(true),
        }]);
        save_codex_session_context_from_home(&root, remove_request).unwrap();
        let saved = fs::read_to_string(&rollout).unwrap();
        assert!(!saved.contains("call-1"));
        assert!(saved.contains("call-2"));
        assert_eq!(
            get_codex_session_context_from_home(&root, session_id)
                .unwrap()
                .stats
                .tool_count,
            2
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_preserves_externally_changed_index_during_rollback() {
        let root = test_root("context-index-external-change");
        let path = root.join("session_index.jsonl");
        let id = dynamic_session_id();
        let other = dynamic_session_id();
        let original = format!("{}\n", json!({"id":id,"thread_name":"original"}));
        fs::write(&path, &original).unwrap();
        let update = build_session_index_title_update(&root, &id, "updated")
            .unwrap()
            .unwrap();
        // Another thread can append before and after this save.
        let mut writer = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(writer, "{}", json!({"id":other,"thread_name":"other"})).unwrap();
        apply_session_index_title_update(&update).unwrap();
        writeln!(
            writer,
            "{}",
            json!({"id":other,"thread_name":"newer other"})
        )
        .unwrap();
        restore_session_index(&update).unwrap();
        let current = fs::read(&path).unwrap();
        assert!(current.starts_with(original.as_bytes()));
        assert_eq!(
            latest_session_index_entry(&current, &id).unwrap().unwrap()["thread_name"],
            "original"
        );
        assert_eq!(
            latest_session_index_entry(&current, &other)
                .unwrap()
                .unwrap()["thread_name"],
            "newer other"
        );
        let update = build_session_index_title_update(&root, &id, "updated again")
            .unwrap()
            .unwrap();
        apply_session_index_title_update(&update).unwrap();
        writeln!(writer, "{}", json!({"id":id,"thread_name":"external"})).unwrap();
        let externally_changed = fs::read(&path).unwrap();
        assert!(restore_session_index(&update)
            .unwrap_err()
            .contains("refusing to overwrite"));
        assert!(apply_session_index_title_update(&update).is_err());
        assert_eq!(fs::read(&path).unwrap(), externally_changed);
        drop(writer);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_rejects_incomplete_database_inspection_before_writing() {
        let root = test_root("context-corrupt-database");
        let session_id = "019f1234-abcd-7000-8000-000000000093";
        fs::create_dir_all(root.join("sessions")).unwrap();
        let rollout = root
            .join("sessions")
            .join(format!("rollout-{session_id}.jsonl"));
        let original = format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":session_id}})
        );
        fs::write(&rollout, &original).unwrap();
        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        fs::write(root.join("state_5.sqlite"), b"not a database").unwrap();
        let mut request = context_save_request(&detail);
        request.title = Some("Blocked change".to_string());
        let error = save_codex_session_context_from_home(&root, request).unwrap_err();
        assert!(error.contains("saving aborted"));
        assert_eq!(fs::read_to_string(&rollout).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_context_syncs_thread_history_and_event_messages() {
        verify_context_projection_edit(false);
    }

    #[test]
    fn session_context_deletes_message_and_syncs_sqlite_and_events() {
        verify_context_projection_edit(true);
    }

    fn verify_context_projection_edit(deleted: bool) {
        for event_first in [false, true] {
            for retained_source in [false, true] {
                let root = test_root("context-projection-edit");
                let session_id = dynamic_session_id();
                let fixture = seed_context_edit_fixture(&root, &session_id);
                let mut records =
                    validate_raw_jsonl(&fs::read_to_string(&fixture.rollout).unwrap(), &session_id)
                        .unwrap()
                        .into_iter()
                        .map(|(_, record)| record)
                        .collect::<Vec<_>>();
                if !retained_source {
                    for record in &mut records {
                        record.as_object_mut().unwrap().remove("metadata");
                    }
                }
                if event_first {
                    records.swap(6, 7);
                }
                fs::write(&fixture.rollout, serialize_jsonl(&records, "\n").unwrap()).unwrap();
                let connection = Connection::open(&fixture.history_db).unwrap();
                connection.execute("UPDATE thread_history_projection_state SET next_rollout_byte_offset=?1 WHERE thread_id=?2", rusqlite::params![fs::metadata(&fixture.rollout).unwrap().len() as i64, session_id]).unwrap();
                connection.execute("INSERT INTO thread_items VALUES (?1,'turn-fixture','item-user-fixture',3,1,?2,'userMessage',0,1,1)",
                    rusqlite::params![session_id, json!({"type":"userMessage","content":[{"type":"text","text":"Question"}]}).to_string()]).unwrap();
                connection.execute("INSERT INTO thread_turns VALUES (?1,'turn-fixture',1,'completed',NULL,1,1,0,'item-user-fixture',?2)",
                    rusqlite::params![session_id,fixture.agent_item_id]).unwrap();
                drop(connection);
                let context = get_codex_session_context_from_home(&root, &session_id).unwrap();
                let mut request = context_save_request(&context);
                request.message_updates = Some(vec![CodexSessionMessageUpdate {
                    line_number: assistant_line_number(&context),
                    role: None,
                    content: Some("Edited answer".into()),
                    deleted: Some(deleted),
                }]);
                let result = save_codex_session_context_from_home(&root, request).unwrap();
                let saved = fs::read_to_string(&fixture.rollout).unwrap();
                let connection = Connection::open(&fixture.history_db).unwrap();
                let agent: Option<String> = connection
                    .query_row(
                        "SELECT item_json FROM thread_items WHERE item_id = ?1",
                        [fixture.agent_item_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .unwrap();
                if deleted {
                    assert!(agent.is_none());
                    assert!(!saved.contains(fixture.agent_item_id));
                    let final_id: Option<String> = connection
                        .query_row("SELECT final_agent_item_id FROM thread_turns", [], |row| {
                            row.get(0)
                        })
                        .unwrap();
                    assert!(final_id.is_none());
                } else {
                    assert!(agent.unwrap().contains("Edited answer"));
                    assert_eq!(saved.matches("Edited answer").count(), 3);
                    let records = validate_raw_jsonl(&saved, &session_id).unwrap();
                    let event = records
                        .iter()
                        .find(|(_, record)| {
                            record["payload"]["item"]["id"] == fixture.agent_item_id
                        })
                        .unwrap();
                    assert_eq!(event.1["payload"]["item"]["content"][0]["type"], "Text");
                }
                let cursor: (i64,i64) = connection.query_row("SELECT next_rollout_byte_offset,next_rollout_ordinal FROM thread_history_projection_state", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
                assert_eq!(cursor, (saved.len() as i64, 9));
                let user: String = connection
                    .query_row(
                        "SELECT item_json FROM thread_items WHERE item_id = 'item-user-fixture'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert!(user.contains("Question"));
                assert!(PathBuf::from(result.backup_path.unwrap())
                    .join("thread-history-rows.json")
                    .is_file());
                drop(connection);
                fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn session_context_edits_reasoning_and_strips_signature() {
        let root = test_root("context-reasoning-strip");
        let session_id = dynamic_session_id();
        let fixture = seed_context_edit_fixture(&root, &session_id);
        let context = get_codex_session_context_from_home(&root, &session_id).unwrap();
        let mut request = context_save_request(&context);
        request.message_updates = Some(vec![CodexSessionMessageUpdate {
            line_number: context
                .messages
                .iter()
                .find(|message| message.role == "reasoning")
                .unwrap()
                .line_number,
            role: None,
            content: Some("New clean edited thoughts".to_string()),
            deleted: None,
        }]);
        save_codex_session_context_from_home(&root, request).unwrap();
        let rollout = fs::read_to_string(&fixture.rollout).unwrap();
        let projected =
            thread_item_json(&fixture.history_db, &session_id, fixture.reasoning_item_id);
        for copy in [rollout, projected] {
            assert!(copy.contains("New clean edited thoughts"));
            assert!(copy.contains("\"signature_stripped\":true"));
            assert!(!copy.contains("fake-signature"));
            assert!(!copy.contains("encrypted-blob"));
        }
        fs::remove_dir_all(root).unwrap();
    }

    struct ContextEditFixture {
        rollout: PathBuf,
        state_db: PathBuf,
        history_db: PathBuf,
        agent_item_id: &'static str,
        reasoning_item_id: &'static str,
    }

    /// Seeds a rollout that carries both copies of every item (`response_item`
    /// and `item_completed`), a state database and a thread history projection,
    /// so the projection contract can be asserted end to end.
    fn seed_context_edit_fixture(root: &Path, session_id: &str) -> ContextEditFixture {
        let turn_id = "turn-fixture";
        let user_item_id = "item-user-fixture";
        let reasoning_item_id = "item-reasoning-fixture";
        let agent_item_id = "item-agent-fixture";

        let sessions_dir = root.join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let rollout = sessions_dir.join(format!("rollout-{session_id}.jsonl"));
        let lines = [
            json!({"ordinal": 0, "type": "session_meta", "payload": {"id": session_id, "title": "Original Title", "cwd": "/old", "model_provider": "openai"}}).to_string(),
            json!({"ordinal": 1, "type": "event_msg", "payload": {"type": "task_started", "turn_id": turn_id}}).to_string(),
            json!({"ordinal": 2, "type": "response_item", "metadata": {"retained_source": {"id": {"message_id": user_item_id, "role": "user", "turn_id": turn_id}}}, "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Question"}]}}).to_string(),
            json!({"ordinal": 3, "type": "event_msg", "payload": {"type": "item_completed", "thread_id": session_id, "turn_id": turn_id, "item": {"id": user_item_id, "type": "UserMessage", "content": [{"type": "text", "text": "Question", "text_elements": []}]}}}).to_string(),
            json!({"ordinal": 4, "type": "response_item", "payload": {"type": "reasoning", "id": reasoning_item_id, "summary": [{"type": "summary_text", "text": "Original thoughts"}], "content": [], "signature": "fake-signature", "encrypted_content": "encrypted-blob"}}).to_string(),
            json!({"ordinal": 5, "type": "event_msg", "payload": {"type": "item_completed", "thread_id": session_id, "turn_id": turn_id, "item": {"id": reasoning_item_id, "type": "Reasoning", "summary_text": ["Original thoughts"], "raw_content": ["Original raw thoughts"], "signature": "fake-signature"}}}).to_string(),
            json!({"ordinal": 6, "type": "response_item", "metadata": {"retained_source": {"id": {"message_id": agent_item_id, "role": "assistant", "turn_id": turn_id}}}, "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Answer"}]}}).to_string(),
            json!({"ordinal": 7, "type": "event_msg", "payload": {"type": "item_completed", "thread_id": session_id, "turn_id": turn_id, "item": {"id": agent_item_id, "type": "AgentMessage", "content": [{"type": "Text", "text": "Answer"}]}}}).to_string(),
            json!({"ordinal": 8, "type": "event_msg", "payload": {"type": "task_complete", "turn_id": turn_id, "last_agent_message": "Answer"}}).to_string(),
        ];
        fs::write(&rollout, format!("{}\n", lines.join("\n"))).unwrap();

        let state_db = root.join("state_5.sqlite");
        let connection = Connection::open(&state_db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, cwd TEXT, model_provider TEXT, archived INTEGER, updated_at_ms INTEGER);\n\
                 CREATE TABLE local_thread_catalog (thread_id TEXT PRIMARY KEY, display_title TEXT, cwd TEXT, model_provider TEXT, source_updated_at REAL);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, 'Original Title', '/old', 'openai', 0, 100)",
                rusqlite::params![session_id, rollout.to_string_lossy().to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO local_thread_catalog VALUES (?1, 'Original Title', '/old', 'openai', 1.0)",
                [session_id],
            )
            .unwrap();
        drop(connection);

        let history_db = root.join("thread_history_1.sqlite");
        let connection = Connection::open(&history_db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE thread_items (
                    thread_id TEXT NOT NULL,
                    turn_id TEXT NOT NULL,
                    item_id TEXT NOT NULL,
                    rollout_ordinal INTEGER NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    item_json TEXT NOT NULL,
                    item_type TEXT NOT NULL,
                    updated_at_ordinal INTEGER NOT NULL,
                    started_at_ms INTEGER,
                    completed_at_ms INTEGER,
                    PRIMARY KEY (thread_id, turn_id, item_id)
                );
                CREATE TABLE thread_turns (
                    thread_id TEXT NOT NULL,
                    turn_id TEXT NOT NULL,
                    rollout_ordinal INTEGER NOT NULL,
                    status TEXT NOT NULL,
                    error_json TEXT,
                    started_at INTEGER,
                    completed_at INTEGER,
                    duration_ms INTEGER,
                    first_user_item_id TEXT,
                    final_agent_item_id TEXT,
                    PRIMARY KEY (thread_id, turn_id)
                );
                CREATE TABLE thread_history_projection_state (
                    thread_id TEXT PRIMARY KEY,
                    next_rollout_byte_offset INTEGER NOT NULL,
                    next_rollout_ordinal INTEGER NOT NULL
                );",
            )
            .unwrap();
        let agent_json =
            json!({"type": "agentMessage", "id": agent_item_id, "text": "Answer"}).to_string();
        connection
            .execute(
                "INSERT INTO thread_items VALUES (?1, ?2, ?3, 6, 1500, ?4, 'agentMessage', 0, 1500, 1500)",
                rusqlite::params![session_id, turn_id, agent_item_id, agent_json],
            )
            .unwrap();
        let reasoning_json = json!({
            "type": "reasoning",
            "id": reasoning_item_id,
            "summary": ["Original thoughts"],
            "content": ["Original raw thoughts"],
            "signature": "fake-signature",
            "encrypted_content": "encrypted-blob"
        })
        .to_string();
        connection
            .execute(
                "INSERT INTO thread_items VALUES (?1, ?2, ?3, 4, 1500, ?4, 'reasoning', 0, 1500, 1500)",
                rusqlite::params![session_id, turn_id, reasoning_item_id, reasoning_json],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO thread_history_projection_state VALUES (?1, ?2, 9)",
                rusqlite::params![session_id, fs::metadata(&rollout).unwrap().len() as i64],
            )
            .unwrap();
        drop(connection);

        ContextEditFixture {
            rollout,
            state_db,
            history_db,
            agent_item_id,
            reasoning_item_id,
        }
    }

    fn thread_item_json(history_db: &Path, session_id: &str, item_id: &str) -> String {
        let connection = Connection::open(history_db).unwrap();
        connection
            .query_row(
                "SELECT item_json FROM thread_items WHERE thread_id = ?1 AND item_id = ?2",
                rusqlite::params![session_id, item_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn thread_item_json_for_turn(
        history_db: &Path,
        session_id: &str,
        turn_id: &str,
        item_id: &str,
    ) -> String {
        let connection = Connection::open(history_db).unwrap();
        connection
            .query_row(
                "SELECT item_json FROM thread_items WHERE thread_id = ?1 AND turn_id = ?2 AND item_id = ?3",
                rusqlite::params![session_id, turn_id, item_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn assistant_line_number(context: &CodexSessionContextDetail) -> usize {
        context
            .messages
            .iter()
            .find(|message| message.role == "assistant")
            .unwrap()
            .line_number
    }

    #[test]
    fn context_save_restores_the_state_database_when_thread_history_sync_fails() {
        let root = test_root("context-state-db-rollback");
        let session_id = "019f1234-abcd-7000-8000-0000000000e1";
        let fixture = seed_context_edit_fixture(&root, session_id);

        let connection = Connection::open(&fixture.history_db).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_thread_item_update BEFORE UPDATE ON thread_items BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
            )
            .unwrap();
        drop(connection);

        let context = get_codex_session_context_from_home(&root, session_id).unwrap();
        let mut request = context_save_request(&context);
        request.title = Some("Changed Title".to_string());
        request.message_updates = Some(vec![CodexSessionMessageUpdate {
            line_number: assistant_line_number(&context),
            role: None,
            content: Some("Answer edited".to_string()),
            deleted: None,
        }]);

        let error = save_codex_session_context_from_home(&root, request).unwrap_err();
        assert!(error.contains("Thread history sync failed"), "{error}");
        assert!(error.contains("session database restored"), "{error}");

        let connection = Connection::open(&fixture.state_db).unwrap();
        let title: String = connection
            .query_row(
                "SELECT title FROM threads WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        let catalog: (String, f64) = connection
            .query_row(
                "SELECT display_title, source_updated_at FROM local_thread_catalog WHERE thread_id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        drop(connection);
        assert_eq!(title, "Original Title");
        assert_eq!(catalog, ("Original Title".to_string(), 1.0));

        let rollout = fs::read_to_string(&fixture.rollout).unwrap();
        assert!(rollout.contains("Answer"));
        assert!(!rollout.contains("Answer edited"));
        assert!(rollout.contains("Original Title"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn context_save_fails_when_the_projection_cursor_cannot_be_updated() {
        let root = test_root("context-cursor-failure");
        let session_id = "019f1234-abcd-7000-8000-0000000000e5";
        let fixture = seed_context_edit_fixture(&root, session_id);

        let connection = Connection::open(&fixture.history_db).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_projection_state BEFORE UPDATE ON thread_history_projection_state BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
            )
            .unwrap();
        drop(connection);

        let context = get_codex_session_context_from_home(&root, session_id).unwrap();
        let mut request = context_save_request(&context);
        request.message_updates = Some(vec![CodexSessionMessageUpdate {
            line_number: assistant_line_number(&context),
            role: None,
            content: Some("Answer edited".to_string()),
            deleted: None,
        }]);

        let error = save_codex_session_context_from_home(&root, request).unwrap_err();
        assert!(error.contains("projection cursor"), "{error}");

        let rollout = fs::read_to_string(&fixture.rollout).unwrap();
        assert!(!rollout.contains("Answer edited"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn assistant_edit_preserves_untouched_reasoning_in_every_copy() {
        let root = test_root("context-signature-copies");
        let session_id = "019f1234-abcd-7000-8000-0000000000e4";
        let fixture = seed_context_edit_fixture(&root, session_id);
        let original_reasoning =
            thread_item_json(&fixture.history_db, session_id, fixture.reasoning_item_id);
        let original_records =
            validate_raw_jsonl(&fs::read_to_string(&fixture.rollout).unwrap(), session_id).unwrap();

        let context = get_codex_session_context_from_home(&root, session_id).unwrap();
        let mut request = context_save_request(&context);
        request.message_updates = Some(vec![CodexSessionMessageUpdate {
            line_number: assistant_line_number(&context),
            role: None,
            content: Some("Answer edited".to_string()),
            deleted: None,
        }]);
        save_codex_session_context_from_home(&root, request).unwrap();

        let rollout = fs::read_to_string(&fixture.rollout).unwrap();
        assert!(rollout.contains("Answer edited"));
        let saved_records = validate_raw_jsonl(&rollout, session_id).unwrap();
        for index in [4, 5] {
            assert_eq!(saved_records[index], original_records[index]);
        }

        let reasoning_json =
            thread_item_json(&fixture.history_db, session_id, fixture.reasoning_item_id);
        assert_eq!(reasoning_json, original_reasoning);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn thinking_edit_rewrites_the_original_thinking_field() {
        let root = test_root("context-thinking-field");
        let session_id = "019f1234-abcd-7000-8000-0000000000e6";
        let sessions_dir = root.join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let rollout = sessions_dir.join(format!("rollout-{session_id}.jsonl"));
        let lines = [
            json!({"type": "session_meta", "payload": {"id": session_id, "title": "Thinking"}}).to_string(),
            json!({"type": "response_item", "payload": {"type": "thinking", "id": "thinking-item", "thinking": "Original thinking", "signature": "thinking-signature"}}).to_string(),
        ];
        fs::write(&rollout, format!("{}\n", lines.join("\n"))).unwrap();

        let context = get_codex_session_context_from_home(&root, session_id).unwrap();
        let reasoning_line = context
            .messages
            .iter()
            .find(|message| message.role == "reasoning")
            .unwrap()
            .line_number;
        let mut request = context_save_request(&context);
        request.message_updates = Some(vec![CodexSessionMessageUpdate {
            line_number: reasoning_line,
            role: None,
            content: Some("Edited thinking".to_string()),
            deleted: None,
        }]);
        save_codex_session_context_from_home(&root, request).unwrap();

        let updated = get_codex_session_context_from_home(&root, session_id).unwrap();
        let reasoning = updated
            .messages
            .iter()
            .find(|message| message.role == "reasoning")
            .unwrap();
        assert_eq!(reasoning.content, "Edited thinking");
        assert!(reasoning.signature_stripped.unwrap_or(false));

        let saved = fs::read_to_string(&rollout).unwrap();
        assert!(
            saved.contains("\"thinking\":\"Edited thinking\""),
            "{saved}"
        );
        assert!(!saved.contains("Original thinking"));
        assert!(!saved.contains("thinking-signature"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn role_only_update_cannot_blank_the_projected_text() {
        let root = test_root("context-role-only");
        let session_id = "019f1234-abcd-7000-8000-0000000000e7";
        let fixture = seed_context_edit_fixture(&root, session_id);
        let before = fs::read_to_string(&fixture.rollout).unwrap();

        let context = get_codex_session_context_from_home(&root, session_id).unwrap();
        let mut request = context_save_request(&context);
        request.message_updates = Some(vec![CodexSessionMessageUpdate {
            line_number: assistant_line_number(&context),
            role: Some("user".to_string()),
            content: None,
            deleted: None,
        }]);
        let error = save_codex_session_context_from_home(&root, request).unwrap_err();
        assert!(error.contains("cannot change the message role"), "{error}");

        assert_eq!(fs::read_to_string(&fixture.rollout).unwrap(), before);
        let agent_json = thread_item_json(&fixture.history_db, session_id, fixture.agent_item_id);
        assert!(agent_json.contains("\"text\":\"Answer\""), "{agent_json}");

        fs::remove_dir_all(root).unwrap();
    }

    /// P1-3: a metadata-only save on a session without a rollout must leave the
    /// projection cursor untouched instead of resetting it to 0/0.
    #[test]
    fn metadata_save_without_a_rollout_keeps_the_projection_cursor() {
        let root = test_root("context-no-rollout-cursor");
        let session_id = "019f1234-abcd-7000-8000-0000000000f2";
        fs::create_dir_all(root.join("sessions")).unwrap();

        let state_db = root.join("state_5.sqlite");
        let connection = Connection::open(&state_db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, cwd TEXT, model_provider TEXT, archived INTEGER, updated_at_ms INTEGER);\n\
                 CREATE TABLE local_thread_catalog (thread_id TEXT PRIMARY KEY, display_title TEXT, cwd TEXT, model_provider TEXT, source_updated_at REAL);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, 'Original Title', '/old', 'openai', 0, 100)",
                rusqlite::params![
                    session_id,
                    root.join("sessions")
                        .join("missing.jsonl")
                        .to_string_lossy()
                        .to_string()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO local_thread_catalog VALUES (?1, 'Original Title', '/old', 'openai', 1.0)",
                [session_id],
            )
            .unwrap();
        drop(connection);

        let history_db = root.join("thread_history_no_rollout.sqlite");
        let connection = Connection::open(&history_db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE thread_items (thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, item_id TEXT NOT NULL, item_type TEXT NOT NULL, item_json TEXT NOT NULL);\n\
                 CREATE TABLE thread_turns (thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, first_user_item_id TEXT, final_agent_item_id TEXT);\n\
                 CREATE TABLE thread_history_projection_state (thread_id TEXT PRIMARY KEY, next_rollout_byte_offset INTEGER NOT NULL, next_rollout_ordinal INTEGER NOT NULL);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO thread_history_projection_state VALUES (?1, 100, 9)",
                [session_id],
            )
            .unwrap();
        drop(connection);

        let detail = get_codex_session_context_from_home(&root, session_id).unwrap();
        let mut request = context_save_request(&detail);
        request.title = Some("Renamed Title".to_string());
        save_codex_session_context_from_home(&root, request).unwrap();

        let connection = Connection::open(&history_db).unwrap();
        let cursor: (i64, i64) = connection
            .query_row(
                "SELECT next_rollout_byte_offset, next_rollout_ordinal FROM thread_history_projection_state WHERE thread_id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        drop(connection);
        assert_eq!(cursor, (100, 9));

        let connection = Connection::open(&state_db).unwrap();
        let title: String = connection
            .query_row(
                "SELECT title FROM threads WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);
        assert_eq!(title, "Renamed Title");

        fs::remove_dir_all(root).unwrap();
    }

    /// P1/P2-4: newly added messages must keep the turn summary ids in sync.
    #[test]
    fn added_messages_update_the_turn_summary_ids() {
        let root = test_root("context-added-turn-summary");
        let session_id = "019f1234-abcd-7000-8000-0000000000f3";
        let fixture = seed_context_edit_fixture(&root, session_id);
        let connection = Connection::open(&fixture.history_db).unwrap();
        connection
            .execute(
                "INSERT INTO thread_turns (thread_id, turn_id, rollout_ordinal, status) VALUES (?1, 'turn-fixture', 1, 'completed')",
                [session_id],
            )
            .unwrap();
        drop(connection);

        let context = get_codex_session_context_from_home(&root, session_id).unwrap();
        let mut request = context_save_request(&context);
        request.new_messages = Some(vec![
            CodexSessionNewMessage {
                role: "user".to_string(),
                content: "Follow-up question".to_string(),
            },
            CodexSessionNewMessage {
                role: "assistant".to_string(),
                content: "Follow-up answer".to_string(),
            },
        ]);
        save_codex_session_context_from_home(&root, request).unwrap();

        let connection = Connection::open(&fixture.history_db).unwrap();
        let summary: (Option<String>, Option<String>) = connection
            .query_row(
                "SELECT first_user_item_id, final_agent_item_id FROM thread_turns WHERE thread_id = ?1 AND turn_id = 'turn-fixture'",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        drop(connection);
        let first_user_item_id = summary.0.expect("first user item id");
        let final_agent_item_id = summary.1.expect("final agent item id");

        let user_json = thread_item_json(&fixture.history_db, session_id, &first_user_item_id);
        assert!(user_json.contains("Follow-up question"), "{user_json}");
        let agent_json = thread_item_json(&fixture.history_db, session_id, &final_agent_item_id);
        assert!(agent_json.contains("Follow-up answer"), "{agent_json}");

        fs::remove_dir_all(root).unwrap();
    }

    /// P2-5: several assistant messages in one turn must resolve the turn's
    /// `last_agent_message` from the message order, not from hash iteration.
    #[test]
    fn same_turn_assistant_edits_resolve_last_agent_message_deterministically() {
        let root = test_root("context-last-agent-deterministic");
        let session_id = "019f1234-abcd-7000-8000-0000000000f4";
        let turn_id = "turn-double-answer";
        let sessions_dir = root.join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let rollout = sessions_dir.join(format!("rollout-{session_id}.jsonl"));
        let lines = [
            json!({"ordinal": 0, "type": "session_meta", "payload": {"id": session_id, "title": "Two answers"}}).to_string(),
            json!({"ordinal": 1, "type": "event_msg", "payload": {"type": "task_started", "turn_id": turn_id}}).to_string(),
            json!({"ordinal": 2, "type": "response_item", "metadata": {"retained_source": {"id": {"message_id": "item-answer-1", "role": "assistant", "turn_id": turn_id}}}, "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "First answer"}]}}).to_string(),
            json!({"ordinal": 3, "type": "event_msg", "payload": {"type": "item_completed", "thread_id": session_id, "turn_id": turn_id, "item": {"id": "item-answer-1", "type": "AgentMessage", "content": [{"type": "Text", "text": "First answer"}]}}}).to_string(),
            json!({"ordinal": 4, "type": "response_item", "metadata": {"retained_source": {"id": {"message_id": "item-answer-2", "role": "assistant", "turn_id": turn_id}}}, "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Second answer"}]}}).to_string(),
            json!({"ordinal": 5, "type": "event_msg", "payload": {"type": "item_completed", "thread_id": session_id, "turn_id": turn_id, "item": {"id": "item-answer-2", "type": "AgentMessage", "content": [{"type": "Text", "text": "Second answer"}]}}}).to_string(),
            json!({"ordinal": 6, "type": "event_msg", "payload": {"type": "task_complete", "turn_id": turn_id, "last_agent_message": "Second answer"}}).to_string(),
        ];
        fs::write(&rollout, format!("{}\n", lines.join("\n"))).unwrap();

        let context = get_codex_session_context_from_home(&root, session_id).unwrap();
        let assistant_lines = context
            .messages
            .iter()
            .filter(|message| message.role == "assistant")
            .map(|message| message.line_number)
            .collect::<Vec<_>>();
        assert_eq!(assistant_lines.len(), 2, "{assistant_lines:?}");

        let mut request = context_save_request(&context);
        request.message_updates = Some(vec![
            CodexSessionMessageUpdate {
                line_number: assistant_lines[0],
                role: None,
                content: Some("First edited".to_string()),
                deleted: None,
            },
            CodexSessionMessageUpdate {
                line_number: assistant_lines[1],
                role: None,
                content: Some("Second edited".to_string()),
                deleted: None,
            },
        ]);
        save_codex_session_context_from_home(&root, request).unwrap();

        let saved = fs::read_to_string(&rollout).unwrap();
        assert!(
            saved.contains("\"last_agent_message\":\"Second edited\""),
            "{saved}"
        );
        assert!(
            !saved.contains("\"last_agent_message\":\"First edited\""),
            "{saved}"
        );
        assert_eq!(saved.matches("Second edited").count(), 3, "{saved}");

        fs::remove_dir_all(root).unwrap();
    }

    /// A corrupt history database must abort before any durable edit.
    #[test]
    fn history_inspection_failure_aborts_before_writing() {
        let root = test_root("context-history-backup-failure");
        let session_id = "019f1234-abcd-7000-8000-0000000000f7";
        let fixture = seed_context_edit_fixture(&root, session_id);
        let before = fs::read_to_string(&fixture.rollout).unwrap();

        fs::write(
            root.join("thread_history_broken.sqlite"),
            b"not a sqlite database",
        )
        .unwrap();

        let context = get_codex_session_context_from_home(&root, session_id).unwrap();
        let mut request = context_save_request(&context);
        request.title = Some("Renamed Title".to_string());
        request.message_updates = Some(vec![CodexSessionMessageUpdate {
            line_number: assistant_line_number(&context),
            role: None,
            content: Some("Answer edited".to_string()),
            deleted: None,
        }]);

        let error = save_codex_session_context_from_home(&root, request).unwrap_err();
        assert!(
            error.contains("Failed to inspect thread history projection"),
            "{error}"
        );

        assert_eq!(fs::read_to_string(&fixture.rollout).unwrap(), before);
        let agent_json = thread_item_json(&fixture.history_db, session_id, fixture.agent_item_id);
        assert!(agent_json.contains("\"text\":\"Answer\""), "{agent_json}");

        let connection = Connection::open(&fixture.state_db).unwrap();
        let title: String = connection
            .query_row(
                "SELECT title FROM threads WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);
        assert_eq!(title, "Original Title");

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(test)]
    mod projection_regressions {
        use super::*;
        fn req() -> SaveCodexSessionContextRequest {
            SaveCodexSessionContextRequest {
                session_id: "s".into(),
                expected_rollout_sha256: None,
                title: None,
                cwd: None,
                model_provider: None,
                model: None,
                archived: None,
                message_updates: None,
                new_messages: None,
            }
        }
        fn meta() -> Value {
            json!({"ordinal":0,"type":"session_meta","payload":{"id":"s"}})
        }
        fn response(turn: &str, id: &str, text: &str, ordinal: i64) -> Value {
            json!({"ordinal":ordinal,"type":"response_item","metadata":{"retained_source":{"id":{"message_id":id,"turn_id":turn}}},"payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}})
        }
        fn event(turn: &str, id: &str, text: &str, ordinal: i64) -> Value {
            json!({"ordinal":ordinal,"type":"event_msg","payload":{"type":"item_completed","thread_id":"s","turn_id":turn,"item":{"id":id,"type":"AgentMessage","content":[{"type":"Text","text":text}]}}})
        }
        fn start(turn: &str, ordinal: i64) -> Value {
            json!({"ordinal":ordinal,"type":"event_msg","payload":{"type":"task_started","turn_id":turn}})
        }
        fn complete(turn: &str, text: &str, ordinal: i64) -> Value {
            json!({"ordinal":ordinal,"type":"event_msg","payload":{"type":"task_complete","turn_id":turn,"last_agent_message":text}})
        }

        #[test]
        fn appending_messages_preserves_turn_terminal_and_timing() {
            for explicit in [false, true] {
                for line_ending in ["\n", "\r\n"] {
                    for terminal in [None, Some("task_complete"), Some("turn_aborted")] {
                        for started in [false, true] {
                            let mut original = vec![
                                meta(),
                                start("earlier", 10),
                                complete("earlier", "earlier", 20),
                            ];
                            if started {
                                original.push(start("t1", 30));
                            }
                            original.extend([
                                response("t1", "old", "old", 40),
                                event("t1", "old", "old", 50),
                            ]);
                            if let Some(kind) = terminal {
                                original.push(json!({"ordinal":60,"timestamp":"original-time","type":"event_msg","payload":{"type":kind,"turn_id":"t1","last_agent_message":"old","completed_at":123,"duration_ms":456,"reason":"interrupted"}}));
                            }
                            original.push(json!({"ordinal":70,"type":"event_msg","payload":{"type":"token_count","info":{"total_tokens":99}}}));
                            if !explicit {
                                for record in &mut original {
                                    record.as_object_mut().unwrap().remove("ordinal");
                                }
                            }
                            let old_terminal = original
                                .iter()
                                .find(|record| {
                                    record["payload"]["type"] == terminal.unwrap_or("absent")
                                        && record["payload"]["turn_id"] == "t1"
                                })
                                .cloned();
                            let mut request = req();
                            request.new_messages = Some(
                                [
                                    ("user", "question"),
                                    ("assistant", "first added"),
                                    ("assistant", "last added"),
                                ]
                                .into_iter()
                                .map(|(role, content)| CodexSessionNewMessage {
                                    role: role.into(),
                                    content: content.into(),
                                })
                                .collect(),
                            );
                            let (saved, plan) = build_updated_rollout(
                                &serialize_jsonl(&original, line_ending).unwrap(),
                                "s",
                                &request,
                            )
                            .unwrap();
                            let records =
                                validate_raw_jsonl(std::str::from_utf8(&saved).unwrap(), "s")
                                    .unwrap()
                                    .into_iter()
                                    .map(|(_, record)| record)
                                    .collect::<Vec<_>>();
                            let terminals = records
                                .iter()
                                .enumerate()
                                .filter(|(_, record)| {
                                    record["payload"]["turn_id"] == "t1"
                                        && matches!(
                                            record["payload"]["type"].as_str(),
                                            Some("task_complete" | "turn_aborted")
                                        )
                                })
                                .collect::<Vec<_>>();
                            assert_eq!(terminals.len(), usize::from(terminal.is_some()));
                            assert_eq!(
                                records[2], original[2],
                                "Earlier turns must stay unchanged"
                            );
                            if let Some(mut expected) = old_terminal {
                                if explicit {
                                    expected["ordinal"] = json!(66);
                                }
                                if terminal == Some("task_complete") {
                                    expected["payload"]["last_agent_message"] = json!("last added");
                                }
                                assert_eq!(*terminals[0].1, expected);
                                for item in &plan.new_items {
                                    let index = records
                                        .iter()
                                        .position(|record| {
                                            record["payload"]["item"]["id"] == item.item_id
                                        })
                                        .unwrap();
                                    assert!(index < terminals[0].0);
                                }
                            }
                            let ordinals = records
                                .iter()
                                .enumerate()
                                .map(|(index, record)| {
                                    record["ordinal"].as_i64().unwrap_or(index as i64)
                                })
                                .collect::<Vec<_>>();
                            assert!(ordinals.windows(2).all(|pair| pair[0] < pair[1]));
                            assert_eq!(
                                plan.projection_cursor.unwrap().next_ordinal,
                                ordinals.last().unwrap() + 1
                            );
                            let position = &plan.turn_positions["t1"];
                            assert_eq!(position.end_ordinal.is_some(), terminal.is_some());
                            if let Some((index, record)) = terminals.first() {
                                assert_eq!(
                                    position.end_ordinal,
                                    Some(record["ordinal"].as_i64().unwrap_or(*index as i64))
                                );
                                let end_offset = saved
                                    .split_inclusive(|byte| *byte == b'\n')
                                    .take(index + 1)
                                    .map(|line| line.len() as i64)
                                    .sum::<i64>();
                                assert_eq!(position.end_byte_offset, Some(end_offset));
                            }
                        }
                    }
                }
            }
        }

        #[test]
        fn tool_deletion_syncs_canonical_events_and_sqlite_without_task_started() {
            for started in [false, true] {
                for output_selected in [false, true] {
                    for tool_type in [
                        "CommandExecution",
                        "FileChange",
                        "DynamicToolCall",
                        "McpToolCall",
                    ] {
                        let mut original = vec![meta()];
                        if started {
                            original.push(start("t1", 1));
                        }
                        let selected = original.len() + usize::from(output_selected);
                        original.extend([
                            json!({"type":"response_item","payload":{"type":"function_call","name":"fixture_tool","arguments":"{}","call_id":"call-1"}}),
                            json!({"type":"response_item","payload":{"type":"function_call_output","output":"output","call_id":"call-1"}}),
                            json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"t1","item":{"type":tool_type,"id":"call-1"}}}),
                            json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"t1","item":{"type":tool_type,"id":"keep"}}}),
                        ]);
                        let mut request = req();
                        request.message_updates = Some(vec![CodexSessionMessageUpdate {
                            line_number: selected,
                            role: None,
                            content: None,
                            deleted: Some(true),
                        }]);
                        let (saved, plan) = build_updated_rollout(
                            &serialize_jsonl(&original, "\n").unwrap(),
                            "s",
                            &request,
                        )
                        .unwrap();
                        assert!(!String::from_utf8(saved).unwrap().contains("call-1"));
                        let path = db("tool-deletion-projection");
                        let connection = Connection::open(&path).unwrap();
                        for (thread, turn, id) in [
                            ("s", "t1", "call-1"),
                            ("s", "t1", "keep"),
                            ("s", "other-turn", "call-1"),
                            ("other-thread", "t1", "call-1"),
                        ] {
                            connection.execute("INSERT INTO thread_items VALUES (?1,?2,?3,1,1,'{}','commandExecution',0,1,1)", rusqlite::params![thread,turn,id]).unwrap();
                        }
                        drop(connection);
                        sync_thread_history_projection_databases(
                            std::slice::from_ref(&path),
                            "s",
                            &plan,
                        )
                        .unwrap();
                        let connection = Connection::open(&path).unwrap();
                        let count: i64 = connection
                            .query_row("SELECT COUNT(*) FROM thread_items", [], |row| row.get(0))
                            .unwrap();
                        assert_eq!(count, 3);
                        let deleted:i64 = connection.query_row("SELECT COUNT(*) FROM thread_items WHERE thread_id='s' AND turn_id='t1' AND item_id='call-1'", [], |row| row.get(0)).unwrap();
                        assert_eq!(deleted, 0);
                        drop(connection);
                        fs::remove_dir_all(path.parent().unwrap()).unwrap();
                    }
                }
            }
        }
        #[test]
        fn structured_partial_edit_must_keep_final_summary() {
            let original = vec![
                meta(),
                start("t1", 1),
                response("t1", "a1", "A", 2),
                event("t1", "a1", "A", 3),
                response("t1", "a2", "B", 4),
                event("t1", "a2", "B", 5),
                complete("t1", "B", 6),
            ];
            let mut request = req();
            request.message_updates = Some(vec![CodexSessionMessageUpdate {
                line_number: 2,
                role: None,
                content: Some("A edited".into()),
                deleted: None,
            }]);
            let (saved, _) =
                build_updated_rollout(&serialize_jsonl(&original, "\n").unwrap(), "s", &request)
                    .unwrap();
            let next = validate_raw_jsonl(std::str::from_utf8(&saved).unwrap(), "s").unwrap();
            assert_eq!(next[6].1["payload"]["last_agent_message"], json!("B"));
        }
        fn db(test_name: &str) -> PathBuf {
            let root = super::test_root(test_name);
            std::fs::create_dir_all(&root).unwrap();
            let path = root.join("history.sqlite");
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE thread_items (thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, item_id TEXT NOT NULL, rollout_ordinal INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, item_json TEXT NOT NULL, item_type TEXT NOT NULL, updated_at_ordinal INTEGER NOT NULL, started_at_ms INTEGER, completed_at_ms INTEGER, PRIMARY KEY(thread_id,turn_id,item_id)); CREATE TABLE thread_turns (thread_id TEXT,turn_id TEXT,first_user_item_id TEXT,final_agent_item_id TEXT); ").unwrap();
            path
        }
        #[test]
        fn new_item_db_ordinal_must_match_rollout() {
            let original = vec![meta(), start("t1", 1), complete("t1", "", 2)];
            let mut request = req();
            request.new_messages = Some(vec![CodexSessionNewMessage {
                role: "user".into(),
                content: "new".into(),
            }]);
            let (saved, plan) =
                build_updated_rollout(&serialize_jsonl(&original, "\n").unwrap(), "s", &request)
                    .unwrap();
            let records = validate_raw_jsonl(std::str::from_utf8(&saved).unwrap(), "s").unwrap();
            let expected = records
                .iter()
                .find(|(_, r)| r["payload"]["type"] == "item_completed")
                .unwrap()
                .1["ordinal"]
                .as_i64()
                .unwrap();
            let path = db("ordinal");
            sync_thread_history_projection_databases(std::slice::from_ref(&path), "s", &plan)
                .unwrap();
            let c = Connection::open(&path).unwrap();
            let actual: i64 = c
                .query_row("SELECT rollout_ordinal FROM thread_items", [], |r| r.get(0))
                .unwrap();
            drop(c);
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
            assert_eq!(
                actual, expected,
                "SQLite ordinal must refer to canonical item_completed"
            );
        }

        #[test]
        fn duplicate_item_ids_are_scoped_to_their_turn() {
            {
                for deleted in [false, true] {
                    let session_id = dynamic_session_id();
                    let item_id = format!("item-{session_id}");
                    let mut original = vec![
                        meta(),
                        start("t1", 1),
                        response("t1", &item_id, "first", 2),
                        event("t1", &item_id, "first", 3),
                        complete("t1", "first", 4),
                        start("t2", 5),
                        response("t2", &item_id, "second", 6),
                        event("t2", &item_id, "second", 7),
                        complete("t2", "second", 8),
                    ];
                    original[0]["payload"]["id"] = json!(session_id);
                    for record in &mut original {
                        if record["payload"].get("thread_id").is_some() {
                            record["payload"]["thread_id"] = json!(session_id);
                        }
                    }
                    let mut request = req();
                    request.session_id = session_id.clone();
                    request.message_updates = Some(vec![CodexSessionMessageUpdate {
                        line_number: 6,
                        role: None,
                        content: Some("edited second".into()),
                        deleted: Some(deleted),
                    }]);
                    let (saved, plan) = build_updated_rollout(
                        &serialize_jsonl(&original, "\n").unwrap(),
                        &session_id,
                        &request,
                    )
                    .unwrap();
                    let path = db("duplicate-item-id");
                    let connection = Connection::open(&path).unwrap();
                    for (turn, text, ordinal) in [("t1", "first", 3), ("t2", "second", 7)] {
                        connection.execute(
                            "INSERT INTO thread_items VALUES (?1, ?2, ?3, ?4, 1, ?5, 'agentMessage', 0, 1, 1)",
                            rusqlite::params![session_id, turn, item_id, ordinal, json!({"type":"agentMessage","id":item_id,"text":text}).to_string()],
                        ).unwrap();
                        connection
                            .execute(
                                "INSERT INTO thread_turns VALUES (?1,?2,NULL,?3)",
                                rusqlite::params![session_id, turn, item_id],
                            )
                            .unwrap();
                    }
                    drop(connection);
                    sync_thread_history_projection_databases(
                        std::slice::from_ref(&path),
                        &session_id,
                        &plan,
                    )
                    .unwrap();
                    assert!(
                        thread_item_json_for_turn(&path, &session_id, "t1", &item_id)
                            .contains("first")
                    );
                    let connection = Connection::open(&path).unwrap();
                    let first_summary: String = connection
                        .query_row(
                            "SELECT final_agent_item_id FROM thread_turns WHERE turn_id = 't1'",
                            [],
                            |row| row.get(0),
                        )
                        .unwrap();
                    assert_eq!(first_summary, item_id);
                    if deleted {
                        let remaining: i64 = connection
                            .query_row(
                                "SELECT COUNT(*) FROM thread_items WHERE turn_id = 't2'",
                                [],
                                |row| row.get(0),
                            )
                            .unwrap();
                        assert_eq!(remaining, 0);
                    } else {
                        assert!(
                            thread_item_json_for_turn(&path, &session_id, "t2", &item_id)
                                .contains("edited second")
                        );
                        assert!(String::from_utf8(saved).unwrap().contains("edited second"));
                    }
                    drop(connection);
                    fs::remove_dir_all(path.parent().unwrap()).unwrap();
                }
            }
        }

        #[test]
        fn rewritten_rollouts_keep_all_projection_positions_in_sync() {
            {
                for explicit_ordinals in [false, true] {
                    for line_ending in ["\n", "\r\n"] {
                        for action in ["metadata", "edit", "delete", "append"] {
                            let session_id = dynamic_session_id();
                            let first_id = format!("first-{session_id}");
                            let second_id = format!("second-{session_id}");
                            let tool_id = format!("tool-{session_id}");
                            let mut original = vec![
                                meta(),
                                start("t1", 10),
                                response("t1", &first_id, "first", 20),
                                event("t1", &first_id, "first", 30),
                                complete("t1", "first", 40),
                                start("t2", 50),
                                response("t2", &second_id, "second", 60),
                                event("t2", &second_id, "second", 70),
                                complete("t2", "second", 80),
                            ];
                            original.insert(8, json!({"ordinal":75,"type":"event_msg","payload":{"type":"item_completed","turn_id":"t2","item":{"id":tool_id,"type":"ShellToolCall","status":"completed"}}}));
                            original[0]["payload"]["title"] = json!("old");
                            if !explicit_ordinals {
                                for record in &mut original {
                                    record.as_object_mut().unwrap().remove("ordinal");
                                }
                            }
                            original[0]["payload"]["id"] = json!(session_id);
                            for record in &mut original {
                                if record["payload"].get("thread_id").is_some() {
                                    record["payload"]["thread_id"] = json!(session_id);
                                }
                            }
                            let mut request = req();
                            request.session_id = session_id.clone();
                            match action {
                                "metadata" => {
                                    request.title = Some("元数据🙂: a longer title".into())
                                }
                                "edit" | "delete" => {
                                    request.message_updates =
                                        Some(vec![CodexSessionMessageUpdate {
                                            line_number: 2,
                                            role: None,
                                            content: Some("edited🙂 longer message".into()),
                                            deleted: Some(action == "delete"),
                                        }])
                                }
                                "append" => {
                                    request.new_messages = Some(vec![CodexSessionNewMessage {
                                        role: "assistant".into(),
                                        content: "added".into(),
                                    }])
                                }
                                _ => unreachable!(),
                            }
                            let (saved, plan) = build_updated_rollout(
                                &serialize_jsonl(&original, line_ending).unwrap(),
                                &session_id,
                                &request,
                            )
                            .unwrap();
                            let path = db("projection-positions");
                            let connection = Connection::open(&path).unwrap();
                            connection.execute_batch("ALTER TABLE thread_turns ADD COLUMN rollout_ordinal INTEGER; ALTER TABLE thread_turns ADD COLUMN rollout_byte_offset INTEGER; ALTER TABLE thread_turns ADD COLUMN rollout_end_ordinal INTEGER; ALTER TABLE thread_turns ADD COLUMN rollout_end_byte_offset INTEGER; CREATE TABLE thread_history_projection_state (thread_id TEXT PRIMARY KEY, next_rollout_byte_offset INTEGER, next_rollout_ordinal INTEGER);").unwrap();
                            connection.execute("INSERT INTO thread_history_projection_state VALUES (?1, ?2, ?3)", rusqlite::params![session_id, plan.original_projection_cursor.unwrap().byte_offset, plan.original_projection_cursor.unwrap().next_ordinal]).unwrap();
                            for (turn, id, ordinal) in
                                [("t1", &first_id, 30), ("t2", &second_id, 70)]
                            {
                                connection.execute("INSERT INTO thread_items VALUES (?1,?2,?3,?4,1,?5,'agentMessage',0,1,1)", rusqlite::params![session_id, turn, id, ordinal, json!({"type":"agentMessage","text":id}).to_string()]).unwrap();
                                connection.execute("INSERT INTO thread_turns (thread_id,turn_id,rollout_ordinal,rollout_byte_offset,rollout_end_ordinal,rollout_end_byte_offset) VALUES (?1,?2,999,999,999,999)", rusqlite::params![session_id, turn]).unwrap();
                            }
                            connection.execute("INSERT INTO thread_items VALUES (?1,'t2',?2,75,1,?3,'commandExecution',0,1,1)", rusqlite::params![session_id,tool_id,json!({"id":tool_id,"type":"commandExecution"}).to_string()]).unwrap();
                            drop(connection);
                            sync_thread_history_projection_databases(
                                std::slice::from_ref(&path),
                                &session_id,
                                &plan,
                            )
                            .unwrap();
                            let connection = Connection::open(&path).unwrap();
                            let mut last_completions = HashMap::new();
                            for (index, line) in
                                saved.split_inclusive(|byte| *byte == b'\n').enumerate()
                            {
                                let record: Value = serde_json::from_slice(line).unwrap();
                                if record["payload"]["type"] == "task_complete" {
                                    last_completions.insert(
                                        record["payload"]["turn_id"].as_str().unwrap().to_string(),
                                        index,
                                    );
                                }
                            }
                            let mut offset = 0;
                            let mut next_ordinal = 0;
                            let mut completed_items = 0;
                            for (index, line) in
                                saved.split_inclusive(|byte| *byte == b'\n').enumerate()
                            {
                                let record: Value = serde_json::from_slice(line).unwrap();
                                let ordinal = record
                                    .get("ordinal")
                                    .and_then(Value::as_i64)
                                    .unwrap_or(index as i64);
                                next_ordinal = next_ordinal.max(ordinal + 1);
                                let event_type =
                                    record["payload"]["type"].as_str().unwrap_or_default();
                                let turn =
                                    record["payload"]["turn_id"].as_str().unwrap_or_default();
                                if event_type == "task_started" {
                                    let position: (i64,i64) = connection.query_row("SELECT rollout_ordinal, rollout_byte_offset FROM thread_turns WHERE turn_id = ?1", [turn], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
                                    assert_eq!(
                                        position,
                                        (ordinal, offset),
                                        "explicit={explicit_ordinals}, action={action}"
                                    );
                                } else if event_type == "task_complete"
                                    && last_completions.get(turn) == Some(&index)
                                {
                                    let position: (i64,i64) = connection.query_row("SELECT rollout_end_ordinal, rollout_end_byte_offset FROM thread_turns WHERE turn_id = ?1", [turn], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
                                    assert_eq!(position, (ordinal, offset + line.len() as i64));
                                } else if event_type == "item_completed" {
                                    let item_id = record["payload"]["item"]["id"].as_str().unwrap();
                                    let actual: i64 = connection.query_row("SELECT rollout_ordinal FROM thread_items WHERE thread_id = ?1 AND turn_id = ?2 AND item_id = ?3", rusqlite::params![session_id, turn, item_id], |row| row.get(0)).unwrap();
                                    assert_eq!(actual, ordinal);
                                    completed_items += 1;
                                }
                                offset += line.len() as i64;
                            }
                            let cursor: (i64,i64) = connection.query_row("SELECT next_rollout_byte_offset, next_rollout_ordinal FROM thread_history_projection_state", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
                            assert_eq!(cursor, (saved.len() as i64, next_ordinal));
                            let count: i64 = connection
                                .query_row("SELECT COUNT(*) FROM thread_items", [], |row| {
                                    row.get(0)
                                })
                                .unwrap();
                            assert_eq!(count, completed_items);
                            drop(connection);
                            fs::remove_dir_all(path.parent().unwrap()).unwrap();
                        }
                    }
                }
            }
        }

        #[test]
        fn open_and_interrupted_turn_positions_are_synchronized() {
            for terminal in [None, Some("turn_aborted")] {
                let session_id = dynamic_session_id();
                let mut records = vec![
                    json!({"type":"session_meta","payload":{"id":session_id}}),
                    start("t1", 10),
                ];
                if let Some(event_type) = terminal {
                    records.push(json!({"ordinal":20,"type":"event_msg","payload":{"type":event_type,"turn_id":"t1"}}));
                }
                let mut request = req();
                request.title = Some("a longer title🙂".into());
                let (saved, plan) = build_updated_rollout(
                    &serialize_jsonl(&records, "\r\n").unwrap(),
                    &session_id,
                    &request,
                )
                .unwrap();
                let path = db("turn-terminal-positions");
                let connection = Connection::open(&path).unwrap();
                connection.execute_batch("ALTER TABLE thread_turns ADD COLUMN rollout_byte_offset INTEGER; ALTER TABLE thread_turns ADD COLUMN rollout_end_ordinal INTEGER; ALTER TABLE thread_turns ADD COLUMN rollout_end_byte_offset INTEGER;").unwrap();
                connection.execute("INSERT INTO thread_turns (thread_id,turn_id,rollout_byte_offset,rollout_end_ordinal,rollout_end_byte_offset) VALUES (?1,'t1',999,999,999)", [&session_id]).unwrap();
                drop(connection);
                sync_thread_history_projection_databases(
                    std::slice::from_ref(&path),
                    &session_id,
                    &plan,
                )
                .unwrap();
                let connection = Connection::open(&path).unwrap();
                let position: (i64,Option<i64>,Option<i64>) = connection.query_row("SELECT rollout_byte_offset,rollout_end_ordinal,rollout_end_byte_offset FROM thread_turns", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
                let first_line_len = saved
                    .split_inclusive(|byte| *byte == b'\n')
                    .next()
                    .unwrap()
                    .len() as i64;
                assert_eq!(position.0, first_line_len);
                assert_eq!(position.1, terminal.map(|_| 20));
                assert_eq!(position.2, terminal.map(|_| saved.len() as i64));
                drop(connection);
                fs::remove_dir_all(path.parent().unwrap()).unwrap();
            }
        }

        #[test]
        fn sqlite_user_edit_preserves_non_text_content_parts() {
            let image = json!({"type":"image","url":"fixture-image"});
            let audio = json!({"type":"audio","data":"fixture-audio"});
            for content in [
                json!([{"type":"text","text":"old"},image.clone(),audio.clone()]),
                json!([image.clone(),{"type":"text","text":"first"},{"type":"text","text":"second"},audio.clone()]),
                json!([image.clone(), audio.clone()]),
            ] {
                {
                    let session_id = dynamic_session_id();
                    let path = db("user-mixed-content");
                    let connection = Connection::open(&path).unwrap();
                    connection.execute("INSERT INTO thread_items VALUES (?1,'t1','user-1',2,1,?2,'userMessage',0,1,1)", rusqlite::params![session_id, json!({"type":"userMessage","id":"user-1","clientId":"keep","content":content}).to_string()]).unwrap();
                    drop(connection);
                    let plan = RolloutSyncPlan {
                        updated_items: vec![RolloutItemSync {
                            item_id: "user-1".into(),
                            turn_id: "t1".into(),
                            item_type: "userMessage".into(),
                            text: "new text🙂".into(),
                        }],
                        ..Default::default()
                    };
                    sync_thread_history_projection_databases(
                        std::slice::from_ref(&path),
                        &session_id,
                        &plan,
                    )
                    .unwrap();
                    let item: Value = serde_json::from_str(&thread_item_json_for_turn(
                        &path,
                        &session_id,
                        "t1",
                        "user-1",
                    ))
                    .unwrap();
                    assert_eq!(extract_content_text(&item["content"]), "new text🙂");
                    let non_text = item["content"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|part| part.get("text").is_none())
                        .cloned()
                        .collect::<Vec<_>>();
                    assert_eq!(non_text, vec![image.clone(), audio.clone()]);
                    assert_eq!(item["clientId"], "keep");
                    let text_part = item["content"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|part| part.get("text").is_some())
                        .unwrap();
                    assert_eq!(text_part["type"], "text");
                    fs::remove_dir_all(path.parent().unwrap()).unwrap();
                }
            }
        }

        #[test]
        fn partial_assistant_edits_and_deletions_recompute_retained_summaries() {
            let original = vec![
                meta(),
                start("t1", 1),
                response("t1", "a1", "A", 2),
                event("t1", "a1", "A", 3),
                response("t1", "a2", "B", 4),
                event("t1", "a2", "B", 5),
                complete("t1", "B", 6),
                start("t2", 7),
                response("t2", "a3", "C", 8),
                event("t2", "a3", "C", 9),
                complete("t2", "C", 10),
            ];
            for (lines, deleted, expected) in [
                (vec![2], false, json!("B")),
                (vec![4], false, json!("Edited")),
                (vec![2], true, json!("B")),
                (vec![4], true, json!("A")),
                (vec![2, 4], true, Value::Null),
            ] {
                {
                    let mut request = req();
                    request.message_updates = Some(
                        lines
                            .iter()
                            .map(|line| CodexSessionMessageUpdate {
                                line_number: *line,
                                role: None,
                                content: Some("Edited".into()),
                                deleted: Some(deleted),
                            })
                            .collect(),
                    );
                    let (saved, _) = build_updated_rollout(
                        &serialize_jsonl(&original, "\n").unwrap(),
                        "s",
                        &request,
                    )
                    .unwrap();
                    let records =
                        validate_raw_jsonl(std::str::from_utf8(&saved).unwrap(), "s").unwrap();
                    for (turn, text) in [("t1", expected.clone()), ("t2", json!("C"))] {
                        let completion = records
                            .iter()
                            .find(|(_, record)| {
                                record["payload"]["type"] == "task_complete"
                                    && record["payload"]["turn_id"] == turn
                            })
                            .unwrap();
                        assert_eq!(
                            completion.1["payload"]["last_agent_message"], text,
                            "deleted={deleted}, lines={lines:?}"
                        );
                    }
                }
            }
        }

        #[test]
        fn appended_item_ordinals_use_canonical_events_after_stamping() {
            for explicit_ordinals in [false, true] {
                {
                    let mut original = vec![meta(), start("t1", 1)];
                    if explicit_ordinals {
                        original[1]["ordinal"] = json!(40);
                    } else {
                        for record in &mut original {
                            record.as_object_mut().unwrap().remove("ordinal");
                        }
                    }
                    let mut request = req();
                    request.new_messages = Some(vec![
                        CodexSessionNewMessage {
                            role: "user".into(),
                            content: "new".into(),
                        },
                        CodexSessionNewMessage {
                            role: "assistant".into(),
                            content: "answer".into(),
                        },
                    ]);
                    let (saved, plan) = build_updated_rollout(
                        &serialize_jsonl(&original, "\n").unwrap(),
                        "s",
                        &request,
                    )
                    .unwrap();
                    let records =
                        validate_raw_jsonl(std::str::from_utf8(&saved).unwrap(), "s").unwrap();
                    let path = db("canonical-ordinals");
                    sync_thread_history_projection_databases(
                        std::slice::from_ref(&path),
                        "s",
                        &plan,
                    )
                    .unwrap();
                    let connection = Connection::open(&path).unwrap();
                    for item in &plan.new_items {
                        let (position, record) = records
                            .iter()
                            .find(|(_, record)| record["payload"]["item"]["id"] == item.item_id)
                            .unwrap();
                        let expected = record
                            .get("ordinal")
                            .and_then(Value::as_i64)
                            .unwrap_or(*position as i64);
                        let actual: i64 = connection
                            .query_row(
                                "SELECT rollout_ordinal FROM thread_items WHERE item_id = ?1",
                                [&item.item_id],
                                |row| row.get(0),
                            )
                            .unwrap();
                        assert_eq!(actual, expected, "explicit={explicit_ordinals}");
                        assert!(expected < plan.projection_cursor.unwrap().next_ordinal);
                    }
                    drop(connection);
                    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
                }
            }
        }
        #[test]
        fn deleted_summary_items_follow_official_phase_and_status_rules() {
            for status in ["completed", "interrupted", "failed", "inProgress", "legacy"] {
                for phase in [
                    None,
                    Some("final_answer"),
                    Some("commentary"),
                    Some("absent"),
                ] {
                    for earlier_final in [false, true] {
                        let path = db("summary-selection");
                        let c = Connection::open(&path).unwrap();
                        if status != "legacy" {
                            c.execute_batch("ALTER TABLE thread_turns ADD COLUMN status TEXT")
                                .unwrap();
                        }
                        c.execute("INSERT INTO thread_turns (thread_id,turn_id,first_user_item_id,final_agent_item_id) VALUES ('s','t1','gone-user','gone-agent'),('s','other','sentinel','sentinel'),('other','t1','sentinel','sentinel')", []).unwrap();
                        if status != "legacy" {
                            c.execute("UPDATE thread_turns SET status=?1", [status])
                                .unwrap();
                        }
                        let mut items = vec![
                            ("gone-user", 0, "userMessage", None),
                            ("user-first", 10, "userMessage", None),
                            ("user-second", 20, "userMessage", None),
                            ("gone-agent", 50, "agentMessage", Some("final_answer")),
                        ];
                        if earlier_final {
                            items.push(("earlier-final", 30, "agentMessage", Some("final_answer")));
                        }
                        if phase != Some("absent") {
                            items.push(("retained-agent", 40, "agentMessage", phase));
                        }
                        for (id, ordinal, kind, phase) in items {
                            c.execute(
                                "INSERT INTO thread_items VALUES ('s','t1',?1,?2,1,?3,?4,0,1,1)",
                                rusqlite::params![
                                    id,
                                    ordinal,
                                    json!({"id":id,"type":kind,"phase":phase}).to_string(),
                                    kind
                                ],
                            )
                            .unwrap();
                        }
                        drop(c);
                        let plan = RolloutSyncPlan {
                            deleted_item_keys: vec![
                                ("gone-user".into(), "t1".into()),
                                ("gone-agent".into(), "t1".into()),
                            ]
                            .into_iter()
                            .collect(),
                            ..Default::default()
                        };
                        sync_thread_history_projection_databases(
                            std::slice::from_ref(&path),
                            "s",
                            &plan,
                        )
                        .unwrap();
                        let c = Connection::open(&path).unwrap();
                        let actual: (Option<String>,Option<String>) = c.query_row("SELECT first_user_item_id,final_agent_item_id FROM thread_turns WHERE thread_id='s' AND turn_id='t1'",[],|r| Ok((r.get(0)?,r.get(1)?))).unwrap();
                        let expected = if phase == Some("final_answer") {
                            Some("retained-agent")
                        } else if earlier_final {
                            Some("earlier-final")
                        } else if phase.is_none() && status != "inProgress" {
                            Some("retained-agent")
                        } else {
                            None
                        };
                        assert_eq!(actual.0.as_deref(), Some("user-first"));
                        assert_eq!(
                            actual.1.as_deref(),
                            expected,
                            "status={status}, phase={phase:?}, earlier_final={earlier_final}"
                        );
                        let untouched: i64 = c.query_row("SELECT COUNT(*) FROM thread_turns WHERE final_agent_item_id='sentinel' AND first_user_item_id='sentinel'",[],|r|r.get(0)).unwrap();
                        assert_eq!(untouched, 2);
                        drop(c);
                        fs::remove_dir_all(path.parent().unwrap()).unwrap();
                    }
                }
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum CanonicalUserInput {
        Text {
            text: String,
            #[serde(default)]
            text_elements: Vec<Value>,
        },
    }

    fn canonical_user_records(id: &str) -> Vec<Value> {
        vec![
            json!({"type":"session_meta","payload":{"id":id}}),
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"turn"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"old"}]}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn","item":{"type":"UserMessage","id":"user","content":[{"type":"text","text":"old","text_elements":[]}]}}}),
        ]
    }

    #[test]
    fn regression_user_edit_retains_official_event_schema() {
        for newline in ["\n", "\r\n"] {
            for text in ["edited", "中文🙂\nsecond line"] {
                let id = dynamic_session_id();
                let mut original = canonical_user_records(&id);
                let image = json!({"type":"image","image_url":"https://example.invalid/image.png"});
                original[3]["payload"]["item"]["content"] = json!([
                    {"type":"text","text":"old","text_elements":[{"byte_range":{"start":0,"end":3}}]},
                    image,
                    {"type":"text","text":"other","text_elements":[]}
                ]);
                let mut request = empty_context_request(id.clone(), None);
                request.message_updates = Some(vec![CodexSessionMessageUpdate {
                    line_number: 2,
                    role: None,
                    content: Some(text.into()),
                    deleted: None,
                }]);
                let (saved, _) = build_updated_rollout(
                    &serialize_jsonl(&original, newline).unwrap(),
                    &id,
                    &request,
                )
                .unwrap();
                let records =
                    validate_raw_jsonl(std::str::from_utf8(&saved).unwrap(), &id).unwrap();
                let content = &records[3].1["payload"]["item"]["content"];
                let CanonicalUserInput::Text {
                    text: actual,
                    text_elements,
                } = serde_json::from_value(content[0].clone()).unwrap();
                assert_eq!(actual, text);
                assert!(text_elements.is_empty());
                assert_eq!(content[1], image);
                assert_eq!(content.as_array().unwrap().len(), 2);
                assert_eq!(records[2].1["payload"]["content"][0]["type"], "input_text");
                assert_eq!(records[2].1["payload"]["content"][0]["text"], text);
            }
        }
    }

    #[test]
    fn regression_appended_user_uses_official_event_schema() {
        for role in ["user", "assistant"] {
            let id = dynamic_session_id();
            let mut request = empty_context_request(id.clone(), None);
            request.new_messages = Some(vec![CodexSessionNewMessage {
                role: role.into(),
                content: "new🙂".into(),
            }]);
            let (saved, _) = build_updated_rollout(
                &serialize_jsonl(&canonical_user_records(&id), "\n").unwrap(),
                &id,
                &request,
            )
            .unwrap();
            let records = validate_raw_jsonl(std::str::from_utf8(&saved).unwrap(), &id).unwrap();
            let content = &records
                .iter()
                .rev()
                .find(|(_, record)| record["payload"]["type"] == "item_completed")
                .unwrap()
                .1["payload"]["item"]["content"];
            assert_eq!(content[0]["text"], "new🙂");
            if role == "user" {
                assert!(serde_json::from_value::<Vec<CanonicalUserInput>>(content.clone()).is_ok());
            } else {
                assert_eq!(content[0]["type"], "Text");
            }
        }
    }

    #[test]
    fn regression_reasoning_edit_updates_official_event_fields() {
        let root = test_root("audit-real-reasoning");
        let id = dynamic_session_id();
        let fixture = seed_context_edit_fixture(&root, &id);
        let context = get_codex_session_context_from_home(&root, &id).unwrap();
        let mut request = context_save_request(&context);
        request.message_updates = Some(vec![CodexSessionMessageUpdate {
            line_number: 4,
            role: None,
            content: Some("Edited thoughts".into()),
            deleted: None,
        }]);
        save_codex_session_context_from_home(&root, request).unwrap();
        let saved =
            validate_raw_jsonl(&fs::read_to_string(&fixture.rollout).unwrap(), &id).unwrap();
        let actual = saved[5].1["payload"]["item"]["summary_text"].clone();
        assert_eq!(saved[5].1["payload"]["item"]["raw_content"], json!([]));
        assert!(saved[5].1["payload"]["item"].get("signature").is_none());
        let projected: Value = serde_json::from_str(&thread_item_json(
            &fixture.history_db,
            &id,
            fixture.reasoning_item_id,
        ))
        .unwrap();
        assert_eq!(projected["summary"], json!(["Edited thoughts"]));
        assert_eq!(projected["content"], json!([]));
        fs::remove_dir_all(root).unwrap();
        assert_eq!(
            actual,
            json!(["Edited thoughts"]),
            "Canonical Reasoning retains stale summary_text"
        );
    }

    #[test]
    fn regression_metadata_save_does_not_skip_unmaterialized_history() {
        let root = test_root("audit-incomplete-projection");
        let id = dynamic_session_id();
        let fixture = seed_context_edit_fixture(&root, &id);
        let connection = Connection::open(&fixture.history_db).unwrap();
        connection
            .execute("DELETE FROM thread_items WHERE thread_id=?1", [&id])
            .unwrap();
        connection.execute("UPDATE thread_history_projection_state SET next_rollout_byte_offset=0,next_rollout_ordinal=0 WHERE thread_id=?1", [&id]).unwrap();
        drop(connection);
        let context = get_codex_session_context_from_home(&root, &id).unwrap();
        let mut request = context_save_request(&context);
        request.title = Some("Renamed".into());
        let original = fs::read(&fixture.rollout).unwrap();
        let original_state = fs::read(&fixture.state_db).unwrap();
        let result = save_codex_session_context_from_home(&root, request);
        assert_eq!(fs::read(&fixture.rollout).unwrap(), original);
        assert_eq!(fs::read(&fixture.state_db).unwrap(), original_state);
        let connection = Connection::open(&fixture.history_db).unwrap();
        let cursor:i64 = connection.query_row("SELECT next_rollout_byte_offset FROM thread_history_projection_state WHERE thread_id=?1", [&id], |row| row.get(0)).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM thread_items WHERE thread_id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);
        fs::remove_dir_all(root).unwrap();
        assert!(result
            .unwrap_err()
            .contains("projection is incomplete or stale"));
        assert_eq!(cursor, 0);
        assert_eq!(count, 0);
    }

    #[test]
    fn regression_deleting_final_assistant_restores_previous_sqlite_summary() {
        let root = test_root("audit-final-agent-summary");
        let id = dynamic_session_id();
        let fixture = seed_context_edit_fixture(&root, &id);
        let mut records = validate_raw_jsonl(&fs::read_to_string(&fixture.rollout).unwrap(), &id)
            .unwrap()
            .into_iter()
            .map(|(_, r)| r)
            .collect::<Vec<_>>();
        let mut second_response = records[6].clone();
        second_response["ordinal"] = json!(8);
        second_response["metadata"]["retained_source"]["id"]["message_id"] = json!("second");
        second_response["payload"]["content"][0]["text"] = json!("Second answer");
        let mut second_event = records[7].clone();
        second_event["ordinal"] = json!(9);
        second_event["payload"]["item"]["id"] = json!("second");
        second_event["payload"]["item"]["content"][0]["text"] = json!("Second answer");
        records[8]["ordinal"] = json!(10);
        records[8]["payload"]["last_agent_message"] = json!("Second answer");
        records.splice(8..8, [second_response, second_event]);
        let original = serialize_jsonl(&records, "\n").unwrap();
        fs::write(&fixture.rollout, &original).unwrap();
        let connection = Connection::open(&fixture.history_db).unwrap();
        connection.execute("INSERT INTO thread_items VALUES (?1,'turn-fixture','second',9,1500,?2,'agentMessage',0,1500,1500)",rusqlite::params![id,json!({"type":"agentMessage","id":"second","text":"Second answer"}).to_string()]).unwrap();
        connection.execute("INSERT INTO thread_turns (thread_id,turn_id,rollout_ordinal,status,final_agent_item_id) VALUES (?1,'turn-fixture',1,'completed','second')",[&id]).unwrap();
        connection.execute("UPDATE thread_history_projection_state SET next_rollout_byte_offset=?1,next_rollout_ordinal=11 WHERE thread_id=?2",rusqlite::params![original.len() as i64,id]).unwrap();
        drop(connection);
        let context = get_codex_session_context_from_home(&root, &id).unwrap();
        let mut request = context_save_request(&context);
        request.message_updates = Some(vec![CodexSessionMessageUpdate {
            line_number: 8,
            role: None,
            content: None,
            deleted: Some(true),
        }]);
        save_codex_session_context_from_home(&root, request).unwrap();
        let connection = Connection::open(&fixture.history_db).unwrap();
        let final_id: Option<String> = connection
            .query_row(
                "SELECT final_agent_item_id FROM thread_turns WHERE thread_id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);
        fs::remove_dir_all(root).unwrap();
        assert_eq!(final_id.as_deref(),Some(fixture.agent_item_id),"SQLite drops final agent reference while rollout completion still summarizes the preceding answer");
    }

    #[test]
    fn regression_index_rewrite_preserves_other_thread_append_handle() {
        use std::io::Write;
        let root = test_root("audit-index-open-handle");
        fs::create_dir_all(&root).unwrap();
        let id = dynamic_session_id();
        let index = root.join("session_index.jsonl");
        fs::write(
            &index,
            format!("{}\n", json!({"id":id,"thread_name":"old"})),
        )
        .unwrap();
        let update = build_session_index_title_update(&root, &id, "renamed")
            .unwrap()
            .unwrap();
        // Official Codex opens an append handle before serializing and writing
        // a name update. Pause that writer across the editor's append.
        let mut other_writer = OpenOptions::new().append(true).open(&index).unwrap();
        let result = apply_session_index_title_update(&update);
        let other_id = dynamic_session_id();
        writeln!(
            other_writer,
            "{}",
            json!({"id":other_id,"thread_name":"other session"})
        )
        .unwrap();
        other_writer.flush().unwrap();
        drop(other_writer);
        let current = fs::read_to_string(&index).unwrap();
        fs::remove_dir_all(root).unwrap();
        result.unwrap();
        assert!(
            current.contains(&other_id),
            "Index loses another thread's successful append: {current}"
        );
        assert_eq!(
            latest_session_index_entry(current.as_bytes(), &id)
                .unwrap()
                .unwrap()["thread_name"],
            "renamed"
        );
    }
    #[test]
    fn reasoning_fields_read_and_rewrite_canonical_and_response_shapes() {
        for (value, expected) in [
            (
                json!({"summary_text":["a","b"],"raw_content":["raw"]}),
                "a\nb",
            ),
            (json!({"summary_text":[],"raw_content":["raw"]}), "raw"),
            (
                json!({"summary":[{"type":"summary_text","text":"summary"}],"content":[]}),
                "summary",
            ),
            (
                json!({"summary":[],"content":[{"type":"text","text":"content"}]}),
                "content",
            ),
        ] {
            assert_eq!(extract_reasoning_text(&value), expected);
            let mut item = value.as_object().unwrap().clone();
            item.insert("id".into(), json!("r"));
            rewrite_event_item_text(&mut item, TrackedItemKind::Reasoning, "edited🙂");
            assert_eq!(item["summary_text"], json!(["edited🙂"]));
            assert_eq!(item["raw_content"], json!([]));
            assert_eq!(item["id"], "r");
        }
    }

    #[test]
    fn projection_preflight_covers_save_actions_and_cursor_states() {
        for action in ["metadata", "edit", "delete", "append"] {
            for state in [
                "complete",
                "partial",
                "wrong-bytes",
                "wrong-ordinal",
                "past-eof",
                "missing",
            ] {
                let root = test_root("projection-preflight");
                let id = dynamic_session_id();
                let fixture = seed_context_edit_fixture(&root, &id);
                let original = fs::read(&fixture.rollout).unwrap();
                let c = Connection::open(&fixture.history_db).unwrap();
                match state {
                    "partial" => {
                        c.execute("UPDATE thread_history_projection_state SET next_rollout_byte_offset=0,next_rollout_ordinal=0",[]).unwrap();
                    }
                    "wrong-bytes" => {
                        c.execute(
                            "UPDATE thread_history_projection_state SET next_rollout_byte_offset=0",
                            [],
                        )
                        .unwrap();
                    }
                    "wrong-ordinal" => {
                        c.execute(
                            "UPDATE thread_history_projection_state SET next_rollout_ordinal=0",
                            [],
                        )
                        .unwrap();
                    }
                    "past-eof" => {
                        c.execute("UPDATE thread_history_projection_state SET next_rollout_byte_offset=next_rollout_byte_offset+1",[]).unwrap();
                    }
                    "missing" => {
                        c.execute("DELETE FROM thread_history_projection_state", [])
                            .unwrap();
                    }
                    _ => {}
                }
                drop(c);
                let before_history = fs::read(&fixture.history_db).unwrap();
                let before_state = fs::read(&fixture.state_db).unwrap();
                let context = get_codex_session_context_from_home(&root, &id).unwrap();
                let mut request = context_save_request(&context);
                match action {
                    "metadata" => request.title = Some("Renamed🙂".into()),
                    "append" => {
                        request.new_messages = Some(vec![CodexSessionNewMessage {
                            role: "user".into(),
                            content: "new🙂".into(),
                        }])
                    }
                    _ => {
                        request.message_updates = Some(vec![CodexSessionMessageUpdate {
                            line_number: assistant_line_number(&context),
                            role: None,
                            content: Some("edited🙂".into()),
                            deleted: Some(action == "delete"),
                        }])
                    }
                }
                let result = save_codex_session_context_from_home(&root, request);
                if state == "complete" || state == "missing" {
                    result.unwrap();
                    let c = Connection::open(&fixture.history_db).unwrap();
                    let cursor: Option<i64> = c
                        .query_row(
                            "SELECT next_rollout_byte_offset FROM thread_history_projection_state",
                            [],
                            |r| r.get(0),
                        )
                        .optional()
                        .unwrap();
                    assert_eq!(
                        cursor,
                        if state == "missing" {
                            None
                        } else {
                            Some(fs::metadata(&fixture.rollout).unwrap().len() as i64)
                        }
                    );
                } else {
                    assert!(result
                        .unwrap_err()
                        .contains("projection is incomplete or stale"));
                    assert_eq!(fs::read(&fixture.rollout).unwrap(), original);
                    assert_eq!(fs::read(&fixture.history_db).unwrap(), before_history);
                    assert_eq!(fs::read(&fixture.state_db).unwrap(), before_state);
                }
                fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn session_index_uses_latest_record_and_rejects_unterminated_append() {
        let root = test_root("index-latest");
        let id = dynamic_session_id();
        let path = root.join("session_index.jsonl");
        let original = format!(
            "{}\n{}\n",
            json!({"id":id,"thread_name":"first"}),
            json!({"id":id,"thread_name":"latest"})
        );
        fs::write(&path, &original).unwrap();
        assert!(build_session_index_title_update(&root, &id, "latest")
            .unwrap()
            .is_none());
        let update = build_session_index_title_update(&root, &id, "new")
            .unwrap()
            .unwrap();
        assert_eq!(update.previous["thread_name"], "latest");
        apply_session_index_title_update(&update).unwrap();
        restore_session_index(&update).unwrap();
        assert_eq!(
            latest_session_index_entry(&fs::read(&path).unwrap(), &id)
                .unwrap()
                .unwrap()["thread_name"],
            "latest"
        );
        fs::write(&path, original.trim_end()).unwrap();
        let update = build_session_index_title_update(&root, &id, "new")
            .unwrap()
            .unwrap();
        assert!(apply_session_index_title_update(&update)
            .unwrap_err()
            .contains("unterminated"));
        assert_eq!(fs::read_to_string(&path).unwrap(), original.trim_end());
        fs::remove_dir_all(root).unwrap();
    }
}
