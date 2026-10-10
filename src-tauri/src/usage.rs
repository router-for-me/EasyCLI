mod resp;
mod token;

#[cfg(target_os = "macos")]
use super::executable_dir;
#[cfg(test)]
use super::VersionDownloadSource;
use super::{
    apply_configured_proxy, core_base_dir, current_core_status, format_management_request_error,
    management_authorization, management_endpoint, management_http_client, CoreProcessState,
    GuiConfigFile, GuiConfigState,
};
use chrono::{DateTime, Local};
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension, Row,
    Transaction,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use tokio_util::sync::CancellationToken;

use self::{
    resp::{pull_usage_queue, UsageSubscription},
    token::{
        normalize as normalize_keeper_tokens, ClampedTokenFields, TokenValues as KeeperTokenValues,
    },
};

const USAGE_DIR_NAME: &str = "usage-records";
const USAGE_DATABASE_FILE: &str = "usage.db";
const USAGE_BACKUP_DIR_NAME: &str = "backups";
const LEGACY_USAGE_EVENTS_DIR: &str = "events";
const LEGACY_USAGE_INBOX_DIR: &str = "inbox";
const LEGACY_JSON_MIGRATION_KEY: &str = "legacy_json_v1";
const USAGE_DATABASE_MIGRATION_KEY: &str = "keeper_v3";
const USAGE_FAILURE_MIGRATION_KEY: &str = "failure_details_v4";
const USAGE_EVENT_KEY_MIGRATION_KEY: &str = "event_key_v5";
const USAGE_DATABASE_MAX_MB_KEY: &str = "max_database_size_mb";
const USAGE_UPDATED_EVENT: &str = "usage-records-updated";
const USAGE_SCHEMA_VERSION: u8 = 1;
const USAGE_DATABASE_SCHEMA_VERSION: i64 = 6;
const MAX_USAGE_FAILURE_BODY_CHARS: usize = 2_000;
const USAGE_QUEUE_BATCH_SIZE: usize = 10_000;
const USAGE_INBOX_PROCESS_LIMIT: usize = 500;
const USAGE_INBOX_MAX_ATTEMPTS: i64 = 5;
const USAGE_SUBSCRIBE_RETRY_SECONDS: u64 = 30;
const USAGE_QUEUE_KEY: &str = "usage";
const LEGACY_USAGE_QUEUE_KEY: &str = "queue";
const HTTP_USAGE_QUEUE_SOURCE: &str = "http_pull:usage_queue";
const SQLITE_BUSY_TIMEOUT_SECONDS: u64 = 5;
const BYTES_PER_MB: u64 = 1024 * 1024;
const TOKENS_PER_PRICE_UNIT: f64 = 1_000_000.0;
const LONG_CONTEXT_INPUT_TOKEN_THRESHOLD: u64 = 272_000;
const GROK_47_LONG_CONTEXT_INPUT_TOKEN_THRESHOLD: u64 = 200_000;
const GROK_47_MODEL_SQL: &str = concat!(
    "(LOWER(TRIM(model)) = 'grok-4.7'",
    " OR LOWER(TRIM(model)) LIKE 'grok-4.7-%'",
    " OR LOWER(TRIM(model)) LIKE '%/grok-4.7'",
    " OR LOWER(TRIM(model)) LIKE '%/grok-4.7-%')"
);
const BUNDLED_MODEL_PRICE_CATALOG: &str = include_str!("../resources/model_prices.json");
const MODEL_PRICE_SYNC_URL: &str = "https://models.dev/api.json";
const LITELLM_PRICE_SYNC_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const MODEL_PRICE_SYNC_MAX_BYTES: usize = 16 * 1024 * 1024;
const MODEL_PRICE_SYNC_ATTEMPTS: usize = 2;
const USAGE_DISPLAY_MODEL_SQL: &str =
    "COALESCE(NULLIF(TRIM(alias), ''), NULLIF(TRIM(model), ''), 'unknown')";

static USAGE_QUERY_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

async fn run_usage_task<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let permit = USAGE_QUERY_SLOTS
        .acquire()
        .await
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        task()
    })
    .await
    .map_err(|error| format!("Usage records background task failed: {error}"))?
}

pub(crate) struct UsageCollectorState {
    inner: Mutex<UsageCollectorInner>,
}

struct UsageCollectorInner {
    token: Option<CancellationToken>,
    status: UsageCollectorStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UsageDatabaseLayout {
    Empty,
    LegacyV2,
    CurrentV3,
}

#[derive(Debug)]
struct UsageInboxRow {
    id: i64,
    source: String,
    raw_message: String,
    attempt_count: i64,
}

#[derive(Clone, Copy, Default)]
struct UsagePersistOutcome {
    inserted: usize,
    deleted: u64,
}

impl UsagePersistOutcome {
    fn add(&mut self, other: Self) {
        self.inserted = self.inserted.saturating_add(other.inserted);
        self.deleted = self.deleted.saturating_add(other.deleted);
    }
}

#[derive(Debug, Eq, PartialEq)]
struct UsageMigrationSnapshot {
    records: i64,
    input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    total_tokens: i64,
    successes: i64,
    failures: i64,
    first_timestamp: Option<String>,
    last_timestamp: Option<String>,
}

impl Default for UsageCollectorState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(UsageCollectorInner {
                token: None,
                status: UsageCollectorStatus::waiting(),
            }),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageCollectorStatus {
    state: String,
    message: String,
    last_collected_at: Option<String>,
    total_records: u64,
}

impl UsageCollectorStatus {
    fn waiting() -> Self {
        Self {
            state: "waiting-core".to_string(),
            message: "Waiting for kernel startup".to_string(),
            last_collected_at: None,
            total_records: 0,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct UsageTokenStats {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    reasoning_tokens: u64,
    #[serde(default)]
    cache_read_tokens: u64,
    #[serde(default)]
    cache_creation_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct UsageRecord {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    row_id: Option<String>,
    timestamp: String,
    #[serde(default)]
    latency_ms: u64,
    #[serde(default)]
    ttft_ms: Option<u64>,
    #[serde(default)]
    source: String,
    #[serde(default)]
    source_display: String,
    #[serde(default)]
    auth_index: String,
    #[serde(default)]
    failed: bool,
    #[serde(default)]
    canceled: bool,
    #[serde(default)]
    failure_status: u16,
    #[serde(default)]
    failure_body: String,
    #[serde(default)]
    provider: String,
    #[serde(default, skip_serializing)]
    api_group_key: String,
    #[serde(default)]
    model: String,
    #[serde(default, alias = "responseModel")]
    response_model: String,
    // Derived from the active price catalog when reading a page; never trust imported estimates.
    #[serde(default, skip_deserializing)]
    cost: Option<UsageRecordCost>,
    #[serde(default)]
    alias: String,
    #[serde(default, skip_serializing)]
    client_ip: Option<String>,
    #[serde(default, skip_serializing)]
    x_forwarded_for: Option<String>,
    #[serde(default, skip_serializing)]
    user_agent: Option<String>,
    #[serde(default)]
    reasoning_effort: String,
    #[serde(default)]
    service_tier: String,
    #[serde(default)]
    response_service_tier: String,
    #[serde(default)]
    executor_type: String,
    #[serde(default)]
    endpoint: String,
    #[serde(default)]
    auth_type: String,
    #[serde(default)]
    api_key_hash: String,
    #[serde(default)]
    api_key_display: String,
    #[serde(default)]
    api_key_remark: String,
    #[serde(default)]
    request_id: String,
    #[serde(default = "default_usage_generate", skip_serializing)]
    generate: bool,
    #[serde(default, skip_serializing)]
    cached_tokens: u64,
    #[serde(default, skip_serializing)]
    collector_source: String,
    #[serde(default)]
    tokens: UsageTokenStats,
}

#[derive(Clone, Serialize)]
struct UsageRecordCost {
    total: f64,
    pricing_model: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyUsageHourFile {
    schema_version: u8,
    records: Vec<UsageRecord>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyUsageInboxFile {
    schema_version: u8,
    records: Vec<UsageRecord>,
}

#[derive(Clone, Default, Deserialize)]
pub(crate) struct UsageQuery {
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    api_key_hash: Option<String>,
    #[serde(default)]
    failed: Option<bool>,
    #[serde(default)]
    canceled: Option<bool>,
    #[serde(default)]
    page: Option<usize>,
    #[serde(default)]
    page_size: Option<usize>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageOverview {
    total_requests: u64,
    success_count: u64,
    failure_count: u64,
    canceled_count: u64,
    success_rate: f64,
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    total_tokens: u64,
    rpm: f64,
    tpm: f64,
    tps: f64,
    tps_sample_count: u64,
    average_latency_ms: f64,
    cache_hit_rate: f64,
    estimated_cost: f64,
    priced_requests: u64,
    timeline: Vec<UsageTimelinePoint>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageRepairResult {
    scanned: u64,
    repaired: u64,
    deleted: u64,
    backup_path: Option<String>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageStorageSettings {
    max_database_size_mb: u64,
    database_size_bytes: u64,
    total_records: u64,
    deleted_records: u64,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelPrice {
    model: String,
    prompt: f64,
    completion: f64,
    cache_read: f64,
    cache_creation: f64,
    prompt_configured: bool,
    completion_configured: bool,
    cache_read_configured: bool,
    cache_creation_configured: bool,
    source: String,
    source_model_id: String,
    updated_at_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelPriceCatalog {
    schema_version: u8,
    #[serde(default)]
    updated_at: String,
    models: HashMap<String, CatalogModelPrice>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogModelPrice {
    input_per_1_m: f64,
    output_per_1_m: f64,
    #[serde(default)]
    cache_read_per_1_m: Option<f64>,
    #[serde(default)]
    cache_creation_per_1_m: Option<f64>,
}

#[derive(Default)]
struct CostTokens {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_creation: u64,
    long_input: u64,
    long_output: u64,
    long_cache_read: u64,
    long_cache_creation: u64,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsagePricing {
    rows: Vec<UsagePriceRow>,
    total_cost: f64,
    total_requests: u64,
    priced_requests: u64,
    saved_prices: usize,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsagePriceRow {
    model: String,
    requests: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    total_tokens: u64,
    estimated_cost: f64,
    price: Option<ModelPrice>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelPriceSyncResult {
    imported: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelPriceSyncPreview {
    source: String,
    source_url: String,
    matches: Vec<ModelPrice>,
    unmatched: Vec<String>,
}

struct UsageCostGroup {
    model: String,
    alias: String,
    service_tier: String,
    response_service_tier: String,
    executor_type: String,
    provider: String,
    auth_type: String,
    requests: u64,
    tokens: CostTokens,
    total_tokens: u64,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageTimelineModel {
    key: String,
    label: String,
    tokens: u64,
    requests: u64,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageTimelinePoint {
    hour: String,
    requests: u64,
    success: u64,
    failure: u64,
    canceled: u64,
    tokens: u64,
    models: Vec<UsageTimelineModel>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageAnalysis {
    models: Vec<UsageCategory>,
    providers: Vec<UsageCategory>,
    sources: Vec<UsageCategory>,
    api_keys: Vec<UsageCategory>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageCategory {
    key: String,
    label: String,
    requests: u64,
    failures: u64,
    tokens: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageEventPage {
    items: Vec<UsageRecord>,
    total: usize,
    page: usize,
    page_size: usize,
    total_pages: usize,
}

struct UsageSqlFilter {
    clause: String,
    params: Vec<SqlValue>,
}

impl UsageCollectorState {
    fn start(&self) -> Option<CancellationToken> {
        let mut inner = self.inner.lock().ok()?;
        if inner.token.is_some() {
            return None;
        }
        let token = CancellationToken::new();
        inner.token = Some(token.clone());
        Some(token)
    }

    fn stop(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(token) = inner.token.take() {
                token.cancel();
            }
        }
    }

    fn set_status(&self, status: UsageCollectorStatus) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.status = status;
        }
    }

    fn set_total_records(&self, total_records: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.status.total_records = total_records;
        }
    }

    fn apply_record_changes(&self, inserted: usize, deleted: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.status.total_records = inner
                .status
                .total_records
                .saturating_add(inserted as u64)
                .saturating_sub(deleted);
        }
    }

    fn status(&self) -> Result<UsageCollectorStatus, String> {
        self.inner
            .lock()
            .map(|inner| inner.status.clone())
            .map_err(|_| "Usage collection status lock is poisoned".to_string())
    }
}

pub(crate) fn initialize_usage_storage() -> Result<(), String> {
    let root = usage_root_dir()?;
    migrate_legacy_usage_storage(&root)?;
    initialize_usage_storage_at(&root)
}

#[cfg(target_os = "macos")]
fn migrate_legacy_usage_storage(target: &Path) -> Result<(), String> {
    let legacy = executable_dir()?.join(USAGE_DIR_NAME);
    migrate_usage_storage_directory(&legacy, target)
}

#[cfg(not(target_os = "macos"))]
fn migrate_legacy_usage_storage(target: &Path) -> Result<(), String> {
    let _ = target;
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn migrate_usage_storage_directory(source: &Path, target: &Path) -> Result<(), String> {
    if source == target || !source.is_dir() || target.exists() {
        return Ok(());
    }
    let target_parent = target
        .parent()
        .ok_or_else(|| "Usage records target directory has no parent directory".to_string())?;
    fs::create_dir_all(target_parent)
        .map_err(|error| format!("Failed to create usage records migration target directory: {error}"))?;
    if fs::rename(source, target).is_ok() {
        return Ok(());
    }

    if let Err(error) = copy_usage_storage_directory(source, target) {
        let _ = fs::remove_dir_all(target);
        return Err(format!("Failed to migrate legacy usage records: {error}"));
    }
    if let Err(error) = fs::remove_dir_all(source) {
        eprintln!(
            "Legacy usage records were copied, but the original directory could not be cleaned up {}: {error}",
            source.display()
        );
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn copy_usage_storage_directory(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|error| format!("Failed to create usage records directory: {error}"))?;
    let entries = fs::read_dir(source).map_err(|error| format!("Failed to read legacy usage records: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("Failed to read legacy usage record entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to read legacy usage record entry type: {error}"))?;
        let destination = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_usage_storage_directory(&entry.path(), &destination)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &destination)
                .map_err(|error| format!("Failed to copy legacy usage records: {error}"))?;
        } else {
            return Err(format!(
                "Legacy usage records contain an unsupported file type: {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn initialize_usage_storage_at(root: &Path) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| format!("Failed to create usage records directory: {error}"))?;
    let mut connection = open_usage_database_at(root)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| format!("Failed to enable SQLite WAL: {error}"))?;
    migrate_usage_database(&mut connection, root)?;
    migrate_legacy_json_storage(&mut connection, root)?;
    cleanup_usage_inbox(&connection, Local::now())?;
    enforce_usage_database_limit(&connection).map(|_| ())
}

#[tauri::command]
pub(crate) async fn repair_usage_cache_records() -> Result<UsageRepairResult, String> {
    run_usage_task(|| repair_usage_cache_records_at(&usage_root_dir()?)).await
}

#[tauri::command]
pub(crate) async fn get_usage_storage_settings() -> Result<UsageStorageSettings, String> {
    run_usage_task(|| {
        let root = usage_root_dir()?;
        let connection = open_usage_database_at(&root)?;
        load_usage_storage_settings(&connection, &root, 0)
    })
    .await
}

#[tauri::command]
pub(crate) async fn save_usage_storage_settings(
    app: tauri::AppHandle,
    max_database_size_mb: u64,
) -> Result<UsageStorageSettings, String> {
    let settings = run_usage_task(move || {
        let root = usage_root_dir()?;
        let connection = open_usage_database_at(&root)?;
        save_usage_database_limit(&connection, max_database_size_mb)?;
        let deleted = enforce_usage_database_limit(&connection)?;
        load_usage_storage_settings(&connection, &root, deleted)
    })
    .await?;
    app.state::<UsageCollectorState>()
        .set_total_records(settings.total_records);
    if settings.deleted_records > 0 {
        let _ = app.emit(USAGE_UPDATED_EVENT, Local::now().to_rfc3339());
    }
    Ok(settings)
}

#[tauri::command]
pub(crate) async fn shrink_usage_database(
    app: tauri::AppHandle,
    target_database_size_mb: u64,
) -> Result<UsageStorageSettings, String> {
    let settings = run_usage_task(move || {
        let root = usage_root_dir()?;
        let connection = open_usage_database_at(&root)?;
        let deleted = shrink_usage_database_to_mb(&connection, target_database_size_mb)?;
        load_usage_storage_settings(&connection, &root, deleted)
    })
    .await?;
    app.state::<UsageCollectorState>()
        .set_total_records(settings.total_records);
    if settings.deleted_records > 0 {
        let _ = app.emit(USAGE_UPDATED_EVENT, Local::now().to_rfc3339());
    }
    Ok(settings)
}

fn repair_usage_cache_records_at(root: &Path) -> Result<UsageRepairResult, String> {
    let mut connection = open_usage_database_at(root)?;
    let candidate_count: i64 = connection
        .query_row(
            r#"SELECT COUNT(*) FROM usage_events
               WHERE lower(trim(model)) = 'unknown'
                  OR (input_tokens > 0
                      AND cache_read_tokens + cache_creation_tokens > input_tokens
                      AND (lower(executor_type) = 'claudeexecutor'
                           OR lower(provider) = 'claude'
                           OR lower(provider) LIKE '%anthropic%'))"#,
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("Failed to check historical usage records for invalid rows: {error}"))?;

    if candidate_count <= 0 {
        return Ok(UsageRepairResult::default());
    }

    let backup_path = {
        let backup_dir = root.join(USAGE_BACKUP_DIR_NAME);
        fs::create_dir_all(&backup_dir)
            .map_err(|error| format!("Failed to create Claude usage record migration backup directory: {error}"))?;
        let backup_path = backup_dir.join(format!(
            "usage-before-history-repair-v2-{}.db",
            unique_file_stamp()
        ));
        connection
            .execute(
                "VACUUM INTO ?1",
                params![backup_path.to_string_lossy().to_string()],
            )
            .map_err(|error| format!("Failed to back up Claude usage records: {error}"))?;
        backup_path.to_string_lossy().to_string()
    };

    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed to begin Claude usage record migration transaction: {error}"))?;
    let migrated = transaction
        .execute(
            r#"UPDATE usage_events
               SET input_tokens = input_tokens + cache_read_tokens + cache_creation_tokens,
                   total_tokens = CASE
                       WHEN total_tokens = 0
                            OR total_tokens = input_tokens + output_tokens
                       THEN input_tokens + cache_read_tokens + cache_creation_tokens + output_tokens
                       ELSE total_tokens
                   END,
                   cached_tokens = MAX(cached_tokens, cache_read_tokens + cache_creation_tokens)
               WHERE input_tokens > 0
                 AND cache_read_tokens + cache_creation_tokens > input_tokens
                 AND lower(trim(model)) <> 'unknown'
                 AND (lower(executor_type) = 'claudeexecutor'
                      OR lower(provider) = 'claude'
                      OR lower(provider) LIKE '%anthropic%')"#,
            [],
        )
        .map_err(|error| format!("Failed to migrate Claude usage records: {error}"))?;
    let deleted = transaction
        .execute(
            "DELETE FROM usage_events WHERE lower(trim(model)) = 'unknown'",
            [],
        )
        .map_err(|error| format!("Failed to delete historical unknown records: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Failed to commit Claude usage record migration: {error}"))?;
    Ok(UsageRepairResult {
        scanned: candidate_count.max(0) as u64,
        repaired: migrated as u64,
        deleted: deleted as u64,
        backup_path: Some(backup_path),
    })
}

fn migrate_usage_database(connection: &mut Connection, root: &Path) -> Result<(), String> {
    match detect_usage_database_layout(connection)? {
        UsageDatabaseLayout::Empty => initialize_usage_schema(connection),
        UsageDatabaseLayout::CurrentV3 => {
            initialize_usage_schema(connection)?;
            migrate_usage_event_key_uniqueness(connection)
        }
        UsageDatabaseLayout::LegacyV2 => {
            let backup_path = create_usage_migration_backup(connection, root)?;
            if let Err(error) = migrate_legacy_v2_usage_schema(connection) {
                return Err(format!(
                    "Failed to migrate legacy usage records database; backup retained at {}: {error}",
                    backup_path.display()
                ));
            }
            initialize_usage_schema(connection)
        }
    }
}

fn detect_usage_database_layout(connection: &Connection) -> Result<UsageDatabaseLayout, String> {
    if !usage_table_exists(connection, "usage_events")? {
        return Ok(UsageDatabaseLayout::Empty);
    }
    let columns = usage_table_columns(connection, "usage_events")?;
    let legacy_columns = [
        "event_key",
        "timestamp_ms",
        "local_hour",
        "api_key_hash",
        "api_key_display",
        "api_key_remark",
    ];
    if !legacy_columns
        .iter()
        .all(|column| columns.contains(*column))
    {
        return Err("Unrecognized usage records database schema; automatic migration refused".to_string());
    }
    let keeper_columns = [
        "api_group_key",
        "model_alias",
        "client_ip",
        "x_forwarded_for",
        "user_agent",
        "generate",
        "cached_tokens",
        "collector_source",
    ];
    let keeper_column_count = keeper_columns
        .iter()
        .filter(|column| columns.contains(**column))
        .count();
    if keeper_column_count == 0 && !usage_table_exists(connection, "usage_inbox")? {
        return Ok(UsageDatabaseLayout::LegacyV2);
    }
    if keeper_column_count == keeper_columns.len()
        && usage_table_exists(connection, "usage_inbox")?
        && usage_table_exists(connection, "usage_aggregation_checkpoints")?
    {
        return Ok(UsageDatabaseLayout::CurrentV3);
    }
    Err("Incomplete usage records database migration detected; further modifications refused".to_string())
}

fn usage_table_exists(connection: &Connection, table: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            params![table],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists != 0)
        .map_err(|error| format!("Failed to inspect SQLite table {table}: {error}"))
}

fn usage_table_columns(connection: &Connection, table: &str) -> Result<HashSet<String>, String> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .map_err(|error| format!("Failed to prepare SQLite table schema read for {table}: {error}"))?;
    let columns = statement
        .query_map(params![table], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Failed to read SQLite table schema {table}: {error}"))?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| format!("Failed to parse SQLite table schema {table}: {error}"))?;
    Ok(columns)
}

fn migrate_usage_event_key_uniqueness(connection: &mut Connection) -> Result<(), String> {
    let migrated = connection
        .query_row(
            "SELECT value FROM usage_metadata WHERE key = ?1",
            params![USAGE_EVENT_KEY_MIGRATION_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("read usage event key migration state failed: {error}"))?;
    if migrated.is_some() {
        return Ok(());
    }

    let transaction = connection
        .transaction()
        .map_err(|error| format!("begin usage event key migration failed: {error}"))?;
    ensure_usage_event_key_non_unique_in_transaction(&transaction)?;
    transaction
        .execute(
            "INSERT INTO usage_metadata (key, value) VALUES (?1, ?2)",
            params![USAGE_EVENT_KEY_MIGRATION_KEY, Local::now().to_rfc3339()],
        )
        .map_err(|error| format!("record usage event key migration state failed: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("commit usage event key migration failed: {error}"))
}

fn ensure_usage_event_key_non_unique_in_transaction(connection: &Connection) -> Result<(), String> {
    if !usage_table_exists(connection, "usage_events")? {
        return Ok(());
    }

    let unique_indexes = usage_event_key_unique_indexes(connection)?;
    if unique_indexes.is_empty() {
        create_usage_event_key_index(connection)?;
        return Ok(());
    }

    if unique_indexes
        .iter()
        .all(|(_, is_auto_index)| !is_auto_index)
    {
        for (index_name, _) in unique_indexes {
            connection
                .execute(
                    &format!(
                        "DROP INDEX IF EXISTS {}",
                        quote_sqlite_identifier(&index_name)
                    ),
                    [],
                )
                .map_err(|error| {
                    format!("drop unique usage event key index {index_name} failed: {error}")
                })?;
        }
        create_usage_event_key_index(connection)?;
        return Ok(());
    }

    rebuild_usage_events_without_event_key_unique(connection, &unique_indexes)
}

fn usage_event_key_unique_indexes(connection: &Connection) -> Result<Vec<(String, bool)>, String> {
    let index_names = {
        let mut statement = connection
            .prepare("SELECT name FROM pragma_index_list(?1) WHERE \"unique\" != 0 ORDER BY seq")
            .map_err(|error| format!("prepare usage event key index query failed: {error}"))?;
        let index_names = statement
            .query_map(params!["usage_events"], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query usage event key indexes failed: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read usage event key indexes failed: {error}"))?;
        index_names
    };

    let mut matches = Vec::new();
    for index_name in index_names {
        let mut statement = connection
            .prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")
            .map_err(|error| format!("prepare usage event key index columns failed: {error}"))?;
        let columns = statement
            .query_map(params![index_name.as_str()], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query usage event key index columns failed: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read usage event key index columns failed: {error}"))?;
        if columns.len() == 1 && columns[0] == "event_key" {
            matches.push((
                index_name.clone(),
                index_name.starts_with("sqlite_autoindex_"),
            ));
        }
    }
    Ok(matches)
}

fn rebuild_usage_events_without_event_key_unique(
    connection: &Connection,
    unique_indexes: &[(String, bool)],
) -> Result<(), String> {
    let table_sql = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params!["usage_events"],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("read usage events table schema failed: {error}"))?;
    let temporary_table = format!("usage_events_rebuild_{}", unique_file_stamp());
    let create_table_sql = replace_sql_fragment_case_insensitive(
        &table_sql,
        "create table usage_events",
        &format!("CREATE TABLE {temporary_table}"),
    )
    .or_else(|| {
        replace_sql_fragment_case_insensitive(
            &table_sql,
            "create table if not exists usage_events",
            &format!("CREATE TABLE {temporary_table}"),
        )
    })
    .ok_or_else(|| "usage events table schema has an unsupported CREATE TABLE form".to_string())?;
    let create_table_sql = replace_sql_fragment_case_insensitive(
        &create_table_sql,
        "event_key text not null unique",
        "event_key TEXT NOT NULL",
    )
    .ok_or_else(|| {
        "usage events table schema does not contain the expected unique event_key constraint"
            .to_string()
    })?;

    let unique_index_names = unique_indexes
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<HashSet<_>>();
    let index_sqls = {
        let mut statement = connection
            .prepare(
                "SELECT name, sql FROM sqlite_master WHERE type = 'index' AND tbl_name = ?1 AND sql IS NOT NULL ORDER BY name",
            )
            .map_err(|error| format!("prepare usage event index schema query failed: {error}"))?;
        let index_sqls = statement
            .query_map(params!["usage_events"], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("query usage event index schemas failed: {error}"))?
            .filter_map(|row| match row {
                Ok((name, sql)) if !unique_index_names.contains(name.as_str()) => Some(Ok(sql)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read usage event index schemas failed: {error}"))?;
        index_sqls
    };

    connection
        .execute_batch(&create_table_sql)
        .map_err(|error| format!("create rebuilt usage events table failed: {error}"))?;
    connection
        .execute(
            &format!(
                "INSERT INTO {} SELECT * FROM usage_events",
                quote_sqlite_identifier(&temporary_table)
            ),
            [],
        )
        .map_err(|error| format!("copy usage events during event key migration failed: {error}"))?;
    connection
        .execute("DROP TABLE usage_events", [])
        .map_err(|error| format!("drop old usage events table failed: {error}"))?;
    connection
        .execute(
            &format!(
                "ALTER TABLE {} RENAME TO usage_events",
                quote_sqlite_identifier(&temporary_table)
            ),
            [],
        )
        .map_err(|error| format!("rename rebuilt usage events table failed: {error}"))?;

    for index_sql in index_sqls {
        connection
            .execute_batch(&index_sql)
            .map_err(|error| format!("restore usage event index failed: {error}"))?;
    }
    create_usage_event_key_index(connection)
}

fn create_usage_event_key_index(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_usage_events_event_key ON usage_events(event_key)",
            [],
        )
        .map(|_| ())
        .map_err(|error| format!("create usage event key index failed: {error}"))
}

fn quote_sqlite_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn replace_sql_fragment_case_insensitive(
    value: &str,
    needle: &str,
    replacement: &str,
) -> Option<String> {
    let value_lower = value.to_ascii_lowercase();
    let needle_lower = needle.to_ascii_lowercase();
    let start = value_lower.find(&needle_lower)?;
    let mut result = String::with_capacity(value.len() + replacement.len());
    result.push_str(&value[..start]);
    result.push_str(replacement);
    result.push_str(&value[start + needle.len()..]);
    Some(result)
}

fn create_usage_migration_backup(connection: &Connection, root: &Path) -> Result<PathBuf, String> {
    let backup_dir = root.join(USAGE_BACKUP_DIR_NAME);
    fs::create_dir_all(&backup_dir)
        .map_err(|error| format!("Failed to create usage records backup directory: {error}"))?;
    let backup_path = backup_dir.join(format!("usage-before-keeper-v3-{}.db", unique_file_stamp()));
    connection
        .execute(
            "VACUUM INTO ?1",
            params![backup_path.to_string_lossy().to_string()],
        )
        .map_err(|error| format!("Failed to back up legacy usage records database: {error}"))?;
    Ok(backup_path)
}

fn migrate_legacy_v2_usage_schema(connection: &mut Connection) -> Result<(), String> {
    let before = load_usage_migration_snapshot(connection)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed to begin usage records database migration transaction: {error}"))?;
    transaction
        .execute_batch(
            r#"
            ALTER TABLE usage_events ADD COLUMN api_group_key TEXT NOT NULL DEFAULT '';
            ALTER TABLE usage_events ADD COLUMN model_alias TEXT;
            ALTER TABLE usage_events ADD COLUMN client_ip TEXT;
            ALTER TABLE usage_events ADD COLUMN x_forwarded_for TEXT;
            ALTER TABLE usage_events ADD COLUMN user_agent TEXT;
            ALTER TABLE usage_events ADD COLUMN generate INTEGER NOT NULL DEFAULT 1;
            ALTER TABLE usage_events ADD COLUMN cached_tokens INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE usage_events ADD COLUMN collector_source TEXT NOT NULL DEFAULT 'legacy_migration';

            UPDATE usage_events
            SET api_group_key = CASE
                    WHEN api_key_hash != '' THEN api_key_hash
                    WHEN provider != '' THEN provider
                    WHEN endpoint != '' THEN endpoint
                    ELSE 'unknown'
                END,
                model_alias = NULLIF(alias, ''),
                generate = CASE
                    WHEN failed = 0
                         AND executor_type = 'CodexWebsocketsExecutor'
                         AND input_tokens = 0
                         AND output_tokens = 0
                         AND reasoning_tokens = 0
                         AND cache_read_tokens = 0
                         AND cache_creation_tokens = 0
                         AND total_tokens = 0
                    THEN 0 ELSE 1
                END,
                cached_tokens = cache_read_tokens + cache_creation_tokens,
                collector_source = 'legacy_migration';
            "#,
        )
        .map_err(|error| format!("Failed to convert legacy usage record fields: {error}"))?;
    let after = load_usage_migration_snapshot(&transaction)?;
    if before != after {
        return Err(format!(
            "Usage record migration verification mismatch: before {before:?}, after {after:?}"
        ));
    }
    initialize_usage_schema(&transaction)?;
    ensure_usage_event_key_non_unique_in_transaction(&transaction)?;
    transaction
        .execute(
            "INSERT INTO usage_metadata (key, value) VALUES (?1, ?2)",
            params![USAGE_EVENT_KEY_MIGRATION_KEY, Local::now().to_rfc3339()],
        )
        .map_err(|error| format!("record usage event key migration state failed: {error}"))?;
    transaction
        .execute(
            "INSERT INTO usage_metadata (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![USAGE_DATABASE_MIGRATION_KEY, Local::now().to_rfc3339()],
        )
        .map_err(|error| format!("Failed to write usage record migration marker: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Failed to commit usage records database migration: {error}"))
}

fn load_usage_migration_snapshot(
    connection: &Connection,
) -> Result<UsageMigrationSnapshot, String> {
    connection
        .query_row(
            r#"
            SELECT
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(reasoning_tokens), 0),
                COALESCE(SUM(cache_read_tokens), 0),
                COALESCE(SUM(cache_creation_tokens), 0),
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(CASE WHEN failed = 0 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN failed != 0 THEN 1 ELSE 0 END), 0),
                MIN(timestamp),
                MAX(timestamp)
            FROM usage_events
            "#,
            [],
            |row| {
                Ok(UsageMigrationSnapshot {
                    records: row.get(0)?,
                    input_tokens: row.get(1)?,
                    output_tokens: row.get(2)?,
                    reasoning_tokens: row.get(3)?,
                    cache_read_tokens: row.get(4)?,
                    cache_creation_tokens: row.get(5)?,
                    total_tokens: row.get(6)?,
                    successes: row.get(7)?,
                    failures: row.get(8)?,
                    first_timestamp: row.get(9)?,
                    last_timestamp: row.get(10)?,
                })
            },
        )
        .map_err(|error| format!("Failed to read usage record migration verification snapshot: {error}"))
}

fn initialize_usage_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS usage_metadata (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS usage_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_key TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                local_hour TEXT NOT NULL,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                ttft_ms INTEGER,
                source TEXT NOT NULL DEFAULT '',
                auth_index TEXT NOT NULL DEFAULT '',
                failed INTEGER NOT NULL DEFAULT 0,
                canceled INTEGER NOT NULL DEFAULT 0,
                failure_status INTEGER NOT NULL DEFAULT 0,
                failure_body TEXT NOT NULL DEFAULT '',
                provider TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                response_model TEXT NOT NULL DEFAULT '',
                alias TEXT NOT NULL DEFAULT '',
                reasoning_effort TEXT NOT NULL DEFAULT '',
                service_tier TEXT NOT NULL DEFAULT '',
                response_service_tier TEXT NOT NULL DEFAULT '',
                executor_type TEXT NOT NULL DEFAULT '',
                endpoint TEXT NOT NULL DEFAULT '',
                auth_type TEXT NOT NULL DEFAULT '',
                api_key_hash TEXT NOT NULL DEFAULT '',
                api_key_display TEXT NOT NULL DEFAULT '',
                api_key_remark TEXT NOT NULL DEFAULT '',
                request_id TEXT NOT NULL DEFAULT '',
                api_group_key TEXT NOT NULL DEFAULT '',
                model_alias TEXT,
                client_ip TEXT,
                x_forwarded_for TEXT,
                user_agent TEXT,
                generate INTEGER NOT NULL DEFAULT 1,
                cached_tokens INTEGER NOT NULL DEFAULT 0,
                collector_source TEXT NOT NULL DEFAULT '',
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                reasoning_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_usage_events_timestamp
                ON usage_events(timestamp_ms DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_usage_events_event_key
                ON usage_events(event_key);
            CREATE INDEX IF NOT EXISTS idx_usage_events_local_hour
                ON usage_events(local_hour, timestamp_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_usage_events_model_timestamp
                ON usage_events(model, timestamp_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_usage_events_provider_timestamp
                ON usage_events(provider, timestamp_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_usage_events_source_timestamp
                ON usage_events(source, timestamp_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_usage_events_api_key_timestamp
                ON usage_events(api_key_hash, timestamp_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_usage_events_failed_timestamp
                ON usage_events(failed, timestamp_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_usage_events_api_group_timestamp
                ON usage_events(api_group_key, timestamp_ms DESC);

            CREATE TABLE IF NOT EXISTS usage_inbox (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                message_hash TEXT NOT NULL,
                raw_message TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                attempt_count INTEGER NOT NULL DEFAULT 0,
                last_error TEXT NOT NULL DEFAULT '',
                usage_event_key TEXT NOT NULL DEFAULT '',
                received_at TEXT NOT NULL,
                processed_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_usage_inbox_status_id
                ON usage_inbox(status, id);

            CREATE TABLE IF NOT EXISTS usage_aggregation_checkpoints (
                name TEXT PRIMARY KEY NOT NULL,
                last_usage_event_id INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS model_prices (
                model TEXT PRIMARY KEY NOT NULL,
                prompt_per_1m REAL NOT NULL DEFAULT 0,
                completion_per_1m REAL NOT NULL DEFAULT 0,
                cache_per_1m REAL NOT NULL DEFAULT 0,
                cache_read_per_1m REAL NOT NULL DEFAULT 0,
                cache_creation_per_1m REAL NOT NULL DEFAULT 0,
                prompt_configured INTEGER NOT NULL DEFAULT 0,
                completion_configured INTEGER NOT NULL DEFAULT 0,
                cache_read_configured INTEGER NOT NULL DEFAULT 0,
                cache_creation_configured INTEGER NOT NULL DEFAULT 0,
                source TEXT NOT NULL DEFAULT '',
                source_model_id TEXT NOT NULL DEFAULT '',
                updated_at_ms INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )
        .map_err(|error| format!("Failed to initialize SQLite usage records schema: {error}"))?;
    ensure_usage_failure_columns(connection)?;
    ensure_usage_response_model_column(connection)?;
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_usage_events_canceled_timestamp ON usage_events(canceled, timestamp_ms DESC)",
            [],
        )
        .map_err(|error| format!("Failed to create SQLite canceled-record index: {error}"))?;
    connection
        .pragma_update(None, "user_version", USAGE_DATABASE_SCHEMA_VERSION)
        .map_err(|error| format!("Failed to update SQLite usage records version: {error}"))
}

fn ensure_usage_response_model_column(connection: &Connection) -> Result<(), String> {
    if !usage_table_columns(connection, "usage_events")?.contains("response_model") {
        connection
            .execute(
                "ALTER TABLE usage_events ADD COLUMN response_model TEXT NOT NULL DEFAULT ''",
                [],
            )
            .map_err(|error| {
                format!("Failed to add SQLite usage record response_model field: {error}")
            })?;
    }
    Ok(())
}

fn ensure_usage_failure_columns(connection: &Connection) -> Result<(), String> {
    let mut columns = usage_table_columns(connection, "usage_events")?;
    for (column, definition) in [
        ("canceled", "INTEGER NOT NULL DEFAULT 0"),
        ("failure_status", "INTEGER NOT NULL DEFAULT 0"),
        ("failure_body", "TEXT NOT NULL DEFAULT ''"),
    ] {
        if columns.contains(column) {
            continue;
        }
        connection
            .execute(
                &format!("ALTER TABLE usage_events ADD COLUMN {column} {definition}"),
                [],
            )
            .map_err(|error| format!("Failed to add SQLite usage record field {column}: {error}"))?;
        columns.insert(column.to_string());
    }
    backfill_usage_failure_details(connection)?;
    Ok(())
}

fn backfill_usage_failure_details(connection: &Connection) -> Result<(), String> {
    let migrated = connection
        .query_row(
            "SELECT value FROM usage_metadata WHERE key = ?1",
            params![USAGE_FAILURE_MIGRATION_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Failed to read usage record failure-details migration status: {error}"))?;
    if migrated.is_some() {
        return Ok(());
    }

    if usage_table_exists(connection, "usage_inbox")? {
        let rows = {
            let mut statement = connection
                .prepare(
                    r#"
                    SELECT usage_event_key, raw_message
                    FROM usage_inbox
                    WHERE status = 'processed' AND usage_event_key != ''
                    "#,
                )
                .map_err(|error| format!("Failed to prepare usage record failure-details backfill: {error}"))?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| format!("Failed to query usage record failure details: {error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Failed to read usage record failure details: {error}"))?;
            rows
        };
        for (event_key, raw_message) in rows {
            let Some(object) = serde_json::from_str::<Value>(&raw_message)
                .ok()
                .and_then(|value| value.as_object().cloned())
            else {
                continue;
            };
            if !object
                .get("failed")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                continue;
            }
            let (failure_status, failure_body) = usage_failure_details(&object);
            let canceled = usage_failure_is_canceled(failure_status, &failure_body);
            connection
                .execute(
                    r#"
                    UPDATE usage_events
                    SET canceled = ?1, failure_status = ?2, failure_body = ?3
                    WHERE event_key = ?4
                    "#,
                    params![canceled, i64::from(failure_status), failure_body, event_key],
                )
                .map_err(|error| format!("Failed to backfill usage record failure details: {error}"))?;
        }
    }

    connection
        .execute(
            "INSERT INTO usage_metadata (key, value) VALUES (?1, ?2)",
            params![USAGE_FAILURE_MIGRATION_KEY, Local::now().to_rfc3339()],
        )
        .map_err(|error| format!("Failed to record usage record failure-details migration status: {error}"))?;
    Ok(())
}

fn open_usage_database() -> Result<Connection, String> {
    open_usage_database_at(&usage_root_dir()?)
}

fn open_usage_database_at(root: &Path) -> Result<Connection, String> {
    fs::create_dir_all(root).map_err(|error| format!("Failed to create usage records directory: {error}"))?;
    let path = root.join(USAGE_DATABASE_FILE);
    let connection = Connection::open(&path)
        .map_err(|error| format!("Failed to open SQLite usage records database {}: {error}", path.display()))?;
    connection
        .busy_timeout(Duration::from_secs(SQLITE_BUSY_TIMEOUT_SECONDS))
        .map_err(|error| format!("Failed to set SQLite busy timeout: {error}"))?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| format!("Failed to enable SQLite foreign keys: {error}"))?;
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| format!("Failed to set SQLite synchronous mode: {error}"))?;
    Ok(connection)
}

fn load_usage_database_limit(connection: &Connection) -> Result<u64, String> {
    let value = connection
        .query_row(
            "SELECT value FROM usage_metadata WHERE key = ?1",
            params![USAGE_DATABASE_MAX_MB_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Failed to read usage records database size limit: {error}"))?;
    value
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|error| format!("Invalid usage records database size limit: {error}"))
        })
        .transpose()
        .map(|value| value.unwrap_or(0))
}

fn save_usage_database_limit(
    connection: &Connection,
    max_database_size_mb: u64,
) -> Result<(), String> {
    max_database_size_mb
        .checked_mul(BYTES_PER_MB)
        .ok_or_else(|| "Usage records database size limit is too large".to_string())?;
    connection
        .execute(
            r#"INSERT INTO usage_metadata (key, value) VALUES (?1, ?2)
               ON CONFLICT(key) DO UPDATE SET value = excluded.value"#,
            params![USAGE_DATABASE_MAX_MB_KEY, max_database_size_mb.to_string()],
        )
        .map(|_| ())
        .map_err(|error| format!("Failed to save usage records database size limit: {error}"))
}

fn usage_database_active_bytes(connection: &Connection) -> Result<u64, String> {
    let page_size = connection
        .query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0))
        .map_err(|error| format!("Failed to read usage records database page size: {error}"))?;
    let page_count = connection
        .query_row("PRAGMA page_count", [], |row| row.get::<_, u64>(0))
        .map_err(|error| format!("Failed to read usage records database page count: {error}"))?;
    let free_pages = connection
        .query_row("PRAGMA freelist_count", [], |row| row.get::<_, u64>(0))
        .map_err(|error| format!("Failed to read usage records database free page count: {error}"))?;
    page_count
        .saturating_sub(free_pages)
        .checked_mul(page_size)
        .ok_or_else(|| "Usage records database size overflow".to_string())
}

fn usage_database_allocated_bytes(connection: &Connection) -> Result<u64, String> {
    let page_size = connection
        .query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0))
        .map_err(|error| format!("Failed to read usage records database page size: {error}"))?;
    let page_count = connection
        .query_row("PRAGMA page_count", [], |row| row.get::<_, u64>(0))
        .map_err(|error| format!("Failed to read usage records database page count: {error}"))?;
    page_count
        .checked_mul(page_size)
        .ok_or_else(|| "Usage records database size overflow".to_string())
}

fn usage_database_file_bytes(database_path: &Path) -> Result<u64, String> {
    let mut wal_path = database_path.as_os_str().to_os_string();
    wal_path.push("-wal");
    let paths = [database_path.to_path_buf(), PathBuf::from(wal_path)];
    let mut total = 0_u64;
    for path in paths {
        match fs::metadata(&path) {
            Ok(metadata) => total = total.saturating_add(metadata.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Failed to read usage records database file size {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Ok(total)
}

fn usage_database_disk_bytes(root: &Path) -> Result<u64, String> {
    usage_database_file_bytes(&root.join(USAGE_DATABASE_FILE))
}

fn usage_database_connection_disk_bytes(connection: &Connection) -> Result<u64, String> {
    let path = connection
        .path()
        .ok_or_else(|| "Unable to read usage records database file path".to_string())?;
    usage_database_file_bytes(Path::new(path))
}

fn truncate_usage_database_wal(connection: &Connection) {
    let _ = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    });
}

fn load_usage_storage_settings(
    connection: &Connection,
    root: &Path,
    deleted_records: u64,
) -> Result<UsageStorageSettings, String> {
    let total_records = connection
        .query_row("SELECT COUNT(*) FROM usage_events", [], |row| {
            row.get::<_, u64>(0)
        })
        .map_err(|error| format!("Failed to count usage records: {error}"))?;
    Ok(UsageStorageSettings {
        max_database_size_mb: load_usage_database_limit(connection)?,
        database_size_bytes: usage_database_disk_bytes(root)?,
        total_records,
        deleted_records,
    })
}

fn enforce_usage_database_limit(connection: &Connection) -> Result<u64, String> {
    let max_database_size_mb = load_usage_database_limit(connection)?;
    if max_database_size_mb == 0 {
        return Ok(0);
    }
    let max_bytes = max_database_size_mb
        .checked_mul(BYTES_PER_MB)
        .ok_or_else(|| "Usage records database size limit is too large".to_string())?;
    prune_usage_database_to_bytes(connection, max_bytes)
}

fn shrink_usage_database_to_mb(
    connection: &Connection,
    target_database_size_mb: u64,
) -> Result<u64, String> {
    if target_database_size_mb == 0 {
        return Err("Database compaction target must be greater than 0 MB".to_string());
    }
    let target_bytes = target_database_size_mb
        .checked_mul(BYTES_PER_MB)
        .ok_or_else(|| "Database compaction target is too large".to_string())?;
    prune_usage_database_to_bytes(connection, target_bytes)
}

fn prune_usage_database_to_bytes(connection: &Connection, max_bytes: u64) -> Result<u64, String> {
    if usage_database_connection_disk_bytes(connection)? <= max_bytes {
        return Ok(0);
    }
    truncate_usage_database_wal(connection);
    let allocated_bytes = usage_database_allocated_bytes(connection)?;
    if allocated_bytes <= max_bytes {
        return Ok(0);
    }

    let inbox_deleted = connection
        .execute(
            "DELETE FROM usage_inbox WHERE status IN ('processed', 'decode_failed', 'discarded')",
            [],
        )
        .map_err(|error| format!("Failed to clean up temporary usage data: {error}"))?;
    let mut active_bytes = usage_database_active_bytes(connection)?;
    let mut deleted_records = 0_u64;

    while active_bytes > max_bytes {
        let total_records = connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| {
                row.get::<_, u64>(0)
            })
            .map_err(|error| format!("Failed to count usage records pending cleanup: {error}"))?;
        if total_records == 0 {
            break;
        }
        let excess = active_bytes.saturating_sub(max_bytes);
        let estimated = ((total_records as u128 * excess as u128)
            .saturating_add(active_bytes.saturating_sub(1) as u128)
            / active_bytes.max(1) as u128) as u64;
        let batch = estimated.max(1).min(total_records).min(i64::MAX as u64) as i64;
        let deleted = connection
            .execute(
                r#"DELETE FROM usage_events
                   WHERE id IN (
                       SELECT id FROM usage_events
                       ORDER BY timestamp_ms ASC, id ASC
                       LIMIT ?1
                   )"#,
                params![batch],
            )
            .map_err(|error| format!("Failed to delete oldest usage records: {error}"))?
            as u64;
        if deleted == 0 {
            break;
        }
        deleted_records = deleted_records.saturating_add(deleted);
        active_bytes = usage_database_active_bytes(connection)?;
    }

    if allocated_bytes > max_bytes || inbox_deleted > 0 || deleted_records > 0 {
        connection
            .execute_batch("VACUUM;")
            .map_err(|error| format!("Failed to reclaim usage records database space: {error}"))?;
        truncate_usage_database_wal(connection);
    }
    Ok(deleted_records)
}

fn migrate_legacy_json_storage(connection: &mut Connection, root: &Path) -> Result<(), String> {
    let migrated = connection
        .query_row(
            "SELECT value FROM usage_metadata WHERE key = ?1",
            params![LEGACY_JSON_MIGRATION_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Failed to read legacy usage record migration status: {error}"))?
        .is_some();
    if migrated {
        return Ok(());
    }

    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed to begin legacy usage record migration transaction: {error}"))?;
    let mut migrated_records = 0_usize;

    for path in sorted_json_files(&root.join(LEGACY_USAGE_EVENTS_DIR))? {
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("Failed to read legacy usage record {}: {error}", path.display()))?;
        let file = serde_json::from_str::<LegacyUsageHourFile>(&content)
            .map_err(|error| format!("Failed to parse legacy usage record {}: {error}", path.display()))?;
        validate_legacy_schema(file.schema_version, &path)?;
        migrated_records = migrated_records.saturating_add(insert_usage_records_in_transaction(
            &transaction,
            &file.records,
        )?);
    }

    for path in sorted_json_files(&root.join(LEGACY_USAGE_INBOX_DIR))? {
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("Failed to read legacy usage record inbox {}: {error}", path.display()))?;
        let file = serde_json::from_str::<LegacyUsageInboxFile>(&content)
            .map_err(|error| format!("Failed to parse legacy usage record inbox {}: {error}", path.display()))?;
        validate_legacy_schema(file.schema_version, &path)?;
        migrated_records = migrated_records.saturating_add(insert_usage_records_in_transaction(
            &transaction,
            &file.records,
        )?);
    }

    transaction
        .execute(
            "INSERT INTO usage_metadata (key, value) VALUES (?1, ?2)",
            params![LEGACY_JSON_MIGRATION_KEY, migrated_records.to_string()],
        )
        .map_err(|error| format!("Failed to record legacy usage record migration status: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Failed to commit legacy usage record migration: {error}"))?;
    Ok(())
}

fn sorted_json_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(directory)
        .map_err(|error| format!("Failed to read legacy usage records directory {}: {error}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn validate_legacy_schema(version: u8, path: &Path) -> Result<(), String> {
    if version == USAGE_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(format!(
            "Unsupported legacy usage record version {version}: {}",
            path.display()
        ))
    }
}

#[derive(Default)]
struct RedisUsageQueueSource {
    selected_key: Option<&'static str>,
}

struct RawUsageQueueBatch {
    source: String,
    messages: Vec<String>,
}

impl RedisUsageQueueSource {
    async fn pull(&mut self, config: &GuiConfigFile) -> Result<RawUsageQueueBatch, String> {
        if let Some(queue_key) = self.selected_key {
            return Self::pull_key(config, queue_key).await;
        }

        match Self::pull_key(config, USAGE_QUEUE_KEY).await {
            Ok(batch) => {
                self.selected_key = Some(USAGE_QUEUE_KEY);
                Ok(batch)
            }
            Err(usage_error) if redis_pull_can_try_legacy_queue(&usage_error) => {
                match Self::pull_key(config, LEGACY_USAGE_QUEUE_KEY).await {
                    Ok(batch) => {
                        self.selected_key = Some(LEGACY_USAGE_QUEUE_KEY);
                        Ok(batch)
                    }
                    Err(legacy_error) => Err(format!(
                        "Failed to fetch Redis usage queue (usage: {usage_error}; queue: {legacy_error})"
                    )),
                }
            }
            Err(error) => Err(error),
        }
    }

    async fn pull_key(
        config: &GuiConfigFile,
        queue_key: &'static str,
    ) -> Result<RawUsageQueueBatch, String> {
        let messages = pull_usage_queue(
            config.port,
            &config.management_secret_key,
            queue_key,
            USAGE_QUEUE_BATCH_SIZE,
        )
        .await?;
        Ok(RawUsageQueueBatch {
            source: format!("redis_pull:{queue_key}"),
            messages,
        })
    }
}

fn redis_pull_can_try_legacy_queue(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("unsupported channel") || error.contains("unsupported queue")
}

pub(crate) fn start_usage_collector(app: tauri::AppHandle) {
    let state = app.state::<UsageCollectorState>();
    if let Ok(total_records) = total_usage_records() {
        state.set_total_records(total_records);
    }
    let Some(token) = state.start() else {
        return;
    };
    tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(usage_collector_loop(app, token));
    });
}

pub(crate) fn stop_usage_collector(app: &tauri::AppHandle) {
    app.state::<UsageCollectorState>().stop();
}

async fn usage_collector_loop(app: tauri::AppHandle, token: CancellationToken) {
    let root = match usage_root_dir() {
        Ok(root) => root,
        Err(error) => {
            set_collector_error(&app, error);
            return;
        }
    };

    let mut retry_seconds = 1_u64;
    let mut subscription: Option<UsageSubscription> = None;
    let mut subscription_config: Option<(u16, String)> = None;
    let mut redis_queue = RedisUsageQueueSource::default();
    let mut subscribe_retry_at = tokio::time::Instant::now();
    let mut next_inbox_cleanup_at = tokio::time::Instant::now() + Duration::from_secs(60 * 60);
    let mut next_inbox_recovery_at = tokio::time::Instant::now();
    let mut next_core_check_at = tokio::time::Instant::now();
    let mut core_ready = false;
    loop {
        if token.is_cancelled() {
            return;
        }
        let config = match app.state::<GuiConfigState>().snapshot() {
            Ok(config) => config,
            Err(error) => {
                set_collector_error(&app, error);
                wait_or_cancel(&token, retry_seconds).await;
                retry_seconds = (retry_seconds * 2).min(10);
                continue;
            }
        };
        if subscription_config.as_ref().is_some_and(|(port, secret)| {
            *port != config.port || secret != &config.management_secret_key
        }) {
            subscription = None;
            subscription_config = None;
            redis_queue = RedisUsageQueueSource::default();
            subscribe_retry_at = tokio::time::Instant::now();
        }
        if tokio::time::Instant::now() >= next_inbox_cleanup_at {
            if let Err(error) = open_usage_database_at(&root)
                .and_then(|connection| cleanup_usage_inbox(&connection, Local::now()))
            {
                eprintln!("Failed to clean up usage records inbox: {error}");
            }
            next_inbox_cleanup_at = tokio::time::Instant::now() + Duration::from_secs(60 * 60);
        }
        if tokio::time::Instant::now() >= next_inbox_recovery_at {
            let recovered = open_usage_database_at(&root)
                .and_then(|mut connection| process_usage_inbox(&mut connection, &config));
            match recovered {
                Ok(outcome) if outcome.inserted > 0 || outcome.deleted > 0 => {
                    let saved = outcome.inserted;
                    let collected_at = Local::now().to_rfc3339();
                    app.state::<UsageCollectorState>()
                        .apply_record_changes(outcome.inserted, outcome.deleted);
                    set_collector_status(
                        &app,
                        "collecting",
                        &format!("Restored {saved} pending records"),
                        Some(collected_at.clone()),
                    );
                    let _ = app.emit(USAGE_UPDATED_EVENT, collected_at);
                    retry_seconds = 1;
                    continue;
                }
                Ok(_) => {
                    next_inbox_recovery_at = tokio::time::Instant::now() + Duration::from_secs(2);
                }
                Err(error) => {
                    set_collector_error(&app, error);
                    wait_or_cancel(&token, retry_seconds).await;
                    retry_seconds = (retry_seconds * 2).min(10);
                    continue;
                }
            }
        }
        if tokio::time::Instant::now() >= next_core_check_at {
            let process_state = app.state::<CoreProcessState>();
            core_ready = current_core_status(Some(process_state.inner()), Some(config.port))
                .map(|status| status.ready)
                .unwrap_or(false);
            next_core_check_at = tokio::time::Instant::now() + Duration::from_secs(2);
        }
        if !core_ready {
            subscription = None;
            subscription_config = None;
            redis_queue = RedisUsageQueueSource::default();
            subscribe_retry_at = tokio::time::Instant::now();
            set_collector_status(&app, "waiting-core", "Waiting for kernel to become ready", None);
            retry_seconds = 1;
            wait_or_cancel(&token, 1).await;
            continue;
        }

        if subscription.is_none() && tokio::time::Instant::now() >= subscribe_retry_at {
            match UsageSubscription::connect(config.port, &config.management_secret_key).await {
                Ok(next_subscription) => {
                    subscription = Some(next_subscription);
                    subscription_config = Some((config.port, config.management_secret_key.clone()));
                    set_collector_status(&app, "collecting", "Connected to CPA usage live subscription", None);
                    match backfill_usage_queue(&root, &config, &mut redis_queue).await {
                        Ok(outcome) => {
                            let saved = outcome.inserted;
                            publish_collected_records(
                                &app,
                                outcome,
                                &format!("Live subscription connected; imported {saved} queued records"),
                            );
                            retry_seconds = 1;
                        }
                        Err(error) => {
                            set_collector_status(
                                &app,
                                "collecting",
                                &format!("Live subscription connected, but historical queue import failed: {error}"),
                                None,
                            );
                        }
                    }
                    continue;
                }
                Err(error) => {
                    subscribe_retry_at = tokio::time::Instant::now()
                        + Duration::from_secs(USAGE_SUBSCRIBE_RETRY_SECONDS);
                    set_collector_status(
                        &app,
                        "collecting",
                        &format!("Collecting in HTTP compatibility mode; live subscription unavailable: {error}"),
                        None,
                    );
                }
            }
        }

        if let Some(active_subscription) = subscription.as_mut() {
            let message = tokio::select! {
                _ = token.cancelled() => return,
                result = tokio::time::timeout(
                    Duration::from_secs(2),
                    active_subscription.next_message(),
                ) => result,
            };
            match message {
                Ok(Ok(payload)) => {
                    match persist_raw_usage_message_from_source(
                        &root,
                        "redis_subscribe:usage",
                        payload,
                        &config,
                    ) {
                        Ok(outcome) => {
                            let saved = outcome.inserted;
                            publish_collected_records(
                                &app,
                                outcome,
                                &format!("Live subscription saved {saved} new records"),
                            );
                            retry_seconds = 1;
                        }
                        Err(error) => {
                            set_collector_error(&app, error);
                            wait_or_cancel(&token, retry_seconds).await;
                            retry_seconds = (retry_seconds * 2).min(10);
                        }
                    }
                    continue;
                }
                Err(_) => {
                    set_collector_status(&app, "collecting", "Collecting from CPA usage live subscription", None);
                    retry_seconds = 1;
                    continue;
                }
                Ok(Err(error)) => {
                    subscription = None;
                    subscription_config = None;
                    subscribe_retry_at = tokio::time::Instant::now()
                        + Duration::from_secs(USAGE_SUBSCRIBE_RETRY_SECONDS);
                    set_collector_status(
                        &app,
                        "collecting",
                        &format!("Live subscription disconnected; switched to HTTP compatibility mode: {error}"),
                        None,
                    );
                }
            }
        }

        match pull_usage_queue_with_fallback(&mut redis_queue, &config).await {
            Ok(batch) if batch.messages.is_empty() => {
                set_collector_status(
                    &app,
                    "collecting",
                    &format!(
                        "Collecting usage records ({})",
                        collector_source_label(&batch.source)
                    ),
                    None,
                );
                retry_seconds = 1;
                wait_or_cancel(&token, 1).await;
            }
            Ok(batch) => {
                let source_label = collector_source_label(&batch.source);
                match persist_raw_usage_messages_from_source(
                    &root,
                    &batch.source,
                    batch.messages,
                    &config,
                ) {
                    Ok(outcome) => {
                        let saved = outcome.inserted;
                        publish_collected_records(
                            &app,
                            outcome,
                            &format!("{source_label} saved {saved} new records"),
                        );
                        retry_seconds = 1;
                    }
                    Err(error) => {
                        set_collector_error(&app, error);
                        wait_or_cancel(&token, retry_seconds).await;
                        retry_seconds = (retry_seconds * 2).min(10);
                    }
                }
            }
            Err(error) => {
                set_collector_error(&app, error);
                wait_or_cancel(&token, retry_seconds).await;
                retry_seconds = (retry_seconds * 2).min(10);
            }
        }
    }
}

async fn backfill_usage_queue(
    root: &Path,
    config: &GuiConfigFile,
    redis_queue: &mut RedisUsageQueueSource,
) -> Result<UsagePersistOutcome, String> {
    let mut outcome = UsagePersistOutcome::default();
    loop {
        match redis_queue.pull(config).await {
            Ok(batch) => {
                let fetched = batch.messages.len();
                if fetched == 0 {
                    return Ok(outcome);
                }
                outcome.add(persist_raw_usage_messages_from_source(
                    root,
                    &batch.source,
                    batch.messages,
                    config,
                )?);
                if fetched < USAGE_QUEUE_BATCH_SIZE {
                    return Ok(outcome);
                }
            }
            Err(redis_error) => {
                return backfill_http_usage_queue(root, config, outcome)
                    .await
                    .map_err(|http_error| {
                        format!("Failed to import queue (Redis: {redis_error}; HTTP: {http_error})")
                    });
            }
        }
    }
}

async fn backfill_http_usage_queue(
    root: &Path,
    config: &GuiConfigFile,
    mut outcome: UsagePersistOutcome,
) -> Result<UsagePersistOutcome, String> {
    loop {
        let messages = fetch_usage_queue_raw(config).await?;
        let fetched = messages.len();
        if fetched == 0 {
            return Ok(outcome);
        }
        outcome.add(persist_raw_usage_messages_from_source(
            root,
            HTTP_USAGE_QUEUE_SOURCE,
            messages,
            config,
        )?);
        if fetched < USAGE_QUEUE_BATCH_SIZE {
            return Ok(outcome);
        }
    }
}

fn publish_collected_records(app: &tauri::AppHandle, outcome: UsagePersistOutcome, message: &str) {
    let saved = outcome.inserted;
    let collected_at = Local::now().to_rfc3339();
    if saved > 0 || outcome.deleted > 0 {
        app.state::<UsageCollectorState>()
            .apply_record_changes(saved, outcome.deleted);
    }
    set_collector_status(
        app,
        "collecting",
        message,
        (saved > 0).then_some(collected_at.clone()),
    );
    if saved > 0 {
        let _ = app.emit(USAGE_UPDATED_EVENT, collected_at);
    }
}

pub(crate) fn persist_local_usage_event(
    app: &tauri::AppHandle,
    collector_source: &str,
    mut value: Value,
) -> Result<usize, String> {
    if let Some(object) = value.as_object_mut() {
        if string_field(object, "request_id").is_none() {
            object.insert(
                "request_id".to_string(),
                Value::String(format!("{collector_source}-{}", unique_file_stamp())),
            );
        }
    }
    let config = app.state::<GuiConfigState>().snapshot()?;
    let root = usage_root_dir()?;
    let outcome = persist_queue_items_from_source(&root, collector_source, vec![value], &config)?;
    publish_collected_records(app, outcome, "Saved desktop health check usage record");
    Ok(outcome.inserted)
}

async fn wait_or_cancel(token: &CancellationToken, seconds: u64) {
    tokio::select! {
        _ = token.cancelled() => {},
        _ = tokio::time::sleep(Duration::from_secs(seconds)) => {},
    }
}

async fn fetch_usage_queue(config: &GuiConfigFile) -> Result<Vec<Value>, String> {
    let client = management_http_client()?;
    let response = client
        .get(management_endpoint(config, "observability/usage/queue")?)
        .header("Authorization", management_authorization(config)?)
        .query(&[("count", USAGE_QUEUE_BATCH_SIZE)])
        .send()
        .await
        .map_err(|error| format_management_request_error("Failed to read CPA usage records queue", &error))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format_management_request_error("Failed to read CPA usage records response", &error))?;
    if !status.is_success() {
        return Err(format!(
            "CPA usage records queue returned HTTP {}: {}",
            status.as_u16(),
            text.trim()
        ));
    }
    serde_json::from_str::<Vec<Value>>(&text)
        .map_err(|error| format!("Failed to parse CPA usage records: {error}"))
}

async fn fetch_usage_queue_raw(config: &GuiConfigFile) -> Result<Vec<String>, String> {
    fetch_usage_queue(config)
        .await?
        .into_iter()
        .map(|item| {
            serde_json::to_string(&item)
                .map_err(|error| format!("Failed to serialize CPA usage records: {error}"))
        })
        .collect()
}

async fn pull_usage_queue_with_fallback(
    redis_queue: &mut RedisUsageQueueSource,
    config: &GuiConfigFile,
) -> Result<RawUsageQueueBatch, String> {
    match redis_queue.pull(config).await {
        Ok(batch) => Ok(batch),
        Err(redis_error) => fetch_usage_queue_raw(config)
            .await
            .map(|messages| RawUsageQueueBatch {
                source: HTTP_USAGE_QUEUE_SOURCE.to_string(),
                messages,
            })
            .map_err(|http_error| {
                format!("Failed to read CPA usage records (Redis: {redis_error}; HTTP: {http_error})")
            }),
    }
}

fn collector_source_label(source: &str) -> &'static str {
    if source.starts_with("redis_pull:") {
        "Redis queue"
    } else {
        "HTTP compatibility mode"
    }
}

fn set_collector_error(app: &tauri::AppHandle, error: String) {
    set_collector_status(app, "error", &error, None);
}

fn set_collector_status(
    app: &tauri::AppHandle,
    state_name: &str,
    message: &str,
    last_collected_at: Option<String>,
) {
    let state = app.state::<UsageCollectorState>();
    let previous = state.status().ok();
    let total_records = previous
        .as_ref()
        .map(|value| value.total_records)
        .unwrap_or(0);
    state.set_status(UsageCollectorStatus {
        state: state_name.to_string(),
        message: message.to_string(),
        last_collected_at: last_collected_at
            .or_else(|| previous.and_then(|value| value.last_collected_at)),
        total_records,
    });
}

fn persist_queue_items_from_source(
    root: &Path,
    source: &str,
    items: Vec<Value>,
    config: &GuiConfigFile,
) -> Result<UsagePersistOutcome, String> {
    let mut connection = open_usage_database_at(root)?;
    enqueue_usage_queue_items(&mut connection, source, items)?;
    process_usage_inbox(&mut connection, config)
}

fn enqueue_usage_queue_items(
    connection: &mut Connection,
    source: &str,
    items: Vec<Value>,
) -> Result<usize, String> {
    let messages = items
        .into_iter()
        .map(|item| {
            serde_json::to_string(&item)
                .map_err(|error| format!("Failed to serialize CPA usage records: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    enqueue_usage_raw_messages(connection, source, messages)
}

fn enqueue_usage_raw_messages(
    connection: &mut Connection,
    source: &str,
    messages: Vec<String>,
) -> Result<usize, String> {
    let messages = messages
        .into_iter()
        .filter(|message| !is_ignorable_usage_message(message))
        .collect::<Vec<_>>();
    if messages.is_empty() {
        return Ok(0);
    }
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed to begin SQLite usage records inbox transaction: {error}"))?;
    let mut statement = transaction
        .prepare(
            r#"
            INSERT INTO usage_inbox (
                source, message_hash, raw_message, status, attempt_count,
                received_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, 'pending', 0, ?4, ?4, ?4)
            "#,
        )
        .map_err(|error| format!("Failed to prepare SQLite usage records inbox write: {error}"))?;
    let received_at = Local::now().to_rfc3339();
    let mut inserted = 0_usize;
    for raw_message in messages {
        inserted = inserted.saturating_add(
            statement
                .execute(params![
                    source,
                    hash_text(&raw_message),
                    raw_message,
                    received_at,
                ])
                .map_err(|error| format!("Failed to write SQLite usage records inbox: {error}"))?,
        );
    }
    drop(statement);
    transaction
        .commit()
        .map_err(|error| format!("Failed to commit SQLite usage records inbox: {error}"))?;
    Ok(inserted)
}

fn is_ignorable_usage_message(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return true;
    }
    if trimmed.contains("\"request_id\"") {
        return false;
    }

    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.len() != 1 {
        return false;
    }
    object
        .get("refresh")
        .and_then(Value::as_bool)
        .is_some_and(|enabled| enabled)
        || object
            .get("support_refresh")
            .and_then(Value::as_bool)
            .is_some_and(|enabled| enabled)
}

fn persist_raw_usage_message_from_source(
    root: &Path,
    source: &str,
    raw_message: String,
    config: &GuiConfigFile,
) -> Result<UsagePersistOutcome, String> {
    persist_raw_usage_messages_from_source(root, source, vec![raw_message], config)
}

fn persist_raw_usage_messages_from_source(
    root: &Path,
    source: &str,
    raw_messages: Vec<String>,
    config: &GuiConfigFile,
) -> Result<UsagePersistOutcome, String> {
    let mut connection = open_usage_database_at(root)?;
    enqueue_usage_raw_messages(&mut connection, source, raw_messages)?;
    process_usage_inbox(&mut connection, config)
}

fn process_usage_inbox(
    connection: &mut Connection,
    config: &GuiConfigFile,
) -> Result<UsagePersistOutcome, String> {
    let rows = list_processable_usage_inbox(connection, USAGE_INBOX_PROCESS_LIMIT)?;
    if rows.is_empty() {
        return Ok(UsagePersistOutcome::default());
    }
    let mut valid_rows = Vec::with_capacity(rows.len());
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let parsed = serde_json::from_str::<Value>(&row.raw_message)
            .map_err(|error| format!("Failed to parse inbox JSON: {error}"))
            .and_then(|value| normalize_usage_record(value, config));
        match parsed {
            Ok(mut record) => {
                record.collector_source = row.source.clone();
                valid_rows.push(row);
                records.push(record);
            }
            Err(error) => mark_usage_inbox_decode_failed(connection, row.id, &error)?,
        }
    }
    if records.is_empty() {
        return Ok(UsagePersistOutcome {
            inserted: 0,
            deleted: enforce_usage_database_limit(connection)?,
        });
    }

    let persist_result = (|| -> Result<usize, String> {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("Failed to begin SQLite inbox processing transaction: {error}"))?;
        let inserted = insert_usage_records_in_transaction(&transaction, &records)?;
        let processed_at = Local::now().to_rfc3339();
        for (row, record) in valid_rows.iter().zip(records.iter()) {
            transaction
                .execute(
                    r#"
                    UPDATE usage_inbox
                    SET status = 'processed', attempt_count = attempt_count + 1,
                        last_error = '', usage_event_key = ?1,
                        processed_at = ?2, updated_at = ?2
                    WHERE id = ?3
                    "#,
                    params![record.id, processed_at, row.id],
                )
                .map_err(|error| format!("Failed to mark SQLite usage records inbox as processed: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("Failed to commit SQLite inbox processing transaction: {error}"))?;
        Ok(inserted)
    })();
    match persist_result {
        Ok(inserted) => Ok(UsagePersistOutcome {
            inserted,
            deleted: enforce_usage_database_limit(connection)?,
        }),
        Err(error) => {
            mark_usage_inbox_process_failed(connection, &valid_rows, &error)?;
            Err(error)
        }
    }
}

fn list_processable_usage_inbox(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<UsageInboxRow>, String> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT id, source, raw_message, attempt_count
            FROM usage_inbox
            WHERE status IN ('pending', 'process_failed')
              AND attempt_count < ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .map_err(|error| format!("Failed to prepare SQLite usage records inbox read: {error}"))?;
    let rows = statement
        .query_map(
            params![
                USAGE_INBOX_MAX_ATTEMPTS,
                limit.min(i64::MAX as usize) as i64
            ],
            |row| {
                Ok(UsageInboxRow {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    raw_message: row.get(2)?,
                    attempt_count: row.get(3)?,
                })
            },
        )
        .map_err(|error| format!("Failed to query SQLite usage records inbox: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Failed to read SQLite usage records inbox: {error}"))?;
    Ok(rows)
}

fn mark_usage_inbox_decode_failed(
    connection: &Connection,
    id: i64,
    error: &str,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    connection
        .execute(
            r#"
            UPDATE usage_inbox
            SET status = 'decode_failed', attempt_count = attempt_count + 1,
                last_error = ?1, processed_at = ?2, updated_at = ?2
            WHERE id = ?3
            "#,
            params![bounded_usage_inbox_error(error), now, id],
        )
        .map(|_| ())
        .map_err(|update_error| {
            format!("Error marking SQLite usage records inbox decode failure: {update_error}")
        })
}

fn mark_usage_inbox_process_failed(
    connection: &Connection,
    rows: &[UsageInboxRow],
    error: &str,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    let error = bounded_usage_inbox_error(error);
    for row in rows {
        let next_attempt = row.attempt_count.saturating_add(1);
        let status = if next_attempt >= USAGE_INBOX_MAX_ATTEMPTS {
            "discarded"
        } else {
            "process_failed"
        };
        connection
            .execute(
                r#"
                UPDATE usage_inbox
                SET status = ?1, attempt_count = ?2, last_error = ?3,
                    processed_at = CASE WHEN ?1 = 'discarded' THEN ?4 ELSE NULL END,
                    updated_at = ?4
                WHERE id = ?5
                "#,
                params![status, next_attempt, error, now, row.id],
            )
            .map_err(|update_error| {
                format!("Error marking SQLite usage records inbox processing failure: {update_error}")
            })?;
    }
    Ok(())
}

fn bounded_usage_inbox_error(error: &str) -> String {
    error.chars().take(1_000).collect()
}

fn cleanup_usage_inbox(connection: &Connection, now: DateTime<Local>) -> Result<(), String> {
    let processed_cutoff = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|value| value.and_local_timezone(Local).single())
        .unwrap_or(now)
        .to_rfc3339();
    let failed_cutoff = (now - chrono::Duration::days(7)).to_rfc3339();
    connection
        .execute(
            "DELETE FROM usage_inbox WHERE status = 'processed' AND processed_at < ?1",
            params![processed_cutoff],
        )
        .map_err(|error| format!("Failed to clean up processed usage records inbox: {error}"))?;
    connection
        .execute(
            "DELETE FROM usage_inbox WHERE status IN ('decode_failed', 'discarded') AND updated_at < ?1",
            params![failed_cutoff],
        )
        .map_err(|error| format!("Failed to clean up failed usage records inbox: {error}"))?;
    Ok(())
}

#[cfg(test)]
fn insert_usage_records(
    connection: &mut Connection,
    records: &[UsageRecord],
) -> Result<usize, String> {
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed to begin SQLite usage records transaction: {error}"))?;
    let inserted = insert_usage_records_in_transaction(&transaction, records)?;
    transaction
        .commit()
        .map_err(|error| format!("Failed to commit SQLite usage records: {error}"))?;
    Ok(inserted)
}

fn insert_usage_records_in_transaction(
    transaction: &Transaction<'_>,
    records: &[UsageRecord],
) -> Result<usize, String> {
    if records.is_empty() {
        return Ok(0);
    }
    let mut statement = transaction
        .prepare(
            r#"
            INSERT INTO usage_events (
                event_key, timestamp, timestamp_ms, local_hour, latency_ms, ttft_ms,
                source, auth_index, failed, provider, model, alias, reasoning_effort,
                service_tier, response_service_tier, executor_type, endpoint, auth_type,
                api_key_hash, api_key_display, api_key_remark, request_id,
                api_group_key, model_alias, client_ip, x_forwarded_for, user_agent,
                generate, cached_tokens, collector_source,
                input_tokens, output_tokens, reasoning_tokens, cache_read_tokens,
                cache_creation_tokens, total_tokens, canceled, failure_status,
                failure_body, created_at, response_model
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30,
                ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40, ?41
            )
            "#,
        )
        .map_err(|error| format!("Failed to prepare SQLite usage records write: {error}"))?;
    let created_at = Local::now().to_rfc3339();
    let mut inserted = 0_usize;
    for record in records {
        let api_group_key = if !record.api_group_key.trim().is_empty() {
            record.api_group_key.as_str()
        } else if !record.api_key_hash.trim().is_empty() {
            record.api_key_hash.as_str()
        } else if !record.provider.trim().is_empty() {
            record.provider.as_str()
        } else if !record.endpoint.trim().is_empty() {
            record.endpoint.as_str()
        } else {
            "unknown"
        };
        let collector_source = if record.collector_source.trim().is_empty() {
            "legacy_json"
        } else {
            record.collector_source.as_str()
        };
        inserted = inserted.saturating_add(
            statement
                .execute(params![
                    record.id,
                    record.timestamp,
                    record_timestamp_millis(record),
                    record_local_hour(record),
                    to_sql_i64(record.latency_ms),
                    record.ttft_ms.map(to_sql_i64),
                    record.source,
                    record.auth_index,
                    record.failed,
                    record.provider,
                    record.model,
                    record.alias,
                    record.reasoning_effort,
                    record.service_tier,
                    record.response_service_tier,
                    record.executor_type,
                    record.endpoint,
                    record.auth_type,
                    record.api_key_hash,
                    record.api_key_display,
                    record.api_key_remark,
                    record.request_id,
                    api_group_key,
                    if record.alias.trim().is_empty() {
                        None::<&str>
                    } else {
                        Some(record.alias.as_str())
                    },
                    record.client_ip,
                    record.x_forwarded_for,
                    record.user_agent,
                    record.generate,
                    to_sql_i64(record.cached_tokens),
                    collector_source,
                    to_sql_i64(record.tokens.input_tokens),
                    to_sql_i64(record.tokens.output_tokens),
                    to_sql_i64(record.tokens.reasoning_tokens),
                    to_sql_i64(record.tokens.cache_read_tokens),
                    to_sql_i64(record.tokens.cache_creation_tokens),
                    to_sql_i64(record.tokens.total_tokens),
                    record.canceled,
                    i64::from(record.failure_status),
                    record.failure_body,
                    created_at,
                    record.response_model,
                ])
                .map_err(|error| format!("Failed to write SQLite usage records: {error}"))?,
        );
    }
    Ok(inserted)
}

fn normalize_usage_record(value: Value, config: &GuiConfigFile) -> Result<UsageRecord, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "CPA usage record must be a JSON object".to_string())?;
    let timestamp = string_field(object, "timestamp")
        .filter(|value| DateTime::parse_from_rfc3339(value).is_ok())
        .unwrap_or_else(|| Local::now().to_rfc3339());
    let request_id = string_field(object, "request_id")
        .ok_or_else(|| "CPA usage record must contain request_id".to_string())?;
    let api_key = string_field(object, "api_key").unwrap_or_default();
    let api_key_hash = hash_text(&api_key);
    let api_key_remark = config
        .api_keys
        .iter()
        .find(|entry| entry.key == api_key)
        .map(|entry| entry.remark.clone())
        .unwrap_or_default();
    let provider = string_field(object, "provider").unwrap_or_default();
    let executor_type = string_field(object, "executor_type").unwrap_or_default();
    let auth_type = string_field(object, "auth_type").unwrap_or_default();
    let tokens_object = object.get("tokens").and_then(Value::as_object);
    let cache_read_present = tokens_object
        .and_then(|tokens| tokens.get("cache_read_tokens"))
        .is_some();
    let raw_cache_read_tokens = token_u64(tokens_object, "cache_read_tokens");
    let cache_creation_tokens = token_u64(tokens_object, "cache_creation_tokens");
    let raw_cached_tokens =
        token_u64(tokens_object, "cached_tokens").max(token_u64(tokens_object, "cache_tokens"));
    let normalized_tokens = normalize_keeper_tokens(
        &executor_type,
        &provider,
        &auth_type,
        KeeperTokenValues {
            input: token_u64(tokens_object, "input_tokens"),
            output: token_u64(tokens_object, "output_tokens"),
            reasoning: token_u64(tokens_object, "reasoning_tokens"),
            cached: raw_cached_tokens,
            cache_read: raw_cache_read_tokens,
            cache_read_present,
            cache_creation: cache_creation_tokens,
            total: token_u64(tokens_object, "total_tokens"),
            clamped: ClampedTokenFields {
                input: token_was_negative(tokens_object, "input_tokens"),
                output: token_was_negative(tokens_object, "output_tokens"),
                reasoning: token_was_negative(tokens_object, "reasoning_tokens"),
                cached: token_was_negative(tokens_object, "cached_tokens")
                    || token_was_negative(tokens_object, "cache_tokens"),
                cache_read: token_was_negative(tokens_object, "cache_read_tokens"),
                cache_creation: token_was_negative(tokens_object, "cache_creation_tokens"),
                total: token_was_negative(tokens_object, "total_tokens"),
            },
        },
    );
    let tokens = UsageTokenStats {
        input_tokens: normalized_tokens.input,
        output_tokens: normalized_tokens.output,
        reasoning_tokens: normalized_tokens.reasoning,
        cache_read_tokens: normalized_tokens.cache_read,
        cache_creation_tokens: normalized_tokens.cache_creation,
        total_tokens: normalized_tokens.total,
    };
    let id = request_id.clone();
    let endpoint = string_field(object, "endpoint").unwrap_or_default();
    let failed = object
        .get("failed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (failure_status, failure_body) = usage_failure_details(object);
    let canceled = failed && usage_failure_is_canceled(failure_status, &failure_body);
    let generate = object
        .get("generate")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            !(executor_type == "CodexWebsocketsExecutor"
                && !failed
                && tokens.input_tokens == 0
                && tokens.output_tokens == 0
                && tokens.reasoning_tokens == 0
                && tokens.cache_read_tokens == 0
                && tokens.cache_creation_tokens == 0
                && tokens.total_tokens == 0)
        });
    let api_group_key = if api_key_hash.is_empty() {
        if !provider.is_empty() {
            provider.clone()
        } else if !endpoint.is_empty() {
            endpoint.clone()
        } else {
            "unknown".to_string()
        }
    } else {
        api_key_hash.clone()
    };
    Ok(UsageRecord {
        id,
        row_id: None,
        timestamp,
        latency_ms: u64_field(object, "latency_ms"),
        ttft_ms: optional_u64_field(object, "ttft_ms"),
        source: string_field(object, "source").unwrap_or_default(),
        source_display: String::new(),
        auth_index: string_field(object, "auth_index").unwrap_or_default(),
        failed,
        canceled,
        failure_status,
        failure_body,
        provider,
        api_group_key,
        model: string_field(object, "resolved_model")
            .or_else(|| string_field(object, "resolvedModel"))
            .or_else(|| string_field(object, "model"))
            .unwrap_or_else(|| "unknown".to_string()),
        response_model: string_field(object, "response_model")
            .or_else(|| string_field(object, "responseModel")).unwrap_or_default(),
        cost: None,
        alias: string_field(object, "alias")
            .or_else(|| string_field(object, "requested_model"))
            .or_else(|| string_field(object, "requestedModel")).unwrap_or_default(),
        client_ip: string_field(object, "client_ip"),
        x_forwarded_for: string_field(object, "x_forwarded_for"),
        user_agent: string_field(object, "user_agent"),
        reasoning_effort: string_field(object, "reasoning_effort").unwrap_or_default(),
        service_tier: string_field(object, "service_tier").unwrap_or_default(),
        response_service_tier: string_field(object, "response_service_tier").unwrap_or_default(),
        executor_type,
        endpoint,
        auth_type,
        api_key_hash,
        api_key_display: mask_api_key(&api_key),
        api_key_remark,
        request_id,
        generate,
        cached_tokens: normalized_tokens.cached,
        collector_source: HTTP_USAGE_QUEUE_SOURCE.to_string(),
        tokens,
    })
}

fn usage_failure_body(value: &Value) -> String {
    let body = match value {
        Value::String(value) => value.trim().to_string(),
        Value::Null => String::new(),
        value => serde_json::to_string(value).unwrap_or_default(),
    };
    if body.chars().count() <= MAX_USAGE_FAILURE_BODY_CHARS {
        body
    } else {
        let mut bounded = body
            .chars()
            .take(MAX_USAGE_FAILURE_BODY_CHARS)
            .collect::<String>();
        bounded.push('…');
        bounded
    }
}

fn usage_failure_details(object: &serde_json::Map<String, Value>) -> (u16, String) {
    let failure = object.get("fail").and_then(Value::as_object);
    let status = failure
        .and_then(|value| value.get("status_code").or_else(|| value.get("statusCode")))
        .and_then(Value::as_u64)
        .unwrap_or_default()
        .min(u16::MAX as u64) as u16;
    let body = failure
        .and_then(|value| value.get("body"))
        .map(usage_failure_body)
        .unwrap_or_default();
    (status, body)
}

fn usage_failure_is_canceled(status: u16, body: &str) -> bool {
    if status == 499 {
        return true;
    }
    let body = body.to_ascii_lowercase();
    body.contains("context canceled") || body.contains("client closed request")
}

fn default_usage_generate() -> bool {
    true
}

fn build_usage_filter(query: &UsageQuery) -> UsageSqlFilter {
    let mut clauses = Vec::<String>::new();
    let mut params = Vec::<SqlValue>::new();
    if let Some(start) = query.start.as_deref().and_then(parse_timestamp_millis) {
        clauses.push("timestamp_ms >= ?".to_string());
        params.push(SqlValue::Integer(start));
    }
    if let Some(end) = query.end.as_deref().and_then(parse_timestamp_millis) {
        clauses.push("timestamp_ms <= ?".to_string());
        params.push(SqlValue::Integer(end));
    }
    add_text_filter(
        &mut clauses,
        &mut params,
        USAGE_DISPLAY_MODEL_SQL,
        query.model.as_deref(),
    );
    add_text_filter(
        &mut clauses,
        &mut params,
        "provider",
        query.provider.as_deref(),
    );
    add_text_filter(&mut clauses, &mut params, "source", query.source.as_deref());
    add_text_filter(
        &mut clauses,
        &mut params,
        "api_key_hash",
        query.api_key_hash.as_deref(),
    );
    if let Some(canceled) = query.canceled {
        clauses.push("canceled = ?".to_string());
        params.push(SqlValue::Integer(i64::from(canceled)));
    }
    if let Some(failed) = query.failed {
        if failed {
            clauses.push("failed != 0 AND canceled = 0".to_string());
        } else {
            clauses.push("failed = 0".to_string());
        }
    }
    UsageSqlFilter {
        clause: if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        },
        params,
    }
}

fn add_text_filter(
    clauses: &mut Vec<String>,
    params: &mut Vec<SqlValue>,
    column: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        clauses.push(format!("{column} = ? COLLATE NOCASE"));
        params.push(SqlValue::Text(value.to_string()));
    }
}

#[tauri::command]
pub(crate) fn get_usage_collector_status(
    state: tauri::State<'_, UsageCollectorState>,
) -> Result<UsageCollectorStatus, String> {
    state.status()
}

#[tauri::command]
pub(crate) async fn get_usage_overview(query: UsageQuery) -> Result<UsageOverview, String> {
    run_usage_task(move || load_usage_overview(&open_usage_database()?, &query)).await
}

fn load_usage_overview(
    connection: &Connection,
    query: &UsageQuery,
) -> Result<UsageOverview, String> {
    let filter = build_usage_filter(query);
    let summary_sql = format!(
        r#"
        SELECT
            COUNT(*),
            COALESCE(SUM(CASE WHEN failed = 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN failed != 0 AND canceled = 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN canceled != 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(input_tokens), 0),
            COALESCE(SUM(output_tokens), 0),
            COALESCE(SUM(reasoning_tokens), 0),
            COALESCE(SUM(cache_read_tokens), 0),
            COALESCE(SUM(cache_creation_tokens), 0),
            COALESCE(SUM(total_tokens), 0),
            COALESCE(SUM(latency_ms), 0),
            COALESCE(AVG(CASE
                WHEN generate != 0
                 AND failed = 0
                 AND canceled = 0
                 AND output_tokens > 0
                 AND latency_ms > 0
                THEN output_tokens * 1000.0 / latency_ms
            END), 0.0),
            COALESCE(SUM(CASE
                WHEN generate != 0
                 AND failed = 0
                 AND canceled = 0
                 AND output_tokens > 0
                 AND latency_ms > 0
                THEN 1
                ELSE 0
            END), 0),
            MIN(timestamp_ms),
            MAX(timestamp_ms)
        FROM usage_events{}
        "#,
        filter.clause
    );
    let summary = connection
        .query_row(
            &summary_sql,
            params_from_iter(filter.params.iter()),
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, f64>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, Option<i64>>(13)?,
                    row.get::<_, Option<i64>>(14)?,
                ))
            },
        )
        .map_err(|error| format!("Failed to count SQLite usage records: {error}"))?;

    let (estimated_cost, priced_requests) = load_estimated_cost(connection, &filter)?;

    let timeline_sql = format!(
        r#"
        SELECT
            CASE
                WHEN timestamp_ms > 0 THEN
                    strftime('%Y-%m-%d-%H-', datetime(timestamp_ms / 1000, 'unixepoch', 'localtime'))
                    || CASE
                        WHEN CAST(strftime('%M', datetime(timestamp_ms / 1000, 'unixepoch', 'localtime')) AS INTEGER) < 30
                        THEN '00'
                        ELSE '30'
                    END
                ELSE local_hour || '-00'
            END,
            {USAGE_DISPLAY_MODEL_SQL},
            COUNT(*),
            COALESCE(SUM(CASE WHEN failed = 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN failed != 0 AND canceled = 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN canceled != 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(total_tokens), 0)
        FROM usage_events{}
        GROUP BY 1, 2
        ORDER BY 1 ASC, 2 ASC
        "#,
        filter.clause
    );
    let mut statement = connection
        .prepare(&timeline_sql)
        .map_err(|error| format!("Failed to prepare SQLite usage trend query: {error}"))?;
    let timeline_rows = statement
        .query_map(params_from_iter(filter.params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                from_sql_i64(row.get(2)?),
                from_sql_i64(row.get(3)?),
                from_sql_i64(row.get(4)?),
                from_sql_i64(row.get(5)?),
                from_sql_i64(row.get(6)?),
            ))
        })
        .map_err(|error| format!("Failed to query SQLite usage trends: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Failed to read SQLite usage trends: {error}"))?;
    let mut grouped = BTreeMap::<String, UsageTimelinePoint>::new();
    for (hour, model, requests, success, failure, canceled, tokens) in timeline_rows {
        let point = grouped.entry(hour.clone()).or_insert_with(|| UsageTimelinePoint {
            hour,
            ..UsageTimelinePoint::default()
        });
        point.requests = point.requests.saturating_add(requests);
        point.success = point.success.saturating_add(success);
        point.failure = point.failure.saturating_add(failure);
        point.canceled = point.canceled.saturating_add(canceled);
        point.tokens = point.tokens.saturating_add(tokens);
        if let Some(existing) = point.models.iter_mut().find(|item| item.key == model) {
            existing.requests = existing.requests.saturating_add(requests);
            existing.tokens = existing.tokens.saturating_add(tokens);
        } else {
            point.models.push(UsageTimelineModel {
                key: model.clone(),
                label: model,
                tokens,
                requests,
            });
        }
    }
    let timeline = grouped
        .into_values()
        .map(|mut point| {
            point.models.sort_by(|left, right| {
                right
                    .tokens
                    .cmp(&left.tokens)
                    .then_with(|| left.key.cmp(&right.key))
            });
            point
        })
        .collect::<Vec<_>>();

    let mut overview = UsageOverview {
        total_requests: from_sql_i64(summary.0),
        success_count: from_sql_i64(summary.1),
        failure_count: from_sql_i64(summary.2),
        canceled_count: from_sql_i64(summary.3),
        input_tokens: from_sql_i64(summary.4),
        output_tokens: from_sql_i64(summary.5),
        reasoning_tokens: from_sql_i64(summary.6),
        cache_read_tokens: from_sql_i64(summary.7),
        cache_creation_tokens: from_sql_i64(summary.8),
        total_tokens: from_sql_i64(summary.9),
        estimated_cost,
        priced_requests,
        timeline,
        ..UsageOverview::default()
    };
    if overview.total_requests > 0 {
        let completed_requests = overview
            .success_count
            .saturating_add(overview.failure_count);
        if completed_requests > 0 {
            overview.success_rate =
                overview.success_count as f64 * 100.0 / completed_requests as f64;
        }
        overview.average_latency_ms =
            from_sql_i64(summary.10) as f64 / overview.total_requests as f64;
        overview.tps = summary.11;
        overview.tps_sample_count = from_sql_i64(summary.12);
        if overview.input_tokens > 0 {
            overview.cache_hit_rate =
                (overview.cache_read_tokens as f64 / overview.input_tokens as f64).min(1.0);
        }
        let minutes = query_window_minutes(query, summary.13, summary.14);
        overview.rpm = overview.total_requests as f64 / minutes;
        overview.tpm = overview.total_tokens as f64 / minutes;
    }
    Ok(overview)
}

fn load_estimated_cost(
    connection: &Connection,
    filter: &UsageSqlFilter,
) -> Result<(f64, u64), String> {
    let prices = load_model_prices(connection)?;
    let groups = load_usage_cost_groups(connection, filter)?;
    Ok(sum_usage_cost(&groups, &prices))
}

fn load_usage_cost_groups(
    connection: &Connection,
    filter: &UsageSqlFilter,
) -> Result<Vec<UsageCostGroup>, String> {
    let long_context_sql = format!(
        "input_tokens > {LONG_CONTEXT_INPUT_TOKEN_THRESHOLD} OR (input_tokens > {GROK_47_LONG_CONTEXT_INPUT_TOKEN_THRESHOLD} AND {GROK_47_MODEL_SQL})"
    );
    let sql = format!(
        r#"
        SELECT
            model,
            alias,
            service_tier,
            response_service_tier,
            executor_type,
            provider,
            auth_type,
            COUNT(*),
            COALESCE(SUM(input_tokens), 0),
            COALESCE(SUM(output_tokens), 0),
            COALESCE(SUM(cache_read_tokens), 0),
            COALESCE(SUM(cache_creation_tokens), 0),
            COALESCE(SUM(CASE WHEN ({long_context_sql}) THEN input_tokens ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN ({long_context_sql}) THEN output_tokens ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN ({long_context_sql}) THEN cache_read_tokens ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN ({long_context_sql}) THEN cache_creation_tokens ELSE 0 END), 0),
            COALESCE(SUM(total_tokens), 0)
        FROM usage_events{}
        GROUP BY model, alias, service_tier, response_service_tier, executor_type, provider, auth_type
        "#,
        filter.clause
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Failed to prepare usage cost query: {error}"))?;
    let groups = statement
        .query_map(params_from_iter(filter.params.iter()), |row| {
            Ok(UsageCostGroup {
                model: row.get(0)?,
                alias: row.get(1)?,
                service_tier: row.get(2)?,
                response_service_tier: row.get(3)?,
                executor_type: row.get(4)?,
                provider: row.get(5)?,
                auth_type: row.get(6)?,
                requests: from_sql_i64(row.get(7)?),
                tokens: CostTokens {
                    input: from_sql_i64(row.get(8)?),
                    output: from_sql_i64(row.get(9)?),
                    cache_read: from_sql_i64(row.get(10)?),
                    cache_creation: from_sql_i64(row.get(11)?),
                    long_input: from_sql_i64(row.get(12)?),
                    long_output: from_sql_i64(row.get(13)?),
                    long_cache_read: from_sql_i64(row.get(14)?),
                    long_cache_creation: from_sql_i64(row.get(15)?),
                },
                total_tokens: from_sql_i64(row.get(16)?),
            })
        })
        .map_err(|error| format!("Failed to query usage costs: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Failed to read usage costs: {error}"))?;
    Ok(groups)
}

fn sum_usage_cost(groups: &[UsageCostGroup], prices: &HashMap<String, ModelPrice>) -> (f64, u64) {
    let mut total_cost = 0.0;
    let mut priced_requests = 0_u64;
    for group in groups {
        let Some((model, price)) = resolve_model_price(&group.model, &group.alias, prices) else {
            continue;
        };
        let identity = format!(
            "{} {} {}",
            group.executor_type, group.provider, group.auth_type
        )
        .to_ascii_lowercase();
        let service_tier =
            if identity.contains("codex") || group.response_service_tier.trim().is_empty() {
                group.service_tier.as_str()
            } else {
                group.response_service_tier.as_str()
            };
        total_cost += cost_for_price(model, service_tier, &group.tokens, &price);
        priced_requests = priced_requests.saturating_add(group.requests);
    }
    (total_cost, priced_requests)
}

fn cost_for_price(model: &str, service_tier: &str, tokens: &CostTokens, price: &ModelPrice) -> f64 {
    let price = enriched_model_price(model, price);
    let long_output_multiplier = if is_model_family(model, "grok-4.7") {
        2.0
    } else {
        1.5
    };
    let short_cost = cost_for_token_segment(
        tokens.input.saturating_sub(tokens.long_input),
        tokens.output.saturating_sub(tokens.long_output),
        tokens.cache_read.saturating_sub(tokens.long_cache_read),
        tokens
            .cache_creation
            .saturating_sub(tokens.long_cache_creation),
        &price,
        1.0,
        1.0,
    );
    let long_cost = cost_for_token_segment(
        tokens.long_input,
        tokens.long_output,
        tokens.long_cache_read,
        tokens.long_cache_creation,
        &price,
        2.0,
        long_output_multiplier,
    );
    let tier = service_tier.trim().to_ascii_lowercase();
    let multiplier = match tier.as_str() {
        "flex" | "batch" => 0.5,
        "priority" | "fast" => service_tier_multiplier(model),
        _ => 1.0,
    };
    // A grouped total can contain both context lengths. Long-context overrides
    // must not suppress the service-tier multiplier on its short requests.
    let long_multiplier = if matches!(tier.as_str(), "priority" | "fast")
        && !is_model_family(model, "gpt-6")
    {
        1.0
    } else {
        multiplier
    };
    short_cost * multiplier + long_cost * long_multiplier
}

fn usage_record_cost(record: &UsageRecord, prices: &HashMap<String, ModelPrice>) -> Option<UsageRecordCost> {
    let (model, price) = resolve_model_price(&record.model, &record.alias, prices)?;
    let input = record.tokens.input_tokens;
    // Keep the same context buckets and tier selection as the aggregate pricing query.
    let long = input > LONG_CONTEXT_INPUT_TOKEN_THRESHOLD
        || (input > GROK_47_LONG_CONTEXT_INPUT_TOKEN_THRESHOLD && is_model_family(model, "grok-4.7"));
    let tokens = CostTokens {
        input,
        output: record.tokens.output_tokens,
        cache_read: record.tokens.cache_read_tokens,
        cache_creation: record.tokens.cache_creation_tokens,
        long_input: if long { input } else { 0 },
        long_output: if long { record.tokens.output_tokens } else { 0 },
        long_cache_read: if long { record.tokens.cache_read_tokens } else { 0 },
        long_cache_creation: if long { record.tokens.cache_creation_tokens } else { 0 },
    };
    let identity = format!("{} {} {}", record.executor_type, record.provider, record.auth_type).to_ascii_lowercase();
    let tier = if identity.contains("codex") || record.response_service_tier.trim().is_empty() {
        &record.service_tier
    } else {
        &record.response_service_tier
    };
    Some(UsageRecordCost {
        total: cost_for_price(model, tier, &tokens, &price),
        pricing_model: price.model,
    })
}

fn cost_for_token_segment(
    input: u64,
    output: u64,
    cache_read: u64,
    cache_creation: u64,
    price: &ModelPrice,
    input_multiplier: f64,
    output_multiplier: f64,
) -> f64 {
    let prompt = input.saturating_sub(cache_read.saturating_add(cache_creation));
    ((prompt as f64 * price.prompt
        + cache_read as f64 * price.cache_read
        + cache_creation as f64 * price.cache_creation)
        * input_multiplier
        + output as f64 * price.completion * output_multiplier)
        / TOKENS_PER_PRICE_UNIT
}

fn official_model_price(model: &str) -> Option<ModelPrice> {
    find_model_price(cached_bundled_model_prices().ok()?, model).cloned()
}

fn bundled_model_prices() -> Result<HashMap<String, ModelPrice>, String> {
    cached_bundled_model_prices().cloned()
}

fn cached_bundled_model_prices() -> Result<&'static HashMap<String, ModelPrice>, String> {
    static PRICES: LazyLock<Result<HashMap<String, ModelPrice>, String>> =
        LazyLock::new(|| parse_model_price_catalog(BUNDLED_MODEL_PRICE_CATALOG, "builtin", 0));
    PRICES.as_ref().map_err(Clone::clone)
}

fn parse_model_price_catalog(
    content: &str,
    source: &str,
    updated_at_ms: i64,
) -> Result<HashMap<String, ModelPrice>, String> {
    let catalog = serde_json::from_str::<ModelPriceCatalog>(content)
        .map_err(|error| format!("Failed to parse model pricing file: {error}"))?;
    if catalog.schema_version != 1 {
        return Err(format!(
            "Unsupported model pricing file version {}",
            catalog.schema_version
        ));
    }
    if catalog.models.is_empty() {
        return Err("Model pricing file contains no models".to_string());
    }
    let _catalog_updated_at = catalog.updated_at;
    let mut prices = HashMap::with_capacity(catalog.models.len());
    for (model, entry) in catalog.models {
        let cache_read = entry.cache_read_per_1_m.unwrap_or(0.0);
        let cache_creation = entry.cache_creation_per_1_m.unwrap_or(0.0);
        let price = ModelPrice {
            model: model.trim().to_string(),
            prompt: entry.input_per_1_m,
            completion: entry.output_per_1_m,
            cache_read,
            cache_creation,
            prompt_configured: true,
            completion_configured: true,
            cache_read_configured: entry.cache_read_per_1_m.is_some(),
            cache_creation_configured: entry.cache_creation_per_1_m.is_some(),
            source: source.to_string(),
            source_model_id: String::new(),
            updated_at_ms,
        };
        validate_model_price(&price)?;
        prices.insert(price.model.clone(), price);
    }
    Ok(prices)
}

fn parse_models_dev_prices(
    content: &str,
    updated_at_ms: i64,
) -> Result<Vec<ModelPrice>, String> {
    let catalog: Value = serde_json::from_str(content)
        .map_err(|error| format!("Failed to parse models.dev pricing: {error}"))?;
    let providers = catalog
        .get("providers")
        .and_then(Value::as_object)
        .or_else(|| catalog.as_object())
        .ok_or("Invalid models.dev pricing catalog format")?;
    let mut prices = Vec::new();
    for (provider, entry) in providers {
        let Some(models) = entry
            .get("models")
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (model_id, entry) in models {
            let Some(cost) = entry.get("cost") else {
                continue;
            };
            let (Some(prompt), Some(completion)) = (
                cost.get("input").and_then(Value::as_f64),
                cost.get("output").and_then(Value::as_f64),
            ) else {
                continue;
            };
            let cache_read = cost.get("cache_read").and_then(Value::as_f64);
            let cache_creation = cost.get("cache_write").and_then(Value::as_f64);
            let price = ModelPrice {
                model: model_id.trim().to_string(),
                prompt,
                completion,
                cache_read: cache_read.unwrap_or(0.0),
                cache_creation: cache_creation.unwrap_or(0.0),
                prompt_configured: true,
                completion_configured: true,
                cache_read_configured: cache_read.is_some(),
                cache_creation_configured: cache_creation.is_some(),
                source: "models.dev".to_string(),
                source_model_id: format!("{provider}/{model_id}"),
                updated_at_ms,
            };
            validate_model_price(&price)?;
            prices.push(price);
        }
    }
    if prices.is_empty() {
        return Err("models.dev has no available model pricing".to_string());
    }
    Ok(prices)
}

fn parse_litellm_prices(content: &str, updated_at_ms: i64) -> Result<Vec<ModelPrice>, String> {
    let catalog: Value = serde_json::from_str(content)
        .map_err(|error| format!("Failed to parse LiteLLM pricing: {error}"))?;
    let models = catalog.as_object().ok_or("Invalid LiteLLM pricing catalog format")?;
    let mut prices = Vec::new();
    for (model_id, entry) in models {
        let mode = entry.get("mode").and_then(Value::as_str).unwrap_or_default();
        if !matches!(mode, "chat" | "completion" | "responses") {
            continue;
        }
        if let Some(modalities) = entry.get("supported_output_modalities").and_then(Value::as_array) {
            if !modalities.iter().any(|value| value.as_str() == Some("text")) {
                continue;
            }
        }
        let (Some(prompt), Some(completion)) = (
            entry.get("input_cost_per_token").and_then(Value::as_f64),
            entry.get("output_cost_per_token").and_then(Value::as_f64),
        ) else {
            continue;
        };
        let cache_read = entry
            .get("cache_read_input_token_cost")
            .and_then(Value::as_f64)
            .or_else(|| entry.get("input_cost_per_token_cache_hit").and_then(Value::as_f64));
        let cache_creation = entry
            .get("cache_creation_input_token_cost")
            .and_then(Value::as_f64);
        let price = ModelPrice {
            model: model_id.trim().to_string(),
            prompt: prompt * TOKENS_PER_PRICE_UNIT,
            completion: completion * TOKENS_PER_PRICE_UNIT,
            cache_read: cache_read.unwrap_or(0.0) * TOKENS_PER_PRICE_UNIT,
            cache_creation: cache_creation.unwrap_or(0.0) * TOKENS_PER_PRICE_UNIT,
            prompt_configured: true,
            completion_configured: true,
            cache_read_configured: cache_read.is_some(),
            cache_creation_configured: cache_creation.is_some(),
            source: "litellm-sync".to_string(),
            source_model_id: model_id.clone(),
            updated_at_ms,
        };
        if validate_model_price(&price).is_ok() {
            prices.push(price);
        }
    }
    if prices.is_empty() {
        return Err("LiteLLM has no available model pricing".to_string());
    }
    Ok(prices)
}

fn find_model_price<'a>(
    prices: &'a HashMap<String, ModelPrice>,
    model: &str,
) -> Option<&'a ModelPrice> {
    if let Some(price) = prices.get(model) {
        return Some(price);
    }
    let case_insensitive = prices
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(model))
        .collect::<Vec<_>>();
    if case_insensitive.len() == 1 {
        return Some(case_insensitive[0].1);
    }
    let tail = canonical_model_tail(model);
    let exact_tail = prices
        .iter()
        .filter(|(key, _)| canonical_model_tail(key) == tail)
        .collect::<Vec<_>>();
    if exact_tail.len() == 1 {
        return Some(exact_tail[0].1);
    }
    let normalized_tail = normalized_model_tail(model);
    prices
        .iter()
        .filter_map(|(key, price)| {
            let key_tail = normalized_model_tail(key);
            normalized_tail
                .starts_with(&format!("{key_tail}-"))
                .then_some((key_tail.len(), price))
        })
        .max_by_key(|(length, _)| *length)
        .map(|(_, price)| price)
}

fn resolve_model_price<'a>(
    model: &'a str,
    alias: &'a str,
    prices: &HashMap<String, ModelPrice>,
) -> Option<(&'a str, ModelPrice)> {
    for candidate in [model, alias] {
        if candidate.trim().is_empty() {
            continue;
        }
        if let Some(price) = find_model_price(prices, candidate) {
            return Some((model, price.clone()));
        }
    }
    None
}

fn enriched_model_price(model: &str, price: &ModelPrice) -> ModelPrice {
    let mut price = price.clone();
    if let Some(official) = official_model_price(model) {
        if !price.prompt_configured && price.prompt <= 0.0 {
            price.prompt = official.prompt;
        }
        if !price.completion_configured && price.completion <= 0.0 {
            price.completion = official.completion;
        }
    }
    price
}

fn is_model_family(model: &str, family: &str) -> bool {
    let normalized = model
        .trim()
        .to_ascii_lowercase()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string();
    normalized == family || normalized.starts_with(&format!("{family}-"))
}

fn service_tier_multiplier(model: &str) -> f64 {
    if is_model_family(model, "gpt-5.5") {
        2.5
    } else if is_model_family(model, "gpt-6")
        || is_model_family(model, "gpt-5.6")
        || is_model_family(model, "gpt-5.4")
        || is_model_family(model, "gpt-5.4-mini")
        || is_model_family(model, "gpt-5.3-codex")
    {
        2.0
    } else {
        1.0
    }
}

fn load_model_prices(connection: &Connection) -> Result<HashMap<String, ModelPrice>, String> {
    let mut merged = bundled_model_prices()?;
    let mut statement = connection
        .prepare(
            r#"
            SELECT model, prompt_per_1m, completion_per_1m,
                   cache_read_per_1m, cache_creation_per_1m,
                   prompt_configured, completion_configured,
                   cache_read_configured, cache_creation_configured,
                   source, source_model_id, updated_at_ms
            FROM model_prices ORDER BY model
            "#,
        )
        .map_err(|error| format!("Failed to prepare model pricing query: {error}"))?;
    let prices = statement
        .query_map([], |row| {
            Ok(ModelPrice {
                model: row.get(0)?,
                prompt: row.get(1)?,
                completion: row.get(2)?,
                cache_read: row.get(3)?,
                cache_creation: row.get(4)?,
                prompt_configured: row.get(5)?,
                completion_configured: row.get(6)?,
                cache_read_configured: row.get(7)?,
                cache_creation_configured: row.get(8)?,
                source: row.get(9)?,
                source_model_id: row.get(10)?,
                updated_at_ms: row.get(11)?,
            })
        })
        .map_err(|error| format!("Failed to query model pricing: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Failed to read model pricing: {error}"))?;
    for price in prices {
        if price.source != "litellm" {
            if let Some(existing) = merged
                .keys()
                .find(|model| model.eq_ignore_ascii_case(&price.model))
                .cloned()
            {
                merged.remove(&existing);
            }
            merged.insert(price.model.clone(), price);
        }
    }
    Ok(merged)
}

fn validate_model_price(price: &ModelPrice) -> Result<(), String> {
    if price.model.trim().is_empty() {
        return Err("Model name cannot be empty".to_string());
    }
    for value in [
        price.prompt,
        price.completion,
        price.cache_read,
        price.cache_creation,
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(format!("Model {} contains invalid pricing", price.model));
        }
    }
    Ok(())
}

fn upsert_model_price(connection: &Connection, price: &ModelPrice) -> Result<(), String> {
    validate_model_price(price)?;
    connection
        .execute(
            r#"
            INSERT INTO model_prices (
                model, prompt_per_1m, completion_per_1m,
                cache_read_per_1m, cache_creation_per_1m,
                prompt_configured, completion_configured,
                cache_read_configured, cache_creation_configured,
                source, source_model_id, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(model) DO UPDATE SET
                prompt_per_1m = excluded.prompt_per_1m,
                completion_per_1m = excluded.completion_per_1m,
                cache_per_1m = 0,
                cache_read_per_1m = excluded.cache_read_per_1m,
                cache_creation_per_1m = excluded.cache_creation_per_1m,
                prompt_configured = excluded.prompt_configured,
                completion_configured = excluded.completion_configured,
                cache_read_configured = excluded.cache_read_configured,
                cache_creation_configured = excluded.cache_creation_configured,
                source = excluded.source,
                source_model_id = excluded.source_model_id,
                updated_at_ms = excluded.updated_at_ms
            "#,
            params![
                price.model.trim(),
                price.prompt,
                price.completion,
                price.cache_read,
                price.cache_creation,
                price.prompt_configured,
                price.completion_configured,
                price.cache_read_configured,
                price.cache_creation_configured,
                price.source,
                price.source_model_id,
                price.updated_at_ms,
            ],
        )
        .map_err(|error| format!("Failed to save model pricing: {error}"))?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn get_usage_pricing(query: UsageQuery) -> Result<UsagePricing, String> {
    run_usage_task(move || load_usage_pricing(&open_usage_database()?, &query)).await
}

fn load_usage_pricing(connection: &Connection, query: &UsageQuery) -> Result<UsagePricing, String> {
    let filter = build_usage_filter(query);
    let prices = load_model_prices(connection)?;
    let groups = load_usage_cost_groups(connection, &filter)?;
    let mut rows = HashMap::<String, UsagePriceRow>::new();
    let mut total_requests = 0_u64;
    let mut priced_requests = 0_u64;
    let mut total_cost = 0.0;
    for group in &groups {
        total_requests = total_requests.saturating_add(group.requests);
        let entry = rows
            .entry(group.model.clone())
            .or_insert_with(|| UsagePriceRow {
                model: group.model.clone(),
                ..UsagePriceRow::default()
            });
        entry.requests = entry.requests.saturating_add(group.requests);
        entry.input_tokens = entry.input_tokens.saturating_add(group.tokens.input);
        entry.output_tokens = entry.output_tokens.saturating_add(group.tokens.output);
        entry.cache_read_tokens = entry
            .cache_read_tokens
            .saturating_add(group.tokens.cache_read);
        entry.cache_creation_tokens = entry
            .cache_creation_tokens
            .saturating_add(group.tokens.cache_creation);
        entry.total_tokens = entry.total_tokens.saturating_add(group.total_tokens);

        if let Some((model, price)) = resolve_model_price(&group.model, &group.alias, &prices) {
            let identity = format!(
                "{} {} {}",
                group.executor_type, group.provider, group.auth_type
            )
            .to_ascii_lowercase();
            let service_tier =
                if identity.contains("codex") || group.response_service_tier.trim().is_empty() {
                    group.service_tier.as_str()
                } else {
                    group.response_service_tier.as_str()
                };
            let cost = cost_for_price(model, service_tier, &group.tokens, &price);
            entry.estimated_cost += cost;
            entry.price = Some(enriched_model_price(model, &price));
            total_cost += cost;
            priced_requests = priced_requests.saturating_add(group.requests);
        }
    }
    for price in prices.values() {
        rows.entry(price.model.clone())
            .or_insert_with(|| UsagePriceRow {
                model: price.model.clone(),
                price: Some(enriched_model_price(&price.model, price)),
                ..UsagePriceRow::default()
            });
    }
    let mut rows = rows.into_values().collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.price
            .is_some()
            .cmp(&right.price.is_some())
            .then_with(|| right.requests.cmp(&left.requests))
            .then_with(|| left.model.cmp(&right.model))
    });
    Ok(UsagePricing {
        rows,
        total_cost,
        total_requests,
        priced_requests,
        saved_prices: prices.len(),
    })
}

#[tauri::command]
pub(crate) async fn save_usage_model_price(mut price: ModelPrice) -> Result<(), String> {
    price.model = price.model.trim().to_string();
    price.source = "manual".to_string();
    price.source_model_id.clear();
    price.updated_at_ms = Local::now().timestamp_millis();
    run_usage_task(move || upsert_model_price(&open_usage_database()?, &price)).await
}

#[tauri::command]
pub(crate) async fn delete_usage_model_price(model: String) -> Result<(), String> {
    run_usage_task(move || {
        let connection = open_usage_database()?;
        connection
            .execute(
                "DELETE FROM model_prices WHERE model = ?1 COLLATE NOCASE",
                params![model.trim()],
            )
            .map_err(|error| format!("Failed to delete model pricing: {error}"))?;
        Ok(())
    })
    .await
}

fn sync_price_for_model<'a>(model: &str, prices: &'a [ModelPrice]) -> Option<&'a ModelPrice> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let exact = prices
        .iter()
        .filter(|price| price.source_model_id.eq_ignore_ascii_case(model))
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return Some(exact[0]);
    }
    let tail = model.rsplit('/').next().unwrap_or(model);
    let family = tail.to_ascii_lowercase();
    let mut candidates = prices
        .iter()
        .filter(|price| price.model.eq_ignore_ascii_case(tail))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        let normalized = canonical_model_tail(tail);
        candidates = prices
            .iter()
            .filter(|price| canonical_model_tail(&price.model) == normalized)
            .collect();
    }
    let provider = if family.starts_with("gpt-")
        || family.starts_with("chatgpt-")
        || family.starts_with("o1-")
        || family.starts_with("o3-")
        || family.starts_with("o4-")
    {
        Some("openai")
    } else if family.starts_with("claude-") {
        Some("anthropic")
    } else if family.starts_with("gemini-") {
        Some("google")
    } else if family.starts_with("deepseek-") {
        Some("deepseek")
    } else if family.starts_with("grok-") {
        Some("xai")
    } else {
        None
    };
    if let Some(provider) = provider {
        let official = candidates
            .iter()
            .filter(|price| price.source_model_id.starts_with(&format!("{provider}/")))
            .collect::<Vec<_>>();
        if official.len() == 1 {
            return Some(official[0]);
        }
    }
    (candidates.len() == 1).then(|| candidates[0])
}

fn build_model_price_preview(
    models: &[(String, String)],
    remote_prices: &[ModelPrice],
    source: &str,
    source_url: &str,
) -> ModelPriceSyncPreview {
    let mut matches = Vec::new();
    let mut unmatched = Vec::new();
    for (model, alias) in models {
        let matched = [model.as_str(), alias.as_str()]
            .into_iter()
            .find_map(|candidate| sync_price_for_model(candidate, remote_prices));
        if let Some(price) = matched {
            let mut price = price.clone();
            price.model = model.clone();
            matches.push(price);
        } else {
            unmatched.push(model.clone());
        }
    }
    ModelPriceSyncPreview {
        source: source.to_string(),
        source_url: source_url.to_string(),
        matches,
        unmatched,
    }
}

fn collect_price_preview_models(
    connection: &Connection,
    query: &UsageQuery,
) -> Result<Vec<(String, String)>, String> {
    let mut models = std::collections::BTreeMap::<String, (String, String)>::new();
    for price in load_model_prices(connection)?.values() {
        models.insert(price.model.to_ascii_lowercase(), (price.model.clone(), String::new()));
    }
    let filter = build_usage_filter(query);
    for group in load_usage_cost_groups(connection, &filter)? {
        let entry = models
            .entry(group.model.to_ascii_lowercase())
            .or_insert_with(|| (group.model, String::new()));
        if entry.1.is_empty() && !group.alias.trim().is_empty() {
            entry.1 = group.alias;
        }
    }
    Ok(models.into_values().collect())
}

async fn fetch_model_price_catalog_once(
    client: &reqwest::Client,
    source_name: &str,
    source_url: &str,
) -> Result<String, (bool, String)> {
    let mut response = client
        .get(source_url)
        .send()
        .await
        .map_err(|error| (true, format!("Failed to connect to {source_name}: {error}")))?;
    let status = response.status();
    if !status.is_success() {
        let retryable = status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
        return Err((retryable, format!("Failed to read {source_name} pricing: HTTP {status}")));
    }
    if response.content_length().is_some_and(|length| length > MODEL_PRICE_SYNC_MAX_BYTES as u64) {
        return Err((false, format!("{source_name} pricing catalog is too large")));
    }
    let mut content = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| (true, format!("Failed to read {source_name} response: {error}")))?
    {
        if content.len().saturating_add(chunk.len()) > MODEL_PRICE_SYNC_MAX_BYTES {
            return Err((false, format!("{source_name} pricing catalog is too large")));
        }
        content.extend_from_slice(&chunk);
    }
    String::from_utf8(content)
        .map_err(|error| (true, format!("{source_name} response is not UTF-8: {error}")))
}

async fn fetch_model_price_catalog(
    client: &reqwest::Client,
    source_name: &str,
    source_url: &str,
) -> Result<String, String> {
    for attempt in 1..=MODEL_PRICE_SYNC_ATTEMPTS {
        match fetch_model_price_catalog_once(client, source_name, source_url).await {
            Ok(content) => return Ok(content),
            Err((true, _)) if attempt < MODEL_PRICE_SYNC_ATTEMPTS => {
                tokio::time::sleep(Duration::from_millis(400 * attempt as u64)).await;
            }
            Err((retryable, error)) => {
                return Err(if retryable {
                    format!("{error} (attempted {attempt} times)")
                } else {
                    error
                });
            }
        }
    }
    unreachable!("Attempt to read the pricing catalog at least once")
}

#[tauri::command]
pub(crate) async fn preview_usage_model_prices(
    query: UsageQuery,
    source: String,
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<ModelPriceSyncPreview, String> {
    let (source_name, source_url) = match source.as_str() {
        "models-dev" => ("Models.dev", MODEL_PRICE_SYNC_URL),
        "litellm" => ("LiteLLM", LITELLM_PRICE_SYNC_URL),
        _ => return Err("Unsupported pricing source".to_string()),
    };
    let config = gui_config_state.snapshot()?;
    let client_builder = reqwest::Client::builder()
        .user_agent("EasyCLIProxyAPI/0.2.97")
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60));
    let proxy_url = config.proxy_url.trim();
    let client = apply_configured_proxy(client_builder, proxy_url)
        .map_err(|error| format!("Failed to configure pricing synchronization proxy: {error}"))?
        .build()
        .map_err(|error| format!("Failed to create pricing synchronization client: {error}"))?;
    let content = fetch_model_price_catalog(&client, source_name, source_url).await?;
    let now = Local::now().timestamp_millis();
    let remote_prices = if source == "models-dev" {
        parse_models_dev_prices(&content, now)?
    } else {
        parse_litellm_prices(&content, now)?
    };

    run_usage_task(move || {
        let connection = open_usage_database()?;
        let models = collect_price_preview_models(&connection, &query)?;
        Ok(build_model_price_preview(
            &models,
            &remote_prices,
            source_name,
            source_url,
        ))
    })
    .await
}

#[tauri::command]
pub(crate) async fn apply_usage_model_prices(prices: Vec<ModelPrice>) -> Result<ModelPriceSyncResult, String> {
    let mut seen = std::collections::HashSet::new();
    for price in &prices {
        validate_model_price(price)?;
        if !matches!(price.source.as_str(), "models.dev" | "litellm-sync")
            || price.source_model_id.trim().is_empty()
        {
            return Err("Invalid pricing synchronization source".to_string());
        }
        if !seen.insert(price.model.to_ascii_lowercase()) {
            return Err(format!("Duplicate model pricing: {}", price.model));
        }
    }
    run_usage_task(move || {
        let mut connection = open_usage_database()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("Failed to begin saving model pricing: {error}"))?;
        for mut price in prices.iter().cloned() {
            price.model = price.model.trim().to_string();
            price.updated_at_ms = Local::now().timestamp_millis();
            upsert_model_price(&transaction, &price)?;
        }
        transaction
            .commit()
            .map_err(|error| format!("Failed to commit model pricing: {error}"))?;
        Ok(ModelPriceSyncResult { imported: prices.len() })
    })
    .await
}

fn canonical_model_tail(value: &str) -> String {
    normalized_model_tail(value)
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn normalized_model_tail(value: &str) -> String {
    value
        .trim()
        .rsplit('/')
        .find(|part| !part.trim().is_empty() && !part.eq_ignore_ascii_case("models"))
        .unwrap_or(value)
        .to_ascii_lowercase()
}

#[tauri::command]
pub(crate) async fn get_usage_analysis(
    query: UsageQuery,
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<UsageAnalysis, String> {
    let config = gui_config_state.snapshot()?;
    run_usage_task(move || load_usage_analysis(&open_usage_database()?, &query, &config)).await
}

fn load_usage_analysis(
    connection: &Connection,
    query: &UsageQuery,
    config: &GuiConfigFile,
) -> Result<UsageAnalysis, String> {
    let filter = build_usage_filter(query);
    let sql = format!(
        r#"
        SELECT
            {USAGE_DISPLAY_MODEL_SQL},
            COALESCE(NULLIF(TRIM(provider), ''), 'Unknown Provider'),
            COALESCE(NULLIF(TRIM(source), ''), 'Unknown source'),
            COALESCE(NULLIF(TRIM(api_key_hash), ''), 'Unrecorded key'),
            MAX(TRIM(api_key_remark)), MAX(TRIM(api_key_display)),
            COUNT(*),
            COALESCE(SUM(CASE WHEN failed != 0 AND canceled = 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(total_tokens), 0)
        FROM usage_events{}
        GROUP BY 1, 2, 3, 4
        "#,
        filter.clause
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Failed to prepare usage analytics query: {error}"))?;
    let mut rows = statement
        .query(params_from_iter(filter.params.iter()))
        .map_err(|error| format!("Failed to query usage analytics: {error}"))?;
    let mut categories: [HashMap<String, UsageCategory>; 4] = Default::default();
    let mut key_labels = HashMap::<String, (String, String)>::new();
    let read_result = (|| -> rusqlite::Result<()> {
        while let Some(row) = rows.next()? {
            let requests = from_sql_i64(row.get(6)?);
            let failures = from_sql_i64(row.get(7)?);
            let tokens = from_sql_i64(row.get(8)?);
            for (index, group) in categories.iter_mut().enumerate() {
                let key: String = row.get(index)?;
                if index == 3 {
                    let remark: String = row.get(4)?;
                    let display: String = row.get(5)?;
                    let labels = key_labels.entry(key.clone()).or_default();
                    if remark > labels.0 {
                        labels.0 = remark;
                    }
                    if display > labels.1 {
                        labels.1 = display;
                    }
                }
                let category = group.entry(key.clone()).or_insert_with(|| UsageCategory {
                    label: key.clone(),
                    key,
                    ..UsageCategory::default()
                });
                category.requests = category.requests.saturating_add(requests);
                category.failures = category.failures.saturating_add(failures);
                category.tokens = category.tokens.saturating_add(tokens);
            }
        }
        Ok(())
    })();
    read_result.map_err(|error| format!("Failed to read usage analytics: {error}"))?;
    let [models, providers, sources, api_keys] = categories.map(sorted_usage_categories);
    let sources = sources
        .into_iter()
        .map(|mut category| {
            category.label = usage_source_display(config, "", &category.key);
            category
        })
        .collect();
    let api_keys = api_keys
        .into_iter()
        .map(|mut category| {
            let (remark, display) = key_labels.remove(&category.key).unwrap_or_default();
            category.label = api_key_category_label(remark, display);
            category
        })
        .collect();
    Ok(UsageAnalysis {
        models,
        providers,
        sources,
        api_keys,
    })
}

fn sorted_usage_categories(categories: HashMap<String, UsageCategory>) -> Vec<UsageCategory> {
    let mut categories: Vec<_> = categories.into_values().collect();
    categories.sort_by(|left, right| {
        right
            .tokens
            .cmp(&left.tokens)
            .then_with(|| right.requests.cmp(&left.requests))
            .then_with(|| left.key.cmp(&right.key))
    });
    categories
}

#[cfg(test)]
fn load_source_categories(
    connection: &Connection,
    query: &UsageQuery,
    config: &GuiConfigFile,
) -> Result<Vec<UsageCategory>, String> {
    let mut categories = load_simple_categories(connection, query, "source", "Unknown source")?;
    for category in &mut categories {
        category.label = usage_source_display(config, "", &category.key);
    }
    Ok(categories)
}

#[cfg(test)]
fn load_simple_categories(
    connection: &Connection,
    query: &UsageQuery,
    column: &str,
    fallback: &str,
) -> Result<Vec<UsageCategory>, String> {
    let filter = build_usage_filter(query);
    let sql = format!(
        r#"
        SELECT
            COALESCE(NULLIF(TRIM({column}), ''), ?),
            COUNT(*),
            COALESCE(SUM(CASE WHEN failed != 0 AND canceled = 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(total_tokens), 0)
        FROM usage_events{}
        GROUP BY 1
        ORDER BY 4 DESC, 2 DESC
        "#,
        filter.clause
    );
    let mut values = Vec::with_capacity(filter.params.len() + 1);
    values.push(SqlValue::Text(fallback.to_string()));
    values.extend(filter.params);
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Failed to prepare SQLite usage analytics query: {error}"))?;
    let categories = statement
        .query_map(params_from_iter(values.iter()), |row| {
            let key = row.get::<_, String>(0)?;
            Ok(UsageCategory {
                label: key.clone(),
                key,
                requests: from_sql_i64(row.get(1)?),
                failures: from_sql_i64(row.get(2)?),
                tokens: from_sql_i64(row.get(3)?),
            })
        })
        .map_err(|error| format!("Failed to query SQLite usage analytics: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Failed to read SQLite usage analytics: {error}"))?;
    Ok(categories)
}

#[cfg(test)]
fn load_api_key_categories(
    connection: &Connection,
    query: &UsageQuery,
) -> Result<Vec<UsageCategory>, String> {
    let filter = build_usage_filter(query);
    let sql = format!(
        r#"
        SELECT
            COALESCE(NULLIF(TRIM(api_key_hash), ''), 'Unrecorded key'),
            MAX(TRIM(api_key_remark)),
            MAX(TRIM(api_key_display)),
            COUNT(*),
            COALESCE(SUM(CASE WHEN failed != 0 AND canceled = 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(total_tokens), 0)
        FROM usage_events{}
        GROUP BY 1
        ORDER BY 6 DESC, 4 DESC
        "#,
        filter.clause
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Failed to prepare SQLite API Key usage analytics query: {error}"))?;
    let categories = statement
        .query_map(params_from_iter(filter.params.iter()), |row| {
            let key = row.get::<_, String>(0)?;
            let remark = row.get::<_, String>(1)?;
            let display = row.get::<_, String>(2)?;
            let label = api_key_category_label(remark, display);
            Ok(UsageCategory {
                key,
                label,
                requests: from_sql_i64(row.get(3)?),
                failures: from_sql_i64(row.get(4)?),
                tokens: from_sql_i64(row.get(5)?),
            })
        })
        .map_err(|error| format!("Failed to query SQLite API Key usage analytics: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Failed to read SQLite API Key usage analytics: {error}"))?;
    Ok(categories)
}


#[tauri::command]
pub(crate) async fn save_usage_events_export(path: String, contents: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || write_usage_events_export(&path, &contents))
        .await
        .map_err(|error| format!("Failed to save usage export: {error}"))?
}

fn write_usage_events_export(path: &str, contents: &str) -> Result<(), String> {
    let path = validated_usage_export_path(path)?;
    fs::write(&path, contents.as_bytes())
        .map_err(|error| format!("Failed to write usage export {}: {error}", path.display()))
}

fn validated_usage_export_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err("Usage export path is invalid".to_string());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("Usage export path must be an absolute file path".to_string());
    }
    if path.file_name().is_none_or(|name| name.is_empty()) {
        return Err("Usage export path must include a file name".to_string());
    }
    if path.is_dir() {
        return Err("Usage export path is a directory".to_string());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "Usage export path has no directory".to_string())?;
    if !parent.is_dir() {
        return Err(format!(
            "Usage export directory does not exist: {}",
            parent.display()
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod save_usage_events_export_tests {
    use super::{validated_usage_export_path, write_usage_events_export};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "cpa-usage-export-{}-{}-{}",
                std::process::id(),
                label,
                stamp
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn writes_the_selected_csv_and_replaces_an_existing_file() {
        let directory = TempDir::new("write");
        let path = directory.path().join("usage.csv");
        fs::write(&path, "old").unwrap();
        let selected = path.to_string_lossy().into_owned();
        write_usage_events_export(&selected, "\u{feff}id\r\n1").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "\u{feff}id\r\n1");
    }

    #[test]
    fn rejects_relative_paths_missing_directories_and_directories() {
        let directory = TempDir::new("reject");
        assert!(validated_usage_export_path("usage.csv").is_err());
        assert!(validated_usage_export_path(&directory.path().to_string_lossy()).is_err());
        let missing = directory
            .path()
            .join("missing")
            .join("usage.csv")
            .to_string_lossy()
            .into_owned();
        assert!(write_usage_events_export(&missing, "id").is_err());
    }
}

#[tauri::command]
pub(crate) async fn get_usage_events(
    query: UsageQuery,
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<UsageEventPage, String> {
    let config = gui_config_state.snapshot()?;
    run_usage_task(move || load_usage_events(&open_usage_database()?, &query, &config)).await
}

fn load_usage_events(
    connection: &Connection,
    query: &UsageQuery,
    config: &GuiConfigFile,
) -> Result<UsageEventPage, String> {
    let filter = build_usage_filter(query);
    let total_sql = format!("SELECT COUNT(*) FROM usage_events{}", filter.clause);
    let total = connection
        .query_row(&total_sql, params_from_iter(filter.params.iter()), |row| {
            row.get::<_, i64>(0)
        })
        .map(from_sql_i64)
        .map_err(|error| format!("Failed to count SQLite usage events: {error}"))?
        .min(usize::MAX as u64) as usize;
    let page_size = query.page_size.unwrap_or(50).clamp(1, 5_000);
    let total_pages = total.div_ceil(page_size).max(1);
    let page = query.page.unwrap_or(1).clamp(1, total_pages);
    let offset = (page - 1).saturating_mul(page_size);

    let sql = format!(
        r#"
        SELECT
            event_key, timestamp, latency_ms, ttft_ms, source, auth_index, failed,
            provider, model, alias, reasoning_effort, service_tier,
            response_service_tier, executor_type, endpoint, auth_type,
            api_key_hash, api_key_display, api_key_remark, request_id,
            api_group_key, client_ip, x_forwarded_for, user_agent, generate,
            cached_tokens, collector_source,
            input_tokens, output_tokens, reasoning_tokens, cache_read_tokens,
            cache_creation_tokens, total_tokens, canceled, failure_status,
            failure_body, id, response_model
        FROM usage_events{}
        ORDER BY timestamp_ms DESC, id DESC
        LIMIT ? OFFSET ?
        "#,
        filter.clause
    );
    let mut values = filter.params;
    values.push(SqlValue::Integer(page_size as i64));
    values.push(SqlValue::Integer(offset.min(i64::MAX as usize) as i64));
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Failed to prepare SQLite usage event query: {error}"))?;
    let mut items = statement
        .query_map(params_from_iter(values.iter()), usage_record_from_row)
        .map_err(|error| format!("Failed to query SQLite usage events: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Failed to read SQLite usage events: {error}"))?;
    let prices = load_model_prices(connection)?;
    for item in &mut items {
        item.source_display = usage_source_display(config, &item.provider, &item.source);
        item.cost = usage_record_cost(item, &prices);
    }
    Ok(UsageEventPage {
        items,
        total,
        page,
        page_size,
        total_pages,
    })
}

fn usage_record_from_row(row: &Row<'_>) -> rusqlite::Result<UsageRecord> {
    Ok(UsageRecord {
        id: row.get(0)?,
        row_id: Some(row.get::<_, i64>(36)?.to_string()),
        timestamp: row.get(1)?,
        latency_ms: from_sql_i64(row.get(2)?),
        ttft_ms: row.get::<_, Option<i64>>(3)?.map(from_sql_i64),
        source: row.get(4)?,
        source_display: String::new(),
        auth_index: row.get(5)?,
        failed: row.get::<_, i64>(6)? != 0,
        canceled: row.get::<_, i64>(33)? != 0,
        failure_status: row.get::<_, i64>(34)?.clamp(0, u16::MAX as i64) as u16,
        failure_body: row.get(35)?,
        provider: row.get(7)?,
        api_group_key: row.get(20)?,
        model: row.get(8)?,
        response_model: row.get(37)?,
        cost: None,
        alias: row.get(9)?,
        client_ip: row.get(21)?,
        x_forwarded_for: row.get(22)?,
        user_agent: row.get(23)?,
        reasoning_effort: row.get(10)?,
        service_tier: row.get(11)?,
        response_service_tier: row.get(12)?,
        executor_type: row.get(13)?,
        endpoint: row.get(14)?,
        auth_type: row.get(15)?,
        api_key_hash: row.get(16)?,
        api_key_display: row.get(17)?,
        api_key_remark: row.get(18)?,
        request_id: row.get(19)?,
        generate: row.get::<_, i64>(24)? != 0,
        cached_tokens: from_sql_i64(row.get(25)?),
        collector_source: row.get(26)?,
        tokens: UsageTokenStats {
            input_tokens: from_sql_i64(row.get(27)?),
            output_tokens: from_sql_i64(row.get(28)?),
            reasoning_tokens: from_sql_i64(row.get(29)?),
            cache_read_tokens: from_sql_i64(row.get(30)?),
            cache_creation_tokens: from_sql_i64(row.get(31)?),
            total_tokens: from_sql_i64(row.get(32)?),
        },
    })
}

fn total_usage_records() -> Result<u64, String> {
    let connection = open_usage_database()?;
    connection
        .query_row("SELECT COUNT(*) FROM usage_events", [], |row| {
            row.get::<_, i64>(0)
        })
        .map(from_sql_i64)
        .map_err(|error| format!("Failed to count total SQLite usage records: {error}"))
}

fn query_window_minutes(query: &UsageQuery, first: Option<i64>, last: Option<i64>) -> f64 {
    let start = query
        .start
        .as_deref()
        .and_then(parse_timestamp_millis)
        .or(first)
        .unwrap_or(0);
    let end = query
        .end
        .as_deref()
        .and_then(parse_timestamp_millis)
        .or(last)
        .unwrap_or(start);
    ((end.saturating_sub(start)) as f64 / 60_000.0).max(1.0)
}

fn usage_root_dir() -> Result<PathBuf, String> {
    Ok(core_base_dir()?.join(USAGE_DIR_NAME))
}

fn record_local_hour(record: &UsageRecord) -> String {
    local_hour_from_timestamp(&record.timestamp)
        .unwrap_or_else(|| Local::now().format("%Y-%m-%d-%H").to_string())
}

fn local_hour_from_timestamp(value: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(value).ok().map(|timestamp| {
        timestamp
            .with_timezone(&Local)
            .format("%Y-%m-%d-%H")
            .to_string()
    })
}

fn record_timestamp_millis(record: &UsageRecord) -> i64 {
    parse_timestamp_millis(&record.timestamp).unwrap_or(0)
}

fn parse_timestamp_millis(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn to_sql_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn from_sql_i64(value: i64) -> u64 {
    value.max(0) as u64
}

fn unique_file_stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(|value| match value {
            Value::String(value) => Some(value.trim().to_string()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())
}

fn u64_field(object: &serde_json::Map<String, Value>, key: &str) -> u64 {
    optional_u64_field(object, key).unwrap_or(0)
}

fn optional_u64_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key).and_then(|value| {
        value.as_u64().or_else(|| {
            value
                .as_i64()
                .map(|number| number.max(0) as u64)
                .or_else(|| value.as_str().and_then(|text| text.parse::<u64>().ok()))
        })
    })
}

fn token_u64(object: Option<&serde_json::Map<String, Value>>, key: &str) -> u64 {
    object.map(|object| u64_field(object, key)).unwrap_or(0)
}

fn token_was_negative(object: Option<&serde_json::Map<String, Value>>, key: &str) -> bool {
    object
        .and_then(|object| object.get(key))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
        })
        .is_some_and(|value| value < 0)
}

fn hash_text(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn mask_api_key(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if value.chars().count() <= 8 {
        return format!("{}••••", value.chars().take(2).collect::<String>());
    }
    let start = value.chars().take(4).collect::<String>();
    let end = value
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{start}••••{end}")
}

fn api_key_category_label(remark: String, display: String) -> String {
    if !remark.is_empty() {
        remark
    } else if !display.is_empty() {
        mask_api_key(&display)
    } else {
        "Unrecorded key".to_string()
    }
}

fn usage_source_display(config: &GuiConfigFile, provider: &str, source: &str) -> String {
    let source = source.trim();
    if source.is_empty() {
        return "Unknown source".to_string();
    }
    if let Some(remark) = config.api_access_remark_for_source(provider, source) {
        return remark.to_string();
    }
    let character_count = source.chars().count();
    let looks_like_secret = !source.contains('@')
        && !source.chars().any(char::is_whitespace)
        && (source.starts_with("sk-")
            || source.starts_with("AIza")
            || source.starts_with("key-")
            || character_count >= 48);
    if looks_like_secret {
        mask_api_key(source)
    } else {
        source.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn redis_usage_queue_falls_back_to_legacy_key_and_reuses_selection() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let legacy_message = r#"{"request_id":"legacy"}"#;
        let expectations = vec![
            (
                USAGE_QUEUE_KEY.to_string(),
                "-ERR unsupported channel 'usage'\r\n".to_string(),
            ),
            (LEGACY_USAGE_QUEUE_KEY.to_string(), "*0\r\n".to_string()),
            (
                LEGACY_USAGE_QUEUE_KEY.to_string(),
                format!("*1\r\n${}\r\n{legacy_message}\r\n", legacy_message.len()),
            ),
        ];
        let server = tokio::spawn(async move {
            for (queue_key, response) in expectations {
                let (mut stream, _) = listener.accept().await.unwrap();
                let expected_auth = b"*2\r\n$4\r\nAUTH\r\n$6\r\nsecret\r\n";
                let mut auth = vec![0_u8; expected_auth.len()];
                stream.read_exact(&mut auth).await.unwrap();
                assert_eq!(auth, expected_auth);
                stream.write_all(b"+OK\r\n").await.unwrap();

                let batch_size = USAGE_QUEUE_BATCH_SIZE.to_string();
                let expected_pop = format!(
                    "*3\r\n$4\r\nLPOP\r\n${}\r\n{queue_key}\r\n${}\r\n{batch_size}\r\n",
                    queue_key.len(),
                    batch_size.len()
                );
                let mut pop = vec![0_u8; expected_pop.len()];
                stream.read_exact(&mut pop).await.unwrap();
                assert_eq!(pop, expected_pop.as_bytes());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let mut config = GuiConfigFile::default();
        config.port = port;
        config.management_secret_key = "secret".to_string();
        let mut source = RedisUsageQueueSource::default();

        let first = source.pull(&config).await.unwrap();
        assert_eq!(first.source, "redis_pull:queue");
        assert!(first.messages.is_empty());

        let second = source.pull(&config).await.unwrap();
        assert_eq!(second.source, "redis_pull:queue");
        assert_eq!(second.messages, vec![legacy_message]);
        server.await.unwrap();
    }

    #[test]
    fn redis_usage_queue_only_falls_back_for_unsupported_key_errors() {
        assert!(redis_pull_can_try_legacy_queue(
            "ERR unsupported channel 'usage'"
        ));
        assert!(redis_pull_can_try_legacy_queue(
            "ERR unsupported queue 'usage'"
        ));
        assert!(!redis_pull_can_try_legacy_queue("CPA usage subscription authentication failed"));
        assert!(!redis_pull_can_try_legacy_queue("connection refused"));
    }

    #[tokio::test]
    async fn background_usage_jobs_limit_database_concurrency() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut jobs = Vec::new();
        for index in 0..6 {
            let active = active.clone();
            let peak = peak.clone();
            jobs.push(tokio::spawn(run_usage_task(move || {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(10));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(index)
            })));
        }
        for (index, job) in jobs.into_iter().enumerate() {
            assert_eq!(job.await.unwrap().unwrap(), index);
        }
        assert!(peak.load(Ordering::SeqCst) <= 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn single_scan_analysis_matches_independent_categories_for_mixed_data_and_filters() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_usage_schema(&connection).unwrap();
        let records: Vec<_> = (0..600)
            .map(|index| {
                let mut record = sample_record(
                    &format!("analysis-{index}"),
                    if index % 2 == 0 {
                        "2026-08-27T00:00:00Z"
                    } else {
                        "2026-08-28T00:00:00Z"
                    },
                    [" Model-A ", "model-a", "", "unknown", "Model-B"][index % 5],
                );
                record.alias = ["", " model-alias ", ""][index % 3].to_string();
                record.provider = ["openai", "claude", " "][index % 3].to_string();
                record.source =
                    ["source-a", " ", "source-b", "sk-secret-source"][index % 4].to_string();
                record.api_key_hash = ["hash-a", "hash-b", " "][index % 3].to_string();
                record.api_key_remark = ["", " alpha ", "zeta", "Test note"][index % 4].to_string();
                record.api_key_display = ["", "ab••••", "xy••••"][index % 3].to_string();
                record.failed = index % 3 == 0;
                record.canceled = index % 6 == 0;
                record.tokens.total_tokens = (index % 17) as u64;
                record
            })
            .collect();
        insert_usage_records(&mut connection, &records).unwrap();
        let config = GuiConfigFile::default();
        let queries = [
            UsageQuery::default(),
            UsageQuery {
                model: Some("MODEL-A".into()),
                ..UsageQuery::default()
            },
            UsageQuery {
                provider: Some("claude".into()),
                ..UsageQuery::default()
            },
            UsageQuery {
                source: Some("source-a".into()),
                ..UsageQuery::default()
            },
            UsageQuery {
                api_key_hash: Some("hash-a".into()),
                ..UsageQuery::default()
            },
            UsageQuery {
                failed: Some(true),
                canceled: Some(false),
                ..UsageQuery::default()
            },
            UsageQuery {
                canceled: Some(true),
                ..UsageQuery::default()
            },
            UsageQuery {
                start: Some("2026-08-28T00:00:00Z".into()),
                ..UsageQuery::default()
            },
            UsageQuery {
                model: Some("not-present".into()),
                ..UsageQuery::default()
            },
        ];
        let canonical = |mut categories: Vec<UsageCategory>| {
            categories.sort_by(|left, right| left.key.cmp(&right.key));
            serde_json::to_value(categories).unwrap()
        };
        for query in queries {
            let actual = load_usage_analysis(&connection, &query, &config).unwrap();
            assert_eq!(
                canonical(actual.models),
                canonical(
                    load_simple_categories(&connection, &query, USAGE_DISPLAY_MODEL_SQL, "unknown")
                        .unwrap()
                )
            );
            assert_eq!(
                canonical(actual.providers),
                canonical(
                    load_simple_categories(&connection, &query, "provider", "Unknown Provider")
                        .unwrap()
                )
            );
            assert_eq!(
                canonical(actual.sources),
                canonical(load_source_categories(&connection, &query, &config).unwrap())
            );
            assert_eq!(
                canonical(actual.api_keys),
                canonical(load_api_key_categories(&connection, &query).unwrap())
            );
        }
    }

    #[test]
    fn model_categories_and_filter_keep_aliases_separate() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_usage_schema(&connection).unwrap();
        let mut records = Vec::new();
        for (index, alias) in ["", "model-fast", "model-high", "model-fast"]
            .into_iter()
            .enumerate()
        {
            let mut record = sample_record(
                &format!("model-alias-{index}"),
                "2026-09-25T00:00:00Z",
                "base-model",
            );
            record.alias = alias.to_string();
            records.push(record);
        }
        insert_usage_records(&mut connection, &records).unwrap();
        let config = GuiConfigFile::default();
        let analysis = load_usage_analysis(&connection, &UsageQuery::default(), &config).unwrap();
        let counts = analysis
            .models
            .iter()
            .map(|category| (category.key.as_str(), category.requests))
            .collect::<HashMap<_, _>>();
        assert_eq!(counts.len(), 3);
        assert_eq!(counts["base-model"], 1);
        assert_eq!(counts["model-fast"], 2);
        assert_eq!(counts["model-high"], 1);
        let overview = load_usage_overview(&connection, &UsageQuery::default()).unwrap();
        let trend_models = overview
            .timeline
            .iter()
            .flat_map(|point| point.models.iter().map(|model| model.key.as_str()))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(trend_models, ["base-model", "model-fast", "model-high"].into());

        for (model, expected) in [("base-model", 1), ("model-fast", 2), ("model-high", 1)] {
            let query = UsageQuery {
                model: Some(model.to_string()),
                ..UsageQuery::default()
            };
            let filtered = load_usage_analysis(&connection, &query, &config).unwrap();
            assert_eq!(filtered.models.len(), 1);
            assert_eq!(filtered.models[0].key, model);
            assert_eq!(filtered.models[0].requests, expected);
            assert_eq!(load_usage_events(&connection, &query, &config).unwrap().total, expected as usize);
        }
    }

    #[test]
    fn collector_total_records_apply_insertions_and_retention_deletions() {
        let state = UsageCollectorState::default();
        state.set_total_records(40);
        state.apply_record_changes(4, 2);

        assert_eq!(state.status().unwrap().total_records, 42);

        state.apply_record_changes(0, 100);
        assert_eq!(state.status().unwrap().total_records, 0);
    }

    fn test_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "cpa-gui-usage-{name}-{}-{}",
            std::process::id(),
            unique_file_stamp()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn legacy_usage_storage_is_moved_with_nested_files() {
        let root = test_root("storage-location-migration");
        let source = root.join("legacy").join(USAGE_DIR_NAME);
        let target = root.join("application-support").join(USAGE_DIR_NAME);
        fs::create_dir_all(source.join(USAGE_BACKUP_DIR_NAME)).unwrap();
        fs::write(source.join(USAGE_DATABASE_FILE), b"database").unwrap();
        fs::write(
            source.join(USAGE_BACKUP_DIR_NAME).join("usage.db.backup"),
            b"backup",
        )
        .unwrap();

        migrate_usage_storage_directory(&source, &target).unwrap();

        assert!(!source.exists());
        assert_eq!(
            fs::read(target.join(USAGE_DATABASE_FILE)).unwrap(),
            b"database"
        );
        assert_eq!(
            fs::read(target.join(USAGE_BACKUP_DIR_NAME).join("usage.db.backup")).unwrap(),
            b"backup"
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn open_test_database(root: &Path) -> Connection {
        initialize_usage_storage_at(root).unwrap();
        open_usage_database_at(root).unwrap()
    }

    #[test]
    fn usage_database_limit_defaults_to_unlimited_and_persists() {
        let root = test_root("database-limit-persistence");
        let connection = open_test_database(&root);

        assert_eq!(load_usage_database_limit(&connection).unwrap(), 0);
        assert!(shrink_usage_database_to_mb(&connection, 0).is_err());
        save_usage_database_limit(&connection, 128).unwrap();
        drop(connection);

        let connection = open_usage_database_at(&root).unwrap();
        assert_eq!(load_usage_database_limit(&connection).unwrap(), 128);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn one_time_database_shrink_keeps_the_automatic_limit() {
        let root = test_root("one-time-database-shrink");
        let mut connection = open_test_database(&root);
        save_usage_database_limit(&connection, 128).unwrap();

        let payload = "x".repeat(8 * 1024);
        let transaction = connection.transaction().unwrap();
        for timestamp_ms in 0_i64..320 {
            transaction
                .execute(
                    r#"INSERT INTO usage_events (
                           event_key, timestamp, timestamp_ms, local_hour,
                           failure_body, created_at
                       ) VALUES (?1, ?2, ?3, '2026-01-01T00', ?4, ?2)"#,
                    params![
                        format!("one-time-shrink-{timestamp_ms}"),
                        format!("2026-01-01T00:00:{timestamp_ms:03}Z"),
                        timestamp_ms,
                        payload,
                    ],
                )
                .unwrap();
        }
        transaction.commit().unwrap();

        let deleted = shrink_usage_database_to_mb(&connection, 1).unwrap();

        assert!(deleted > 0);
        assert_eq!(load_usage_database_limit(&connection).unwrap(), 128);
        assert!(usage_database_disk_bytes(&root).unwrap() <= BYTES_PER_MB);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn usage_database_limit_removes_oldest_records_after_collection() {
        let root = test_root("database-limit-retention");
        let mut connection = open_test_database(&root);
        save_usage_database_limit(&connection, 1).unwrap();

        let payload = "x".repeat(8 * 1024);
        let transaction = connection.transaction().unwrap();
        for timestamp_ms in 0_i64..512 {
            transaction
                .execute(
                    r#"INSERT INTO usage_events (
                           event_key, timestamp, timestamp_ms, local_hour, model,
                           failure_body, created_at
                       ) VALUES (?1, ?2, ?3, '2026-01-01T00', 'retention-test', ?4, ?2)"#,
                    params![
                        format!("retention-{timestamp_ms}"),
                        format!("2026-01-01T00:00:{timestamp_ms:03}Z"),
                        timestamp_ms,
                        payload,
                    ],
                )
                .unwrap();
        }
        transaction.commit().unwrap();

        enqueue_usage_queue_items(
            &mut connection,
            "retention-test",
            vec![serde_json::json!({
                "timestamp": "2026-09-18T12:00:00+08:00",
                "request_id": "newest-retained-record",
                "model": "retention-test",
                "tokens": { "input_tokens": 1, "output_tokens": 1 }
            })],
        )
        .unwrap();
        let outcome = process_usage_inbox(&mut connection, &GuiConfigFile::default()).unwrap();

        assert_eq!(outcome.inserted, 1);
        assert!(outcome.deleted > 0);
        let (remaining, oldest_timestamp, newest_record): (u64, i64, u64) = connection
            .query_row(
                r#"SELECT COUNT(*), MIN(timestamp_ms),
                          SUM(CASE WHEN request_id = 'newest-retained-record' THEN 1 ELSE 0 END)
                   FROM usage_events"#,
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!(remaining > 0);
        assert!(remaining < 513);
        assert!(oldest_timestamp > 0);
        assert_eq!(newest_record, 1);
        assert!(usage_database_active_bytes(&connection).unwrap() <= BYTES_PER_MB);
        assert!(usage_database_disk_bytes(&root).unwrap() <= BYTES_PER_MB);

        let settings = load_usage_storage_settings(&connection, &root, outcome.deleted).unwrap();
        assert_eq!(settings.max_database_size_mb, 1);
        assert_eq!(settings.total_records, remaining);
        assert_eq!(settings.deleted_records, outcome.deleted);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repairs_legacy_claude_input_tokens_on_demand() {
        let root = test_root("claude-input-migration");
        let connection = open_test_database(&root);
        connection
            .execute(
                r#"INSERT INTO usage_events (
                    event_key, timestamp, timestamp_ms, local_hour, provider,
                    executor_type, input_tokens, output_tokens, cache_read_tokens,
                    cache_creation_tokens, total_tokens, created_at
                ) VALUES ('legacy-claude', '2026-08-27T00:00:00Z', 1,
                          '2026-08-27T00', 'claude', 'ClaudeExecutor',
                          100, 20, 600, 20, 740, '2026-08-27T00:00:00Z')"#,
                [],
            )
            .unwrap();
        drop(connection);

        let result = repair_usage_cache_records_at(&root).unwrap();
        assert_eq!(result.scanned, 1);
        assert_eq!(result.repaired, 1);
        assert!(result.backup_path.is_some());
        let connection = open_usage_database_at(&root).unwrap();
        let row: (i64, i64, i64) = connection
            .query_row(
                "SELECT input_tokens, total_tokens, cached_tokens FROM usage_events WHERE event_key = 'legacy-claude'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, (720, 740, 620));
        drop(connection);

        let second = repair_usage_cache_records_at(&root).unwrap();
        assert_eq!(second.scanned, 0);
        assert_eq!(second.repaired, 0);
        let rerun_connection = open_usage_database_at(&root).unwrap();
        let rerun_input: i64 = rerun_connection
            .query_row(
                "SELECT input_tokens FROM usage_events WHERE event_key = 'legacy-claude'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(rerun_connection);
        assert_eq!(rerun_input, 720);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removes_historical_unknown_records_on_demand() {
        let root = test_root("unknown-repair");
        let connection = open_test_database(&root);
        for (key, index) in ["refresh-control", "support-refresh-control"]
            .iter()
            .zip([1_i64, 2_i64])
        {
            connection
                .execute(
                    "INSERT INTO usage_events (event_key, timestamp, timestamp_ms, local_hour, provider, model, request_id, input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_creation_tokens, total_tokens, latency_ms, failed, canceled, generate, created_at) VALUES (?1, '2026-08-27T00:00:00Z', ?2, '2026-08-27T00', '', 'unknown', '', 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, '2026-08-27T00:00:00Z')",
                    params![key, index],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO usage_events (event_key, timestamp, timestamp_ms, local_hour, provider, model, request_id, input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_creation_tokens, total_tokens, latency_ms, failed, canceled, generate, created_at) VALUES ('legitimate-unknown', '2026-08-27T00:00:00Z', 3, '2026-08-27T00', '', 'unknown', '', 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, '2026-08-27T00:00:00Z')",
                [],
            )
            .unwrap();
        drop(connection);

        let result = repair_usage_cache_records_at(&root).unwrap();
        assert_eq!(result.scanned, 3);
        assert_eq!(result.repaired, 0);
        assert_eq!(result.deleted, 3);
        let connection = open_usage_database_at(&root).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM usage_events WHERE model = 'unknown'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    fn create_legacy_v2_database(root: &Path) -> Connection {
        fs::create_dir_all(root).unwrap();
        let connection = Connection::open(root.join(USAGE_DATABASE_FILE)).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE usage_metadata (
                    key TEXT PRIMARY KEY NOT NULL,
                    value TEXT NOT NULL
                );
                CREATE TABLE usage_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    event_key TEXT NOT NULL UNIQUE,
                    timestamp TEXT NOT NULL,
                    timestamp_ms INTEGER NOT NULL,
                    local_hour TEXT NOT NULL,
                    latency_ms INTEGER NOT NULL DEFAULT 0,
                    ttft_ms INTEGER,
                    source TEXT NOT NULL DEFAULT '',
                    auth_index TEXT NOT NULL DEFAULT '',
                    failed INTEGER NOT NULL DEFAULT 0,
                    provider TEXT NOT NULL DEFAULT '',
                    model TEXT NOT NULL DEFAULT '',
                    alias TEXT NOT NULL DEFAULT '',
                    reasoning_effort TEXT NOT NULL DEFAULT '',
                    service_tier TEXT NOT NULL DEFAULT '',
                    response_service_tier TEXT NOT NULL DEFAULT '',
                    executor_type TEXT NOT NULL DEFAULT '',
                    endpoint TEXT NOT NULL DEFAULT '',
                    auth_type TEXT NOT NULL DEFAULT '',
                    api_key_hash TEXT NOT NULL DEFAULT '',
                    api_key_display TEXT NOT NULL DEFAULT '',
                    api_key_remark TEXT NOT NULL DEFAULT '',
                    request_id TEXT NOT NULL DEFAULT '',
                    input_tokens INTEGER NOT NULL DEFAULT 0,
                    output_tokens INTEGER NOT NULL DEFAULT 0,
                    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
                    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                    total_tokens INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL
                );
                PRAGMA user_version = 2;
                "#,
            )
            .unwrap();
        connection
    }

    fn sample_record(id: &str, timestamp: &str, model: &str) -> UsageRecord {
        UsageRecord {
            id: id.to_string(),
            row_id: None,
            timestamp: timestamp.to_string(),
            latency_ms: 100,
            ttft_ms: Some(20),
            source: "source".to_string(),
            source_display: String::new(),
            auth_index: "auth".to_string(),
            failed: false,
            canceled: false,
            failure_status: 0,
            failure_body: String::new(),
            provider: "openai".to_string(),
            api_group_key: "hash".to_string(),
            model: model.to_string(),
            response_model: String::new(),
            cost: None,
            alias: String::new(),
            client_ip: None,
            x_forwarded_for: None,
            user_agent: None,
            reasoning_effort: "high".to_string(),
            service_tier: String::new(),
            response_service_tier: String::new(),
            executor_type: String::new(),
            endpoint: "POST /v1/responses".to_string(),
            auth_type: "oauth".to_string(),
            api_key_hash: "hash".to_string(),
            api_key_display: "12••••".to_string(),
            api_key_remark: "Built-in key".to_string(),
            request_id: id.to_string(),
            generate: true,
            cached_tokens: 2,
            collector_source: "test".to_string(),
            tokens: UsageTokenStats {
                input_tokens: 10,
                output_tokens: 20,
                reasoning_tokens: 5,
                cache_read_tokens: 2,
                cache_creation_tokens: 0,
                total_tokens: 30,
            },
        }
    }

    #[test]
    fn masks_api_keys_without_exposing_the_full_value() {
        assert_eq!(mask_api_key("123456"), "12••••");
        assert_eq!(mask_api_key("sk-1234567890"), "sk-1••••7890");
    }

    #[test]
    fn api_key_category_uses_either_remark_or_masked_key() {
        assert_eq!(
            api_key_category_label("Production".to_string(), "12••••".to_string()),
            "Production"
        );
        assert_eq!(
            api_key_category_label(String::new(), "123456".to_string()),
            "12••••"
        );
    }

    #[test]
    fn usage_source_prefers_api_access_remark_and_masks_secret_fallbacks() {
        let key = "sk-1234567890abcdefghijklmnopqrstuvwxyz";
        let mut config = GuiConfigFile::default();
        config.api_access_remarks.push(crate::GuiApiAccessRemark {
            provider_section: "codex-api-key".to_string(),
            api_key_hash: hash_text(key),
            record_hash: String::new(),
            remark: "Production".to_string(),
        });

        assert_eq!(usage_source_display(&config, "codex", key), "Production");
        assert_eq!(
            usage_source_display(&GuiConfigFile::default(), "codex", key),
            "sk-1••••wxyz"
        );
        assert_eq!(
            usage_source_display(&config, "codex", "account@example.com"),
            "account@example.com"
        );
    }

    #[test]
    fn usage_source_masks_shared_keys_with_different_record_remarks() {
        let key = "sk-1234567890abcdefghijklmnopqrstuvwxyz";
        let mut config = GuiConfigFile::default();
        for (record_hash, remark) in [
            ("a".repeat(64), "Production"),
            ("b".repeat(64), "Test"),
        ] {
            config.api_access_remarks.push(crate::GuiApiAccessRemark {
                provider_section: "codex-api-key".to_string(),
                api_key_hash: hash_text(key),
                record_hash,
                remark: remark.to_string(),
            });
        }

        assert_eq!(
            usage_source_display(&config, "codex", key),
            "sk-1••••wxyz"
        );
    }

    #[test]
    fn local_hour_key_uses_year_month_day_and_hour() {
        let record = sample_record("id", "2026-07-17T20:30:00+08:00", "model");
        assert_eq!(record_local_hour(&record).len(), "2026-07-17-20".len());
    }

    #[test]
    fn normalizes_queue_records_without_persisting_secrets_or_headers() {
        let config = GuiConfigFile {
            locale: "zh-CN".to_string(),
            port: 8317,
            allow_lan: false,
            host: "127.0.0.1".to_string(),
            run_on_startup: false,
            start_core_on_launch: true,
            silent_start: false,
            close_behavior: crate::WindowsCloseBehavior::Ask,
            default_terminal: crate::DEFAULT_AGENT_TERMINAL.to_string(),
            window_width: None,
            window_height: None,
            auth_dir: String::new(),
            auth_dir_user_selected: false,
            api_keys: Vec::new(),
            api_access_remarks: Vec::new(),
            management_secret_key: "123456".to_string(),
            debug: false,
            commercial_mode: false,
            logging_to_file: false,
            logs_max_total_size_mb: crate::DEFAULT_LOGS_MAX_TOTAL_SIZE_MB,
            error_logs_max_files: crate::DEFAULT_ERROR_LOGS_MAX_FILES,
            usage_statistics_enabled: true,
            usage_statistics_disabled: None,
            redis_usage_queue_retention_seconds: crate::DEFAULT_REDIS_USAGE_QUEUE_RETENTION_SECONDS,
            request_log: false,
            plugins_enabled: false,
            routing_strategy: "round-robin".to_string(),
            proxy_url: String::new(),
            proxy_override: false,
            download_source: VersionDownloadSource::Github,
            custom_download_mirrors: Vec::new(),
            active_custom_download_mirror: String::new(),
            prefer_gitcode_downloads: false,
            routing_session_affinity: false,
            routing_session_affinity_ttl: String::new(),
            disable_cooling: crate::DEFAULT_DISABLE_COOLING,
            request_retry: crate::DEFAULT_REQUEST_RETRY,
            max_retry_credentials: crate::DEFAULT_MAX_RETRY_CREDENTIALS,
            max_retry_interval: crate::DEFAULT_MAX_RETRY_INTERVAL,
            streaming_bootstrap_retries: crate::DEFAULT_STREAMING_BOOTSTRAP_RETRIES,
        };
        let record = normalize_usage_record(
            serde_json::json!({
                "timestamp": "2026-07-17T20:30:00+08:00",
                "request_id": "request-1",
                "api_key": "secret-client-key",
                "response_headers": { "authorization": ["secret-upstream-token"] },
                "model": "gpt-test",
                "tokens": {
                    "input_tokens": 10,
                    "output_tokens": 20,
                    "cached_tokens": 7,
                    "cache_read_tokens": 5,
                    "cache_creation_tokens": 2
                }
            }),
            &config,
        )
        .unwrap();
        let rendered = serde_json::to_string(&record).unwrap();

        assert!(!rendered.contains("secret-client-key"));
        assert!(!rendered.contains("secret-upstream-token"));
        assert_eq!(record.tokens.total_tokens, 30);
        assert_eq!(record.tokens.cache_read_tokens, 5);
        assert!(!record.api_key_hash.is_empty());
    }

    #[test]
    fn normalizes_response_model_when_present_and_defaults_when_missing() {
        let config = GuiConfigFile::default();
        for (response_model, expected) in [
            (None, ""),
            (Some(serde_json::json!(null)), ""),
            (Some(serde_json::json!("  ")), ""),
            (
                Some(serde_json::json!(" gemini-3.8-flash ")),
                "gemini-3.8-flash",
            ),
        ] {
            let mut value = serde_json::json!({
                "request_id": "response-model-normalization",
                "model": "gemini-3.8-flash-high",
                "alias": "client-alias"
            });
            if let Some(response_model) = response_model {
                value["response_model"] = response_model;
            }
            let record = normalize_usage_record(value, &config).unwrap();

            assert_eq!(record.response_model, expected);
            assert_eq!(record.model, "gemini-3.8-flash-high");
            assert_eq!(record.alias, "client-alias");
            assert_eq!(
                serde_json::to_value(&record).unwrap()["response_model"],
                expected
            );
        }
    }

    #[test]
    fn response_model_json_roundtrip_accepts_legacy_records_without_the_field() {
        let mut record = sample_record("response-json", "2026-07-17T20:30:00+08:00", "gpt-a");
        record.response_model = "gpt-upstream".to_string();
        let mut encoded = serde_json::to_value(&record).unwrap();
        let decoded: UsageRecord = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded.response_model, "gpt-upstream");

        encoded.as_object_mut().unwrap().remove("response_model");
        let legacy: UsageRecord = serde_json::from_value(encoded).unwrap();
        assert!(legacy.response_model.is_empty());
        assert_eq!(legacy.model, "gpt-a");
    }

    #[test]
    fn response_model_survives_sqlite_roundtrip() {
        let root = test_root("response-model-roundtrip");
        let mut connection = open_test_database(&root);
        let mut record = sample_record(
            "response-model-roundtrip",
            "2026-07-17T20:30:00+08:00",
            "gemini-alias",
        );
        record.response_model = "gemini-3.8-flash-high".to_string();
        insert_usage_records(&mut connection, &[record]).unwrap();
        let page = load_usage_events(
            &connection,
            &UsageQuery::default(),
            &GuiConfigFile::default(),
        )
        .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].response_model, "gemini-3.8-flash-high");
        assert_eq!(page.items[0].model, "gemini-alias");
        assert_eq!(page.items[0].tokens.total_tokens, 30);
        assert_eq!(
            serde_json::to_value(&page.items[0]).unwrap()["response_model"],
            "gemini-3.8-flash-high"
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn response_model_migration_preserves_v5_records_and_is_idempotent() {
        let root = test_root("response-model-v6-migration");
        let mut connection = open_test_database(&root);
        let record = sample_record("legacy-v5", "2026-07-17T20:30:00+08:00", "gpt-a");
        insert_usage_records(&mut connection, &[record]).unwrap();
        connection
            .execute_batch(
                "ALTER TABLE usage_events DROP COLUMN response_model; PRAGMA user_version = 5;",
            )
            .unwrap();
        assert!(!usage_table_columns(&connection, "usage_events")
            .unwrap()
            .contains("response_model"));
        drop(connection);

        initialize_usage_storage_at(&root).unwrap();
        let mut connection = open_usage_database_at(&root).unwrap();
        let config = GuiConfigFile::default();
        let legacy_page = load_usage_events(&connection, &UsageQuery::default(), &config).unwrap();
        assert_eq!(legacy_page.total, 1);
        assert!(legacy_page.items[0].response_model.is_empty());
        assert_eq!(legacy_page.items[0].tokens.total_tokens, 30);

        let mut record = sample_record("new-v6", "2026-07-17T20:31:00+08:00", "gpt-b");
        record.response_model = "gpt-upstream".to_string();
        insert_usage_records(&mut connection, &[record]).unwrap();
        drop(connection);

        initialize_usage_storage_at(&root).unwrap();
        let connection = open_usage_database_at(&root).unwrap();
        let page = load_usage_events(&connection, &UsageQuery::default(), &config).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items[0].response_model, "gpt-upstream");
        assert!(page.items[1].response_model.is_empty());
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            USAGE_DATABASE_SCHEMA_VERSION
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preserves_explicit_cache_read_and_only_backfills_missing_alias() {
        let config = GuiConfigFile::default();
        let explicit = normalize_usage_record(
            serde_json::json!({
                "request_id": "explicit-cache-read",
                "tokens": {
                    "input_tokens": 100,
                    "cached_tokens": 10,
                    "cache_read_tokens": 5,
                    "cache_creation_tokens": 2,
                    "total_tokens": 100
                }
            }),
            &config,
        )
        .unwrap();
        assert_eq!(explicit.tokens.cache_read_tokens, 5);

        let legacy = normalize_usage_record(
            serde_json::json!({
                "request_id": "legacy-cache-alias",
                "tokens": {
                    "input_tokens": 100,
                    "cached_tokens": 10,
                    "cache_creation_tokens": 2,
                    "total_tokens": 100
                }
            }),
            &config,
        )
        .unwrap();
        assert_eq!(legacy.tokens.cache_read_tokens, 10);
    }

    #[test]
    fn normalizes_claude_input_to_include_cache_tokens() {
        let record = normalize_usage_record(
            serde_json::json!({
                "executor_type": "ClaudeExecutor",
                "provider": "anthropic",
                "request_id": "claude-cache-rate",
                "tokens": {
                    "input_tokens": 100,
                    "output_tokens": 20,
                    "cache_read_tokens": 600,
                    "cache_creation_tokens": 20,
                    "total_tokens": 120
                }
            }),
            &GuiConfigFile::default(),
        )
        .unwrap();

        assert_eq!(record.tokens.input_tokens, 720);
        assert_eq!(record.tokens.total_tokens, 740);
        assert_eq!(record.tokens.cache_read_tokens, 600);
    }

    #[test]
    fn legacy_claude_identity_uses_keeper_missing_cache_contract() {
        let record = normalize_usage_record(
            serde_json::json!({
                "provider": "anthropic",
                "auth_type": "oauth",
                "request_id": "claude-inclusive",
                "tokens": {
                    "input_tokens": 720,
                    "output_tokens": 20,
                    "cache_read_tokens": 600,
                    "cache_creation_tokens": 20,
                    "total_tokens": 740
                }
            }),
            &GuiConfigFile::default(),
        )
        .unwrap();
        assert_eq!(record.tokens.input_tokens, 1_340);
        assert_eq!(record.tokens.total_tokens, 1_360);
    }

    #[test]
    fn unresolved_executors_only_use_provider_for_oauth() {
        for executor in ["", "unknown", "FutureExecutor"] {
            for auth_type in ["", "apikey", "api_key", "unknown", " OAUTH "] {
                let record = normalize_usage_record(
                    serde_json::json!({
                        "request_id": "unresolved-executor",
                        "executor_type": executor,
                        "provider": "anthropic",
                        "auth_type": auth_type,
                        "auth_index": "missing-identity",
                        "tokens": {
                            "input_tokens": 100,
                            "output_tokens": 20,
                            "cache_read_tokens": 30,
                            "total_tokens": 120
                        }
                    }),
                    &GuiConfigFile::default(),
                )
                .unwrap();
                let expected_input = if auth_type.trim().eq_ignore_ascii_case("oauth") {
                    130
                } else {
                    100
                };
                assert_eq!(
                    record.tokens.input_tokens, expected_input,
                    "{executor}/{auth_type}"
                );
                assert_eq!(
                    record.tokens.total_tokens,
                    expected_input + 20,
                    "{executor}/{auth_type}"
                );
            }
        }
    }

    #[test]
    fn codex_cached_only_records_preserve_total_with_missing_or_zero_read() {
        for explicit_read in [false, true] {
            let mut tokens = serde_json::json!({
                "input_tokens": 0,
                "output_tokens": 0,
                "reasoning_tokens": 0,
                "cached_tokens": 30,
                "cache_creation_tokens": 0,
                "total_tokens": 30
            });
            if explicit_read {
                tokens["cache_read_tokens"] = serde_json::json!(0);
            }
            let record = normalize_usage_record(
                serde_json::json!({
                    "request_id": "codex-cached-only",
                    "executor_type": "CodexExecutor",
                    "tokens": tokens
                }),
                &GuiConfigFile::default(),
            )
            .unwrap();
            assert_eq!(
                record.tokens.total_tokens, 30,
                "explicit_read={explicit_read}"
            );
            assert_eq!(record.tokens.input_tokens, 0);
            assert_eq!(record.tokens.output_tokens, 0);
            assert_eq!(
                record.tokens.cache_read_tokens,
                if explicit_read { 0 } else { 30 }
            );
        }
    }

    #[test]
    fn clamped_contract_fields_do_not_rewrite_nonzero_total() {
        for executor in ["ClaudeExecutor", "GeminiExecutor", "CodexExecutor"] {
            for field in [
                "input_tokens",
                "output_tokens",
                "reasoning_tokens",
                "cache_read_tokens",
                "cache_creation_tokens",
            ] {
                let mut tokens = serde_json::json!({
                    "input_tokens": 10,
                    "output_tokens": 5,
                    "total_tokens": 99
                });
                tokens[field] = serde_json::json!(-1);
                let record = normalize_usage_record(
                    serde_json::json!({
                        "request_id": "clamped-contract",
                        "executor_type": executor,
                        "tokens": tokens
                    }),
                    &GuiConfigFile::default(),
                )
                .unwrap();
                assert_eq!(record.tokens.total_tokens, 99, "{executor}/{field}");
            }
        }
    }

    #[test]
    fn zero_total_reconciliation_respects_keeper_clamped_field_dependencies() {
        for executor in [
            "ClaudeExecutor",
            "GeminiExecutor",
            "CodexExecutor",
            "OpenAICompatExecutor",
            "KimiExecutor",
            "unknown",
        ] {
            for field in [
                "input_tokens",
                "output_tokens",
                "total_tokens",
                "reasoning_tokens",
                "cache_read_tokens",
                "cache_creation_tokens",
                "cached_tokens",
            ] {
                let mut tokens = serde_json::json!({
                    "input_tokens": 10,
                    "output_tokens": 5,
                    "total_tokens": 0
                });
                tokens[field] = serde_json::json!(-1);
                let record = normalize_usage_record(
                    serde_json::json!({
                        "request_id": "clamped-zero-fallback",
                        "executor_type": executor,
                        "tokens": tokens
                    }),
                    &GuiConfigFile::default(),
                )
                .unwrap();
                let expected = if matches!(field, "input_tokens" | "output_tokens" | "total_tokens")
                    || (executor == "ClaudeExecutor" && field == "reasoning_tokens")
                {
                    0
                } else {
                    15
                };
                assert_eq!(record.tokens.total_tokens, expected, "{executor}/{field}");
            }
        }
    }

    #[test]
    fn clamped_fields_cannot_trigger_dependent_token_folds() {
        let cases = [
            (
                "ClaudeExecutor",
                "anthropic",
                serde_json::json!({
                    "input_tokens": 10, "output_tokens": 5, "cache_read_tokens": -1,
                    "cache_creation_tokens": 30, "total_tokens": 99
                }),
                10,
                5,
            ),
            (
                "GeminiExecutor",
                "gemini",
                serde_json::json!({
                    "input_tokens": 10, "output_tokens": -1, "reasoning_tokens": 5,
                    "total_tokens": 99
                }),
                10,
                0,
            ),
            (
                "",
                "gemini",
                serde_json::json!({
                    "input_tokens": -1, "output_tokens": 5, "reasoning_tokens": 10,
                    "total_tokens": 15
                }),
                0,
                5,
            ),
            (
                "OpenAICompatExecutor",
                "openai",
                serde_json::json!({
                    "input_tokens": -1, "output_tokens": 5, "reasoning_tokens": 10,
                    "total_tokens": 15
                }),
                0,
                5,
            ),
        ];
        for (executor, provider, tokens, input, output) in cases {
            let record = normalize_usage_record(
                serde_json::json!({
                    "request_id": "clamped-fold",
                    "executor_type": executor,
                    "provider": provider,
                    "auth_type": "oauth",
                    "tokens": tokens
                }),
                &GuiConfigFile::default(),
            )
            .unwrap();
            assert_eq!(record.tokens.input_tokens, input, "{executor}/{provider}");
            assert_eq!(record.tokens.output_tokens, output, "{executor}/{provider}");
        }
    }

    #[test]
    fn clamped_cache_read_is_not_backfilled_from_cached_alias() {
        for executor in ["CodexExecutor", "KimiExecutor", "unknown"] {
            let record = normalize_usage_record(
                serde_json::json!({
                    "request_id": "clamped-cache-alias",
                    "executor_type": executor,
                    "tokens": {
                        "input_tokens": 100,
                        "output_tokens": 20,
                        "cache_read_tokens": -1,
                        "cached_tokens": 30,
                        "total_tokens": 120
                    }
                }),
                &GuiConfigFile::default(),
            )
            .unwrap();
            assert_eq!(record.tokens.cache_read_tokens, 0, "{executor}");
        }
    }

    #[test]
    fn unknown_producers_preserve_nonzero_parent_token_fields() {
        let record = normalize_usage_record(
            serde_json::json!({
                "provider": "custom",
                "request_id": "unknown-cache-shape",
                "tokens": {
                    "input_tokens": 100,
                    "output_tokens": 20,
                    "cache_read_tokens": 600,
                    "total_tokens": 120
                }
            }),
            &GuiConfigFile::default(),
        )
        .unwrap();
        assert_eq!(record.tokens.input_tokens, 100);
        assert_eq!(record.tokens.total_tokens, 120);
        assert_eq!(record.tokens.cache_read_tokens, 600);
    }

    #[test]
    fn filters_usage_control_messages_before_inbox() {
        assert!(is_ignorable_usage_message(""));
        assert!(is_ignorable_usage_message("  null\n"));
        assert!(is_ignorable_usage_message(r#"{"refresh":true}"#));
        assert!(is_ignorable_usage_message(
            r#"{ "support_refresh" : true }"#
        ));
        assert!(!is_ignorable_usage_message(r#"{"refresh":false}"#));
        assert!(!is_ignorable_usage_message(
            r#"{"refresh":true,"request_id":"usage"}"#
        ));
        assert!(!is_ignorable_usage_message(r#"{"request_id":"usage"}"#));
        assert!(!is_ignorable_usage_message("not-json"));

        let root = test_root("usage-control-messages");
        initialize_usage_storage_at(&root).unwrap();
        let config = GuiConfigFile::default();
        let inserted = persist_queue_items_from_source(
            &root,
            HTTP_USAGE_QUEUE_SOURCE,
            vec![
                serde_json::json!({"refresh": true}),
                serde_json::json!({"support_refresh": true}),
                serde_json::json!({
                    "request_id": "real-usage",
                    "provider": "codex",
                    "model": "gpt-test"
                }),
            ],
            &config,
        )
        .unwrap();
        assert_eq!(inserted.inserted, 1);
        let connection = open_usage_database_at(&root).unwrap();
        let inbox_count = connection
            .query_row("SELECT COUNT(*) FROM usage_inbox", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let event_count = connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(inbox_count, 1);
        assert_eq!(event_count, 1);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_usage_messages_without_request_id() {
        let root = test_root("usage-missing-request-id");
        initialize_usage_storage_at(&root).unwrap();
        let mut connection = open_usage_database_at(&root).unwrap();
        enqueue_usage_queue_items(
            &mut connection,
            "redis_subscribe:usage",
            vec![serde_json::json!({
                "provider": "codex",
                "model": "gpt-test"
            })],
        )
        .unwrap();
        let inserted = process_usage_inbox(&mut connection, &GuiConfigFile::default()).unwrap();
        let event_count = connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let status = connection
            .query_row("SELECT status FROM usage_inbox", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap();
        assert_eq!(inserted.inserted, 0);
        assert_eq!(event_count, 0);
        assert_eq!(status, "decode_failed");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persists_successful_health_checks_and_zero_token_websocket_events() {
        let root = test_root("queue-events-without-generation");
        initialize_usage_storage_at(&root).unwrap();
        let config = GuiConfigFile::default();
        let inserted = persist_queue_items_from_source(
            &root,
            HTTP_USAGE_QUEUE_SOURCE,
            vec![
                serde_json::json!({
                    "timestamp": "2026-07-29T10:00:00+08:00",
                    "request_id": "cherry-health-check",
                    "generate": false,
                    "failed": false,
                    "provider": "openai",
                    "model": "gpt-test",
                    "endpoint": "POST /v1/chat/completions",
                    "tokens": {
                        "input_tokens": 1,
                        "output_tokens": 1,
                        "total_tokens": 2
                    }
                }),
                serde_json::json!({
                    "timestamp": "2026-07-29T10:00:01+08:00",
                    "request_id": "websocket-zero-token",
                    "failed": false,
                    "provider": "codex",
                    "model": "gpt-test",
                    "executor_type": "CodexWebsocketsExecutor",
                    "tokens": {
                        "input_tokens": 0,
                        "output_tokens": 0,
                        "total_tokens": 0
                    }
                }),
            ],
            &config,
        )
        .unwrap();
        let connection = open_usage_database_at(&root).unwrap();
        let events = load_usage_events(
            &connection,
            &UsageQuery {
                failed: Some(false),
                ..UsageQuery::default()
            },
            &config,
        )
        .unwrap();

        assert_eq!(inserted.inserted, 2);
        assert_eq!(events.total, 2);
        assert!(events.items.iter().all(|event| !event.failed));
        assert!(events
            .items
            .iter()
            .any(|event| event.request_id == "cherry-health-check"));
        assert!(events
            .items
            .iter()
            .any(|event| event.request_id == "websocket-zero-token"));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persists_failure_details_and_excludes_client_cancellations_from_failures() {
        let root = test_root("usage-failure-details");
        initialize_usage_storage_at(&root).unwrap();
        let config = GuiConfigFile::default();
        persist_queue_items_from_source(
            &root,
            HTTP_USAGE_QUEUE_SOURCE,
            vec![
                serde_json::json!({
                    "timestamp": "2026-07-29T10:00:00+08:00",
                    "request_id": "success",
                    "failed": false,
                    "provider": "antigravity",
                    "model": "gemini-test"
                }),
                serde_json::json!({
                    "timestamp": "2026-07-29T10:00:01+08:00",
                    "request_id": "upstream-failure",
                    "failed": true,
                    "provider": "antigravity",
                    "model": "gemini-test",
                    "fail": {
                        "status_code": 429,
                        "body": { "error": { "message": "quota exhausted" } }
                    }
                }),
                serde_json::json!({
                    "timestamp": "2026-07-29T10:00:02+08:00",
                    "request_id": "client-canceled",
                    "failed": true,
                    "provider": "antigravity",
                    "model": "gemini-test",
                    "fail": {
                        "status_code": 499,
                        "body": "context canceled"
                    }
                }),
            ],
            &config,
        )
        .unwrap();

        let connection = open_usage_database_at(&root).unwrap();
        let overview = load_usage_overview(&connection, &UsageQuery::default()).unwrap();
        let failures = load_usage_events(
            &connection,
            &UsageQuery {
                failed: Some(true),
                ..UsageQuery::default()
            },
            &config,
        )
        .unwrap();
        let cancellations = load_usage_events(
            &connection,
            &UsageQuery {
                canceled: Some(true),
                ..UsageQuery::default()
            },
            &config,
        )
        .unwrap();

        assert_eq!(overview.total_requests, 3);
        assert_eq!(overview.success_count, 1);
        assert_eq!(overview.failure_count, 1);
        assert_eq!(overview.canceled_count, 1);
        assert_eq!(overview.success_rate, 50.0);
        assert_eq!(failures.total, 1);
        assert_eq!(failures.items[0].request_id, "upstream-failure");
        assert_eq!(failures.items[0].failure_status, 429);
        assert!(failures.items[0].failure_body.contains("quota exhausted"));
        assert_eq!(cancellations.total, 1);
        assert_eq!(cancellations.items[0].request_id, "client-canceled");
        assert!(cancellations.items[0].canceled);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_storage_uses_wal_and_reference_busy_timeout() {
        let root = test_root("sqlite-pragmas");
        let connection = open_test_database(&root);
        let journal_mode = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap();
        let busy_timeout = connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap();
        let foreign_keys = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .unwrap();

        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(busy_timeout, 5_000);
        assert_eq!(foreign_keys, 1);
        assert!(root.join(USAGE_DATABASE_FILE).is_file());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_v2_database_is_backed_up_and_migrated_once() {
        let root = test_root("keeper-v3-migration");
        let connection = create_legacy_v2_database(&root);
        connection
            .execute(
                r#"
                INSERT INTO usage_events (
                    event_key, timestamp, timestamp_ms, local_hour, latency_ms, ttft_ms,
                    source, auth_index, failed, provider, model, alias, reasoning_effort,
                    service_tier, response_service_tier, executor_type, endpoint, auth_type,
                    api_key_hash, api_key_display, api_key_remark, request_id,
                    input_tokens, output_tokens, reasoning_tokens, cache_read_tokens,
                    cache_creation_tokens, total_tokens, created_at
                ) VALUES (
                    'request-1', '2026-07-17T20:30:00+08:00', 1784291400000,
                    '2026-07-17-20', 100, 20, 'source', 'auth', 0, 'openai',
                    'gpt-test', 'alias-test', 'high', '', '', '',
                    'POST /v1/responses', 'oauth', 'legacy-hash', '12••••',
                    'Legacy key', 'request-1', 10, 20, 5, 2, 3, 30,
                    '2026-07-17T20:30:01+08:00'
                )
                "#,
                [],
            )
            .unwrap();
        drop(connection);

        initialize_usage_storage_at(&root).unwrap();
        initialize_usage_storage_at(&root).unwrap();
        let connection = open_usage_database_at(&root).unwrap();
        let user_version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap();
        let response_model_column = usage_table_columns(&connection, "usage_events")
            .unwrap()
            .contains("response_model");
        let migrated = connection
            .query_row(
                r#"
                SELECT COUNT(*), SUM(total_tokens), api_group_key, model_alias,
                       cached_tokens, collector_source
                FROM usage_events
                "#,
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .unwrap();
        let marker = connection
            .query_row(
                "SELECT value FROM usage_metadata WHERE key = ?1",
                params![USAGE_DATABASE_MIGRATION_KEY],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let backups = fs::read_dir(root.join(USAGE_BACKUP_DIR_NAME))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(user_version, USAGE_DATABASE_SCHEMA_VERSION);
        assert!(response_model_column);
        assert_eq!(migrated.0, 1);
        assert_eq!(migrated.1, 30);
        assert_eq!(migrated.2, "legacy-hash");
        assert_eq!(migrated.3.as_deref(), Some("alias-test"));
        assert_eq!(migrated.4, 5);
        assert_eq!(migrated.5, "legacy_migration");
        assert!(!marker.is_empty());
        assert_eq!(backups.len(), 1);

        let duplicate_inserted = connection
            .execute(
                r#"
                INSERT INTO usage_events (
                    event_key, timestamp, timestamp_ms, local_hour, created_at
                ) VALUES (
                    'request-1', '2026-07-17T20:31:00+08:00', 1784291460000,
                    '2026-07-17-20', '2026-07-17T20:31:01+08:00'
                )
                "#,
                [],
            )
            .unwrap();
        assert_eq!(duplicate_inserted, 1);
        let duplicate_count = connection
            .query_row(
                "SELECT COUNT(*) FROM usage_events WHERE event_key = 'request-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(duplicate_count, 2);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failure_detail_migration_backfills_processed_inbox_rows_once() {
        let root = test_root("failure-detail-v4-migration");
        let mut connection = open_test_database(&root);
        let mut record = sample_record(
            "legacy-canceled",
            "2026-07-17T20:30:00+08:00",
            "gemini-test",
        );
        record.failed = true;
        insert_usage_records(&mut connection, &[record]).unwrap();
        connection
            .execute(
                r#"
                INSERT INTO usage_inbox (
                    source, message_hash, raw_message, status, attempt_count,
                    usage_event_key, received_at, processed_at, created_at, updated_at
                ) VALUES (
                    'test', 'legacy-canceled-hash', ?1, 'processed', 1,
                    'legacy-canceled', ?2, ?2, ?2, ?2
                )
                "#,
                params![
                    serde_json::json!({
                        "request_id": "legacy-canceled",
                        "failed": true,
                        "fail": {
                            "status_code": 499,
                            "body": "client closed request"
                        }
                    })
                    .to_string(),
                    Local::now().to_rfc3339(),
                ],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM usage_metadata WHERE key = ?1",
                params![USAGE_FAILURE_MIGRATION_KEY],
            )
            .unwrap();
        drop(connection);

        initialize_usage_storage_at(&root).unwrap();
        initialize_usage_storage_at(&root).unwrap();
        let connection = open_usage_database_at(&root).unwrap();
        let migrated = connection
            .query_row(
                r#"
                SELECT canceled, failure_status, failure_body
                FROM usage_events
                WHERE event_key = 'legacy-canceled'
                "#,
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        let markers = connection
            .query_row(
                "SELECT COUNT(*) FROM usage_metadata WHERE key = ?1",
                params![USAGE_FAILURE_MIGRATION_KEY],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();

        assert_eq!(migrated.0, 1);
        assert_eq!(migrated.1, 499);
        assert_eq!(migrated.2, "client closed request");
        assert_eq!(markers, 1);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn durable_inbox_recovers_valid_rows_and_isolates_malformed_rows() {
        let root = test_root("durable-inbox");
        initialize_usage_storage_at(&root).unwrap();
        let mut connection = open_usage_database_at(&root).unwrap();
        enqueue_usage_queue_items(
            &mut connection,
            "http_pull",
            vec![
                serde_json::json!({
                    "timestamp": "2026-07-17T20:30:00+08:00",
                    "request_id": "request-1",
                    "model": "gpt-test",
                    "tokens": { "input_tokens": 10, "output_tokens": 20 }
                }),
                serde_json::json!("not-an-object"),
            ],
        )
        .unwrap();
        drop(connection);

        let mut connection = open_usage_database_at(&root).unwrap();
        let inserted = process_usage_inbox(&mut connection, &GuiConfigFile::default()).unwrap();
        let event_count = connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let processed_count = connection
            .query_row(
                "SELECT COUNT(*) FROM usage_inbox WHERE status = 'processed'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        let failed_count = connection
            .query_row(
                "SELECT COUNT(*) FROM usage_inbox WHERE status = 'decode_failed'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();

        assert_eq!(inserted.inserted, 1);
        assert_eq!(event_count, 1);
        assert_eq!(processed_count, 1);
        assert_eq!(failed_count, 1);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn durable_inbox_preserves_multiple_events_with_same_request_id() {
        let root = test_root("durable-inbox-duplicate-request-id");
        initialize_usage_storage_at(&root).unwrap();
        let mut connection = open_usage_database_at(&root).unwrap();
        enqueue_usage_queue_items(
            &mut connection,
            "redis_subscribe:usage",
            vec![
                serde_json::json!({
                    "timestamp": "2026-07-17T20:30:00+08:00",
                    "request_id": "persistent-connection",
                    "model": "gpt-first",
                    "tokens": { "input_tokens": 10, "output_tokens": 20, "total_tokens": 30 }
                }),
                serde_json::json!({
                    "timestamp": "2026-07-17T20:31:00+08:00",
                    "request_id": "persistent-connection",
                    "model": "gpt-second",
                    "tokens": { "input_tokens": 30, "output_tokens": 40, "total_tokens": 70 }
                }),
            ],
        )
        .unwrap();

        let inserted = process_usage_inbox(&mut connection, &GuiConfigFile::default()).unwrap();
        assert_eq!(inserted.inserted, 2);
        let event_count = connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(event_count, 2);

        let rows = {
            let mut statement = connection
                .prepare("SELECT event_key, model, total_tokens FROM usage_events ORDER BY id")
                .unwrap();
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(
            rows,
            vec![
                (
                    "persistent-connection".to_string(),
                    "gpt-first".to_string(),
                    30
                ),
                (
                    "persistent-connection".to_string(),
                    "gpt-second".to_string(),
                    70
                ),
            ]
        );

        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inbox_cleanup_preserves_pending_and_recent_failure_rows() {
        let root = test_root("inbox-cleanup");
        let connection = open_test_database(&root);
        let now = Local::now();
        let old_processed = (now - chrono::Duration::days(1)).to_rfc3339();
        let recent_failure = (now - chrono::Duration::days(1)).to_rfc3339();
        let old_failure = (now - chrono::Duration::days(8)).to_rfc3339();
        for (status, processed_at, updated_at) in [
            (
                "processed",
                Some(old_processed.as_str()),
                old_processed.as_str(),
            ),
            ("pending", None, old_failure.as_str()),
            (
                "decode_failed",
                Some(recent_failure.as_str()),
                recent_failure.as_str(),
            ),
            (
                "discarded",
                Some(old_failure.as_str()),
                old_failure.as_str(),
            ),
        ] {
            connection
                .execute(
                    r#"
                    INSERT INTO usage_inbox (
                        source, message_hash, raw_message, status, attempt_count,
                        received_at, processed_at, created_at, updated_at
                    ) VALUES ('test', 'hash', '{}', ?1, 0, ?3, ?2, ?3, ?3)
                    "#,
                    params![status, processed_at, updated_at],
                )
                .unwrap();
        }

        cleanup_usage_inbox(&connection, now).unwrap();
        let statuses = {
            let mut statement = connection
                .prepare("SELECT status FROM usage_inbox ORDER BY id")
                .unwrap();
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };

        assert_eq!(statuses, vec!["pending", "decode_failed"]);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_storage_preserves_duplicate_event_keys_transactionally() {
        let root = test_root("sqlite-duplicate-event-keys");
        let mut connection = open_test_database(&root);
        let first = sample_record("request-1", "2026-07-17T20:30:00+08:00", "gpt-test");
        let second = sample_record("request-1", "2026-07-17T20:31:00+08:00", "gpt-test-updated");

        assert_eq!(
            insert_usage_records(&mut connection, std::slice::from_ref(&first)).unwrap(),
            1
        );
        assert_eq!(insert_usage_records(&mut connection, &[second]).unwrap(), 1);
        let count = connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();

        assert_eq!(count, 2);
        let events = load_usage_events(
            &connection,
            &UsageQuery::default(),
            &GuiConfigFile::default(),
        )
        .unwrap();
        assert_eq!(events.items.len(), 2);
        assert_eq!(events.items[0].id, events.items[1].id);
        assert_ne!(events.items[0].row_id, events.items[1].row_id);
        let serialized = serde_json::to_value(&events).unwrap();
        assert!(serialized["items"][0]["row_id"].as_str().is_some());
        assert_ne!(
            serialized["items"][0]["row_id"],
            serialized["items"][1]["row_id"]
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn current_v3_database_migrates_unique_event_key_without_losing_rows() {
        let root = test_root("event-key-v5-migration");
        let mut connection = open_test_database(&root);
        let table_sql = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'usage_events'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let indexes = {
            let mut statement = connection
                .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'usage_events'")
                .unwrap();
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        for index in indexes {
            connection
                .execute(
                    &format!("DROP INDEX {}", quote_sqlite_identifier(&index)),
                    [],
                )
                .unwrap();
        }
        connection
            .execute(
                "ALTER TABLE usage_events RENAME TO usage_events_previous",
                [],
            )
            .unwrap();
        let unique_table_sql = replace_sql_fragment_case_insensitive(
            &table_sql,
            "event_key TEXT NOT NULL,",
            "event_key TEXT NOT NULL UNIQUE,",
        )
        .unwrap();
        connection.execute_batch(&unique_table_sql).unwrap();
        connection
            .execute("DROP TABLE usage_events_previous", [])
            .unwrap();

        migrate_usage_database(&mut connection, &root).unwrap();
        connection
            .execute(
                "INSERT INTO usage_events (event_key, timestamp, timestamp_ms, local_hour, created_at) VALUES ('persistent', '2026-07-17T20:30:00+08:00', 1, '2026-07-17-20', '2026-07-17T20:30:00+08:00'), ('persistent', '2026-07-17T20:31:00+08:00', 2, '2026-07-17-20', '2026-07-17T20:31:00+08:00')",
                [],
            )
            .unwrap();
        let count = connection
            .query_row(
                "SELECT COUNT(*) FROM usage_events WHERE event_key = 'persistent'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM usage_metadata WHERE key = ?1",
                    params![USAGE_EVENT_KEY_MIGRATION_KEY],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_hour_and_inbox_json_are_migrated_once() {
        let root = test_root("legacy-migration");
        let events_dir = root.join(LEGACY_USAGE_EVENTS_DIR);
        let inbox_dir = root.join(LEGACY_USAGE_INBOX_DIR);
        fs::create_dir_all(&events_dir).unwrap();
        fs::create_dir_all(&inbox_dir).unwrap();
        let mut first = serde_json::to_value(sample_record(
            "request-1",
            "2026-07-17T20:30:00+08:00",
            "gpt-a",
        ))
        .unwrap();
        first.as_object_mut().unwrap().remove("response_model");
        let mut second = sample_record("request-2", "2026-07-17T20:31:00+08:00", "gpt-b");
        second.response_model = "gpt-b-upstream".to_string();
        fs::write(
            events_dir.join("2026-07-17-20.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": USAGE_SCHEMA_VERSION,
                "hour": "2026-07-17-20",
                "timezone": "+08:00",
                "records": [first]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            inbox_dir.join("pending.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": USAGE_SCHEMA_VERSION,
                "records": [second]
            }))
            .unwrap(),
        )
        .unwrap();

        initialize_usage_storage_at(&root).unwrap();
        initialize_usage_storage_at(&root).unwrap();
        let connection = open_usage_database_at(&root).unwrap();
        let count = connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let marker = connection
            .query_row(
                "SELECT value FROM usage_metadata WHERE key = ?1",
                params![LEGACY_JSON_MIGRATION_KEY],
                |row| row.get::<_, String>(0),
            )
            .unwrap();

        assert_eq!(count, 2);
        assert_eq!(marker, "2");
        let page = load_usage_events(
            &connection,
            &UsageQuery::default(),
            &GuiConfigFile::default(),
        )
        .unwrap();
        assert_eq!(page.items[0].response_model, "gpt-b-upstream");
        assert!(page.items[1].response_model.is_empty());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_queries_filter_aggregate_and_paginate() {
        let root = test_root("sqlite-query");
        let mut connection = open_test_database(&root);
        let success = sample_record("request-1", "2026-07-17T20:30:00+08:00", "gpt-a");
        let mut failed = sample_record("request-2", "2026-07-17T21:30:00+08:00", "gpt-5.6-terra");
        failed.failed = true;
        insert_usage_records(&mut connection, &[success, failed]).unwrap();
        let query = UsageQuery {
            model: Some("GPT-5.6-TERRA".to_string()),
            failed: Some(true),
            page_size: Some(20),
            ..UsageQuery::default()
        };

        let overview = load_usage_overview(&connection, &query).unwrap();
        let config = GuiConfigFile::default();
        let analysis = load_usage_analysis(&connection, &query, &config).unwrap();
        let events = load_usage_events(&connection, &query, &config).unwrap();

        assert_eq!(overview.total_requests, 1);
        assert_eq!(overview.failure_count, 1);
        assert_eq!(overview.tps, 0.0);
        assert_eq!(overview.tps_sample_count, 0);
        assert_eq!(overview.cache_hit_rate, 0.2);
        assert!((overview.estimated_cost - 0.0002564).abs() < 0.0000001);
        assert_eq!(overview.priced_requests, 1);
        assert_eq!(overview.timeline.len(), 1);
        assert_eq!(overview.timeline[0].models.len(), 1);
        assert_eq!(overview.timeline[0].models[0].key, "gpt-5.6-terra");
        assert!(overview.timeline[0].tokens > 0);
        assert_eq!(analysis.models[0].key, "gpt-5.6-terra");
        assert_eq!(events.total, 1);
        assert_eq!(events.items[0].id, "request-2");
        let exported = load_usage_events(
            &connection,
            &UsageQuery { page_size: Some(5_000), ..query },
            &config,
        ).unwrap();
        assert_eq!(exported.page_size, 5_000);
        assert_eq!(exported.items.len(), 1);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn timeline_groups_requests_into_half_hour_buckets() {
        let root = test_root("sqlite-half-hour");
        let mut connection = open_test_database(&root);
        let first = sample_record("half-1", "2026-07-17T20:10:00+08:00", "gpt-a");
        let second = sample_record("half-2", "2026-07-17T20:40:00+08:00", "gpt-a");
        insert_usage_records(&mut connection, &[first, second]).unwrap();

        let overview = load_usage_overview(&connection, &UsageQuery::default()).unwrap();
        assert_eq!(overview.timeline.len(), 2);
        assert_ne!(overview.timeline[0].hour, overview.timeline[1].hour);
        assert!(overview
            .timeline
            .iter()
            .all(|point| point.hour.ends_with("-00") || point.hour.ends_with("-30")));
        assert_eq!(overview.timeline.iter().map(|point| point.requests).sum::<u64>(), 2);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn overview_tps_averages_request_speeds_without_requiring_ttft() {
        let root = test_root("tps-overview");
        let mut connection = open_test_database(&root);

        let mut first = sample_record("tps-1", "2026-07-17T20:30:00+08:00", "gpt-a");
        first.latency_ms = 1_000;
        first.ttft_ms = Some(200);
        first.tokens.output_tokens = 80;

        let mut second = sample_record("tps-2", "2026-07-17T20:31:00+08:00", "gpt-a");
        second.latency_ms = 2_000;
        second.ttft_ms = Some(1_000);
        second.tokens.output_tokens = 20;

        let mut missing_ttft = sample_record("tps-3", "2026-07-17T20:32:00+08:00", "gpt-a");
        missing_ttft.latency_ms = 500;
        missing_ttft.ttft_ms = None;
        missing_ttft.tokens.output_tokens = 100;

        let mut equal_latency = sample_record("tps-4", "2026-07-17T20:33:00+08:00", "gpt-a");
        equal_latency.latency_ms = 500;
        equal_latency.ttft_ms = Some(500);
        equal_latency.tokens.output_tokens = 100;

        let mut failed = sample_record("tps-5", "2026-07-17T20:34:00+08:00", "gpt-a");
        failed.failed = true;
        failed.tokens.output_tokens = 100;

        let mut canceled = sample_record("tps-6", "2026-07-17T20:35:00+08:00", "gpt-a");
        canceled.canceled = true;
        canceled.tokens.output_tokens = 100;

        let mut non_generation = sample_record("tps-7", "2026-07-17T20:36:00+08:00", "gpt-a");
        non_generation.generate = false;
        non_generation.tokens.output_tokens = 100;

        let mut zero_latency = sample_record("tps-8", "2026-07-17T20:37:00+08:00", "gpt-a");
        zero_latency.latency_ms = 0;
        zero_latency.tokens.output_tokens = 100;

        let mut no_output = sample_record("tps-9", "2026-07-17T20:38:00+08:00", "gpt-a");
        no_output.tokens.output_tokens = 0;

        insert_usage_records(
            &mut connection,
            &[
                first,
                second,
                missing_ttft,
                equal_latency,
                failed,
                canceled,
                non_generation,
                zero_latency,
                no_output,
            ],
        )
        .unwrap();

        let overview = load_usage_overview(&connection, &UsageQuery::default()).unwrap();

        assert_eq!(overview.tps, (80.0 + 10.0 + 200.0 + 200.0) / 4.0);
        assert_eq!(overview.tps_sample_count, 4);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn response_model_accepts_manager_plus_field_names() {
        let record = normalize_usage_record(serde_json::json!({
            "request_id": "compatible-models", "requestedModel": "client-alias",
            "resolvedModel": "gpt-6-sol", "model": "display-model",
            "responseModel": " reported-model "
        }), &GuiConfigFile::default()).unwrap();
        assert_eq!(record.alias, "client-alias");
        assert_eq!(record.model, "gpt-6-sol");
        assert_eq!(record.response_model, "reported-model");
    }

    #[test]
    fn request_cost_matches_aggregate_and_recalculates_current_prices() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_usage_schema(&connection).unwrap();
        let mut records = Vec::new();
        for (index, model) in ["gpt-6-sol", "gpt-5.6-terra", "xai/grok-4.7"].iter().enumerate() {
            for input in [200_000, 250_000, 300_000] {
                let mut record = sample_record(&format!("{index}-{input}"), "2026-10-04T00:00:00Z", model);
                record.provider = if index < 2 { "codex" } else { "openai" }.into();
                record.service_tier = "fast".into();
                record.response_service_tier = "flex".into();
                record.tokens.input_tokens = input;
                record.tokens.output_tokens = 500;
                record.tokens.reasoning_tokens = 100;
                record.tokens.cache_read_tokens = 100_000;
                record.tokens.cache_creation_tokens = 1_000;
                record.tokens.total_tokens = input + 500;
                record.response_model = "unpriced-response-must-not-change-billing".into();
                records.push(record);
            }
        }
        records.push(sample_record("unknown-price", "2026-10-04T00:00:00Z", "unpriced-model"));
        insert_usage_records(&mut connection, &records).unwrap();
        let query = UsageQuery::default();
        let page = load_usage_events(&connection, &query, &GuiConfigFile::default()).unwrap();
        let (aggregate, priced) = load_estimated_cost(&connection, &build_usage_filter(&query)).unwrap();
        let total: f64 = page.items.iter().filter_map(|r| r.cost.as_ref().map(|c| c.total)).sum();
        assert!((aggregate - total).abs() < 1e-9);
        assert_eq!(priced, 9);
        assert!(page.items.iter().find(|r| r.id == "unknown-price").unwrap().cost.is_none());

        let free_price = ModelPrice { model: "unpriced-model".into(), prompt_configured: true,
            completion_configured: true, cache_read_configured: true, cache_creation_configured: true,
            source: "manual".into(), ..ModelPrice::default() };
        upsert_model_price(&connection, &free_price).unwrap();
        let updated = load_usage_events(&connection, &query, &GuiConfigFile::default()).unwrap();
        let free = updated.items.iter().find(|r| r.id == "unknown-price").unwrap();
        assert_eq!(free.cost.as_ref().unwrap().total, 0.0);
        let imported: UsageRecord = serde_json::from_value(serde_json::to_value(free).unwrap()).unwrap();
        assert!(imported.cost.is_none(), "Imported estimates must be recomputed");
    }

    #[test]
    fn estimates_gpt56_cost_with_cache_and_service_tier_rules() {
        let terra = official_model_price("openai/gpt-5.6-terra").unwrap();
        let standard_tokens = CostTokens {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 200_000,
            cache_creation: 100_000,
            ..CostTokens::default()
        };
        let standard = cost_for_price("openai/gpt-5.6-terra", "default", &standard_tokens, &terra);
        assert!((standard - 13.69).abs() < 0.000001);

        let long_tokens = CostTokens {
            input: 300_000,
            output: 200_000,
            cache_read: 100_000,
            long_input: 300_000,
            long_output: 200_000,
            long_cache_read: 100_000,
            ..CostTokens::default()
        };
        let long_priority = cost_for_price("gpt-5.6-terra", "priority", &long_tokens, &terra);
        assert!((long_priority - 4.44).abs() < 0.000001);
        assert!(official_model_price("unpriced-model").is_none());
    }

    #[test]
    fn estimates_gpt6_fast_mode_for_short_and_long_context() {
        let sol = official_model_price("gpt-6-sol").unwrap();
        let short_tokens = CostTokens {
            input: 1_000_000,
            output: 1_000_000,
            ..CostTokens::default()
        };
        assert!(
            (cost_for_price("gpt-6-sol", "fast", &short_tokens, &sol) - 24.0).abs() < 0.000001
        );

        let long_tokens = CostTokens {
            input: 300_000,
            output: 200_000,
            cache_read: 100_000,
            long_input: 300_000,
            long_output: 200_000,
            long_cache_read: 100_000,
            ..CostTokens::default()
        };
        assert!(
            (cost_for_price("gpt-6-sol", "fast", &long_tokens, &sol) - 7.68).abs() < 0.000001
        );
    }

    #[test]
    fn grok_47_uses_200k_threshold_and_doubles_long_context_output_price() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_usage_schema(&connection).unwrap();
        let mut short = sample_record("grok-short", "2026-09-25T00:00:00Z", "grok-4.7");
        short.tokens.input_tokens = 200_000;
        short.tokens.output_tokens = 100_000;
        short.tokens.cache_read_tokens = 0;
        short.tokens.total_tokens = 300_000;
        let mut long = sample_record("grok-long", "2026-09-25T00:00:00Z", "xai/grok-4.7");
        long.tokens.input_tokens = 250_000;
        long.tokens.output_tokens = 100_000;
        long.tokens.cache_read_tokens = 0;
        long.tokens.total_tokens = 350_000;
        insert_usage_records(&mut connection, &[short, long]).unwrap();

        let query = UsageQuery::default();
        let groups = load_usage_cost_groups(&connection, &build_usage_filter(&query)).unwrap();
        assert_eq!(groups.iter().find(|group| group.model == "grok-4.7").unwrap().tokens.long_input, 0);
        assert_eq!(groups.iter().find(|group| group.model == "xai/grok-4.7").unwrap().tokens.long_input, 250_000);
        let pricing = load_usage_pricing(&connection, &query).unwrap();
        assert!((pricing.total_cost - 3.2).abs() < 0.000001);
    }

    #[test]
    fn bundled_model_prices_are_available_offline_and_override_legacy_cloud_cache() {
        let root = test_root("bundled-pricing");
        let connection = open_test_database(&root);
        let bundled = bundled_model_prices().unwrap();
        assert!(bundled.len() >= 50);
        assert!(bundled.values().all(|price| price.source == "builtin"));
        assert_eq!(
            find_model_price(&bundled, "openai/gpt-5.6-terra-high")
                .unwrap()
                .prompt,
            2.0
        );

        upsert_model_price(
            &connection,
            &ModelPrice {
                model: "gpt-5.6-terra".to_string(),
                prompt: 999.0,
                completion: 999.0,
                prompt_configured: true,
                completion_configured: true,
                source: "litellm".to_string(),
                ..ModelPrice::default()
            },
        )
        .unwrap();
        let prices = load_model_prices(&connection).unwrap();
        assert_eq!(prices["gpt-5.6-terra"].prompt, 2.0);
        assert_eq!(prices["gpt-5.6-terra"].source, "builtin");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn models_dev_sync_uses_first_party_prices_and_preserves_explicit_zero_cache() {
        let catalog = serde_json::json!({
            "openai": { "models": {
                "gpt-6-sol": { "cost": { "input": 2.25, "output": 10.0, "cache_read": 0.0 } },
                "missing-output": { "cost": { "input": 1.0 } }
            } },
            "anthropic": { "models": {} },
            "google": { "models": {} },
            "deepseek": { "models": {} },
            "xai": { "models": {} },
            "openrouter": { "models": {
                "gpt-6-sol": { "cost": { "input": 99.0, "output": 99.0 } }
            } }
        });
        let prices = parse_models_dev_prices(&catalog.to_string(), 123).unwrap();
        assert_eq!(prices.len(), 2);
        let price = prices.iter().find(|price| price.source_model_id == "openai/gpt-6-sol").unwrap();
        assert_eq!(price.model, "gpt-6-sol");
        assert_eq!(price.prompt, 2.25);
        assert_eq!(price.cache_read, 0.0);
        assert!(price.cache_read_configured);
        assert_eq!(price.source, "models.dev");
        assert_eq!(price.source_model_id, "openai/gpt-6-sol");

        let effective = enriched_model_price("gpt-6-sol", price);
        assert_eq!(effective.cache_read, 0.0);
        assert_eq!(effective.cache_creation, 0.0);
    }

    #[test]
    fn price_preview_matches_official_models_without_guessing_dynamic_aliases() {
        let catalog = serde_json::json!({
            "openai": { "models": { "gpt-6-sol": { "cost": { "input": 2.0, "output": 10.0 } } } },
            "google": { "models": { "gpt-6-sol": { "cost": { "input": 90.0, "output": 90.0 } } } }
        });
        let remote = parse_models_dev_prices(&catalog.to_string(), 1).unwrap();
        let models = vec![
            ("gpt-6-sol".to_string(), String::new()),
            ("gpt-daybreak-blue-latest".to_string(), String::new()),
            ("my-alias".to_string(), "gpt-6-sol".to_string()),
        ];
        let preview = build_model_price_preview(&models, &remote, "Models.dev", MODEL_PRICE_SYNC_URL);
        assert_eq!(preview.matches.len(), 2);
        assert_eq!(preview.matches[0].prompt, 2.0);
        assert_eq!(preview.matches[0].source_model_id, "openai/gpt-6-sol");
        assert_eq!(preview.matches[1].model, "my-alias");
        assert_eq!(preview.unmatched, ["gpt-daybreak-blue-latest"]);
    }

    #[test]
    fn price_preview_uses_usage_alias_for_models_with_existing_prices() {
        let root = test_root("price-preview-existing-alias");
        let connection = open_test_database(&root);
        upsert_model_price(
            &connection,
            &ModelPrice {
                model: "my-alias".to_string(),
                prompt: 1.0,
                completion: 2.0,
                source: "manual".to_string(),
                ..ModelPrice::default()
            },
        )
        .unwrap();
        connection
            .execute(
                "INSERT INTO usage_events (event_key, timestamp, timestamp_ms, local_hour, model, alias, created_at) VALUES ('preview-alias', '2026-09-24T00:00:00Z', 1, '2026-09-24T00', 'my-alias', 'gpt-6-sol', '2026-09-24T00:00:00Z')",
                [],
            )
            .unwrap();

        let models = collect_price_preview_models(&connection, &UsageQuery::default()).unwrap();
        let remote = parse_models_dev_prices(
            &serde_json::json!({
                "openai": { "models": { "gpt-6-sol": { "cost": { "input": 2.0, "output": 10.0 } } } }
            })
            .to_string(),
            1,
        )
        .unwrap();
        let preview =
            build_model_price_preview(&models, &remote, "Models.dev", MODEL_PRICE_SYNC_URL);
        let matched = preview
            .matches
            .iter()
            .find(|price| price.model == "my-alias")
            .unwrap();
        assert_eq!(matched.source_model_id, "openai/gpt-6-sol");
        assert!(!preview.unmatched.contains(&"my-alias".to_string()));

        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn litellm_preview_reads_only_base_text_token_prices() {
        let catalog = serde_json::json!({
            "openai/test": {
                "mode": "chat", "input_cost_per_token": 0.000002,
                "output_cost_per_token": 0.000010,
                "cache_read_input_token_cost": 0.0,
                "input_cost_per_token_above_200k_tokens": 0.000004
            },
            "audio/test": {
                "mode": "chat", "supported_output_modalities": ["audio"],
                "input_cost_per_token": 0.000002, "output_cost_per_token": 0.000010
            }
        });
        let prices = parse_litellm_prices(&catalog.to_string(), 1).unwrap();
        assert_eq!(prices.len(), 1);
        assert_eq!(prices[0].prompt, 2.0);
        assert_eq!(prices[0].completion, 10.0);
        assert!(prices[0].cache_read_configured);
        assert_eq!(prices[0].cache_read, 0.0);
    }

    #[tokio::test]
    async fn price_catalog_retries_after_truncated_body_and_decodes_gzip() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/api.json", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            std::io::Write::write_all(&mut encoder, b"{}").unwrap();
            let compressed = encoder.finish().unwrap();
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 2048];
                let size = stream.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..size]).to_ascii_lowercase();
                assert!(request.contains("accept-encoding: gzip"));
                if attempt == 0 {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}").await.unwrap();
                } else {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Encoding: gzip\r\nConnection: close\r\n\r\n",
                        compressed.len()
                    );
                    stream.write_all(header.as_bytes()).await.unwrap();
                    stream.write_all(&compressed).await.unwrap();
                }
            }
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let content = tokio::time::timeout(
            Duration::from_secs(5),
            fetch_model_price_catalog(&client, "test", &url),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(content, "{}");
        server.await.unwrap();
    }

    #[test]
    fn manual_model_prices_drive_pricing_and_overview_cost() {
        let root = test_root("manual-pricing");
        let mut connection = open_test_database(&root);
        let record = sample_record("request-1", "2026-07-17T20:30:00+08:00", "custom-model");
        insert_usage_records(&mut connection, &[record]).unwrap();
        upsert_model_price(
            &connection,
            &ModelPrice {
                model: "custom-model".to_string(),
                prompt: 1.0,
                completion: 2.0,
                cache_read: 0.1,
                cache_read_configured: true,
                prompt_configured: true,
                completion_configured: true,
                source: "manual".to_string(),
                ..ModelPrice::default()
            },
        )
        .unwrap();

        let query = UsageQuery::default();
        let overview = load_usage_overview(&connection, &query).unwrap();
        let pricing = load_usage_pricing(&connection, &query).unwrap();

        assert!((overview.estimated_cost - 0.0000482).abs() < 0.0000001);
        assert_eq!(overview.priced_requests, 1);
        assert!((pricing.total_cost - overview.estimated_cost).abs() < f64::EPSILON);
        assert_eq!(pricing.rows[0].model, "custom-model");
        assert_eq!(
            pricing.saved_prices,
            bundled_model_prices().unwrap().len() + 1
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_cache_prices_stay_zero_even_with_builtin_and_legacy_prices() {
        let root = test_root("zero-cache-pricing");
        let connection = open_test_database(&root);
        upsert_model_price(
            &connection,
            &ModelPrice {
                model: "gpt-6-sol".to_string(),
                prompt: 2.0,
                completion: 10.0,
                prompt_configured: true,
                completion_configured: true,
                source: "manual".to_string(),
                ..ModelPrice::default()
            },
        )
        .unwrap();
        connection
            .execute("UPDATE model_prices SET cache_per_1m = 99 WHERE model = 'gpt-6-sol'", [])
            .unwrap();
        let price = load_model_prices(&connection).unwrap().remove("gpt-6-sol").unwrap();
        assert_eq!(price.cache_read, 0.0);
        assert_eq!(price.cache_creation, 0.0);
        let tokens = CostTokens {
            input: 200_000,
            cache_read: 100_000,
            cache_creation: 100_000,
            ..CostTokens::default()
        };
        assert_eq!(cost_for_price("gpt-6-sol", "", &tokens, &price), 0.0);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
