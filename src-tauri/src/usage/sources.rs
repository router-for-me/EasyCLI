//! Source identities and readable labels from current credential/provider metadata.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Source {
    label: String,
    provider: String,
}

pub(super) struct Snapshot {
    scope: String,
    oauth: Option<HashMap<String, Source>>,
    providers: Option<HashMap<String, Source>>,
}

fn field<'a>(value: &'a Value, names: &[&str]) -> &'a str {
    names
        .iter()
        .filter_map(|name| value.get(*name)?.as_str())
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or("")
}

fn credentials(value: &Value) -> Option<HashMap<String, Source>> {
    let files = value.get("files")?.as_array()?;
    let mut result = HashMap::new();
    for file in files {
        let id = field(file, &["auth_index", "auth-index"]);
        if id.is_empty() {
            continue;
        }
        let label = field(file, &["note", "name", "email"]);
        result.insert(
            id.to_string(),
            Source {
                label: label.to_string(),
                provider: field(file, &["provider", "type"]).to_string(),
            },
        );
    }
    Some(result)
}

fn provider_label(_kind: &str, group: &Value, key: &Value) -> String {
    let note = field(key, &["note"]);
    let note = if note.is_empty() {
        field(group, &["note"])
    } else {
        note
    };
    if !note.is_empty() {
        return note.to_string();
    }
    let address = field(group, &["base-url", "base_url"]);
    if let Ok(mut url) = reqwest::Url::parse(address) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        return url.to_string();
    }
    "—".to_string()
}

fn providers(value: &Value) -> Option<HashMap<String, Source>> {
    let groups_by_kind = value.as_object()?;
    let mut result = HashMap::new();
    for (kind, groups) in groups_by_kind {
        if groups.is_null() {
            continue;
        }
        let groups = groups.as_array()?;
        for group in groups {
            let empty = Vec::new();
            let keys = match group.get("keys") {
                None | Some(Value::Null) => &empty,
                Some(value) => value.as_array()?,
            };
            // v8 also supports keyless OpenAI-compatible providers.
            let entries: Vec<&Value> = if keys.is_empty() {
                vec![group]
            } else {
                keys.iter().collect()
            };
            for key in entries {
                let id = field(key, &["auth_index", "auth-index"]);
                if id.is_empty() {
                    continue;
                }
                result.insert(
                    id.to_string(),
                    Source {
                        label: provider_label(kind, group, key),
                        provider: kind.to_string(),
                    },
                );
            }
        }
    }
    Some(result)
}

async fn fetch_json(config: &GuiConfigFile, path: &str) -> Option<Value> {
    management_http_client()
        .ok()?
        .get(management_endpoint(config, path).ok()?)
        .header("Authorization", management_authorization(config).ok()?)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()
}

pub(super) async fn fetch(config: &GuiConfigFile) -> Snapshot {
    let (oauth, api_keys) = tokio::join!(
        fetch_json(config, "credentials"),
        fetch_json(config, "config/api-keys")
    );
    let mut provider_sources = api_keys.as_ref().and_then(providers);
    if let (Some(value), Some(sources)) = (
        api_keys.as_ref().and_then(Value::as_object),
        provider_sources.as_mut(),
    ) {
        for (kind, groups) in value {
            for group in groups.as_array().into_iter().flatten() {
                for key in group
                    .get("keys")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if !field(key, &["note"]).is_empty() || !field(group, &["note"]).is_empty() {
                        continue;
                    }
                    if let Some(note) = config
                        .api_access_remark_for_source(kind, field(key, &["api-key", "api_key"]))
                    {
                        if let Some(source) =
                            sources.get_mut(field(key, &["auth_index", "auth-index"]))
                        {
                            source.label = note.to_string();
                        }
                    }
                }
            }
        }
    }
    Snapshot {
        scope: hash_text(&management_endpoint(config, "").unwrap_or_default()),
        oauth: oauth.as_ref().and_then(credentials),
        providers: provider_sources,
    }
}

// Cache only sanitized display metadata. Failed fetches preserve the last successful
// snapshot; a successful empty list removes stale identities, as keeper does.
pub(super) fn resolve(
    connection: &Connection,
    snapshot: Snapshot,
) -> Result<HashMap<String, Source>, String> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS usage_source_snapshots (scope TEXT NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(scope, kind))")
        .map_err(|_| "Failed to initialize usage source metadata".to_string())?;
    let mut result = HashMap::new();
    for (kind, incoming) in [("oauth", snapshot.oauth), ("providers", snapshot.providers)] {
        let entries = if let Some(entries) = incoming {
            let payload = serde_json::to_string(&entries)
                .map_err(|_| "Failed to encode usage source metadata".to_string())?;
            connection.execute("INSERT INTO usage_source_snapshots(scope, kind, payload) VALUES (?1, ?2, ?3) ON CONFLICT(scope, kind) DO UPDATE SET payload=excluded.payload", params![snapshot.scope, kind, payload])
                .map_err(|_| "Failed to cache usage source metadata".to_string())?;
            entries
        } else {
            let payload: Option<String> = connection
                .query_row(
                    "SELECT payload FROM usage_source_snapshots WHERE scope=?1 AND kind=?2",
                    params![snapshot.scope, kind],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| "Failed to read usage source metadata".to_string())?;
            payload
                .and_then(|value| serde_json::from_str(&value).ok())
                .unwrap_or_default()
        };
        result.extend(entries);
    }
    Ok(result)
}

pub(super) fn apply_analysis(analysis: &mut UsageAnalysis, sources: &HashMap<String, Source>) {
    analysis.upstreams.retain_mut(|entry| {
        let Some(source) = entry
            .category
            .key
            .strip_prefix("auth:")
            .and_then(|id| sources.get(id))
        else {
            return false;
        };
        entry.category.label = source.label.clone();
        entry.provider = source.provider.clone();
        true
    });
    let mut groups: BTreeMap<String, Vec<UsageUpstream>> = BTreeMap::new();
    for entry in std::mem::take(&mut analysis.upstreams) {
        groups
            .entry(entry.category.label.clone())
            .or_default()
            .push(entry);
    }
    analysis.upstreams = groups
        .into_values()
        .map(|mut entries| {
            let mut keys = entries
                .iter()
                .map(|entry| entry.category.key.clone())
                .collect::<Vec<_>>();
            keys.sort();
            keys.dedup();
            let mut combined = entries.remove(0);
            for entry in entries {
                combined.category.requests += entry.category.requests;
                combined.category.failures += entry.category.failures;
                combined.category.tokens += entry.category.tokens;
                if combined.provider != entry.provider {
                    combined.provider.clear();
                }
            }
            if keys.len() > 1 {
                combined.category.key = format!(
                    "group:{}",
                    serde_json::to_string(&keys).expect("string keys serialize")
                );
            }
            combined
        })
        .collect();
}

pub(super) fn apply_events(
    page: &mut UsageEventPage,
    sources: &HashMap<String, Source>,
    config: &GuiConfigFile,
) {
    for record in &mut page.items {
        record.source_display = if let Some(source) = sources.get(record.auth_index.trim()) {
            source.label.clone()
        } else if record.auth_type == "oauth" {
            usage_source_display(config, &record.provider, &record.source)
        } else {
            record.provider.clone()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_follow_current_metadata_not_historical_labels() {
        let oauth = credentials(&serde_json::json!({"files":[
            {"auth_index":"current", "email":"account@example.com", "name":"account.json", "note":"not an alias"},
            {"auth_index":"current", "email":"account@example.com"}
        ]})).unwrap();
        let api = providers(&serde_json::json!({"codex":[{"name":"Codex", "prefix":"work", "base-url":"https://api.example.com/v1", "keys":[{"auth_index":"api", "api-key":"test-secret"}]}]})).unwrap();
        assert_eq!(oauth.len(), 1);
        assert_eq!(oauth["current"].label, "account@example.com");
        assert_eq!(api["api"].label, "https://api.example.com/v1");
        let db = Connection::open_in_memory().unwrap();
        let sources = resolve(
            &db,
            Snapshot {
                scope: "test".into(),
                oauth: Some(oauth),
                providers: Some(api),
            },
        )
        .unwrap();
        let cached = resolve(
            &db,
            Snapshot {
                scope: "test".into(),
                oauth: None,
                providers: None,
            },
        )
        .unwrap();
        assert_eq!(cached.len(), 2);
        let cleared = resolve(
            &db,
            Snapshot {
                scope: "test".into(),
                oauth: Some(HashMap::new()),
                providers: None,
            },
        )
        .unwrap();
        assert_eq!(cleared.len(), 1);
        let mut analysis = UsageAnalysis {
            models: vec![],
            providers: vec![],
            sources: vec![],
            api_keys: vec![],
            upstreams: ["current", "old-1", "old-2", "api"]
                .into_iter()
                .map(|id| UsageUpstream {
                    category: UsageCategory {
                        key: format!("auth:{id}"),
                        label: "same historical email".into(),
                        requests: 1,
                        failures: 0,
                        tokens: 1,
                    },
                    provider: "".into(),
                    is_default: false,
                })
                .collect(),
        };
        apply_analysis(&mut analysis, &sources);
        assert_eq!(analysis.upstreams.len(), 2);
        assert_eq!(analysis.upstreams[0].category.label, "account@example.com");
        assert_eq!(
            analysis.upstreams[1].category.label,
            "https://api.example.com/v1"
        );
    }
    #[tokio::test]
    async fn fetches_both_v8_metadata_endpoints_and_filters_deleted_identities() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = GuiConfigFile::default();
        config.host = "127.0.0.1".into();
        config.port = listener.local_addr().unwrap().port();
        config.management_secret_key = "test-management-key".into();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                }
                let body = if request.starts_with(b"GET /v8/management/credentials HTTP/1.1") {
                    r#"{"files":[{"auth_index":"active","email":"account@example.com","note":"Account note"},{"auth_index":"unused","email":"unused@example.com"}]}"#
                } else {
                    assert!(request.starts_with(b"GET /v8/management/config/api-keys HTTP/1.1"));
                    r#"{"codex":[{"name":"Codex","prefix":"Team","base-url":"https://api.example.com/v1","keys":[{"auth_index":"api","api-key":"sk-provider-secret"}]}]}"#
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let snapshot = fetch(&config).await;
        server.await.unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        initialize_usage_schema(&db).unwrap();
        let registry = resolve(&db, snapshot).unwrap();
        let mut records = Vec::new();
        for (id, auth, source, kind) in [
            ("one", "active", "outdated@example.com", "oauth"),
            ("two", "deleted", "account@example.com", "oauth"),
            ("three", "api", "sk-provider-secret", "apikey"),
            ("four", "old-api", "sk-old-secret", "apikey"),
        ] {
            let mut record =
                super::super::tests::sample_record(id, "2026-10-13T00:00:00Z", "model");
            record.auth_index = auth.into();
            record.source = source.into();
            record.auth_type = kind.into();
            record.provider = "codex".into();
            records.push(record);
        }
        insert_usage_records(&mut db, &records).unwrap();
        let query = UsageQuery::default();
        let mut analysis =
            load_usage_analysis_with_notes(&db, &query, &config, &HashMap::new()).unwrap();
        apply_analysis(&mut analysis, &registry);
        assert_eq!(analysis.upstreams.len(), 2); // deleted and unused identities are absent
        let mut page = load_usage_events_with_notes(&db, &query, &config, &HashMap::new()).unwrap();
        apply_events(&mut page, &registry, &config);
        assert_eq!(page.total, 4); // historical requests are still retained
        assert_eq!(
            page.items
                .iter()
                .find(|r| r.auth_index == "active")
                .unwrap()
                .source_display,
            "Account note"
        );
        assert_eq!(
            page.items
                .iter()
                .find(|r| r.auth_index == "api")
                .unwrap()
                .source_display,
            "https://api.example.com/v1"
        );
        assert_eq!(
            page.items
                .iter()
                .find(|r| r.auth_index == "old-api")
                .unwrap()
                .source_display,
            "codex"
        );
        for option in analysis.upstreams {
            let query = UsageQuery {
                upstream: Some(option.category.key),
                ..Default::default()
            };
            let mut filtered =
                load_usage_events_with_notes(&db, &query, &config, &HashMap::new()).unwrap();
            apply_events(&mut filtered, &registry, &config);
            assert_eq!(filtered.total, 1);
            assert_eq!(filtered.items[0].source_display, option.category.label);
        }
        let payload: String = db
            .query_row(
                "SELECT group_concat(payload) FROM usage_source_snapshots",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!payload.contains("sk-provider-secret"));
    }
    #[test]
    fn labels_follow_only_credential_and_api_priorities() {
        let sources = credentials(&serde_json::json!({"files":[
            {"auth_index":"a", "note":"Work", "name":"file.json", "email":"a@example.com"},
            {"auth_index":"b", "note":"  ", "name":"account.json", "email":"same@example.com", "provider":"codex"},
            {"auth_index":"c", "email":"same@example.com", "provider":"devin"},
            {"auth_index":"d", "email":"same@example.com", "provider":"codex"}
        ]})).unwrap();
        assert_eq!(sources["a"].label, "Work");
        assert_eq!(sources["b"].label, "account.json");
        assert_eq!(sources["c"].label, "same@example.com");
        assert_eq!(sources["d"].label, "same@example.com");
        let group = serde_json::json!({"name":"ignored", "prefix":"ignored", "note":"API note", "base-url":"https://api.example.com/v1"});
        assert_eq!(provider_label("codex", &group, &Value::Null), "API note");
        assert_eq!(
            provider_label("codex", &group, &serde_json::json!({"note":"Key note"})),
            "Key note"
        );
        assert_eq!(
            provider_label(
                "codex",
                &serde_json::json!({"name":"ignored", "prefix":"ignored", "base-url":"https://api.example.com/v1"}),
                &Value::Null
            ),
            "https://api.example.com/v1"
        );
        assert_eq!(
            provider_label(
                "codex",
                &serde_json::json!({"base-url":"https://user:password@example.com:8317/v1?key=secret"}),
                &Value::Null
            ),
            "https://example.com:8317/v1"
        );
    }
    #[test]
    fn identical_display_sources_merge_and_filter_every_member() {
        let mut db = Connection::open_in_memory().unwrap();
        initialize_usage_schema(&db).unwrap();
        let config = GuiConfigFile::default();
        let mut records = Vec::new();
        let mut registry = HashMap::new();
        for (id, label, provider) in [
            ("a", "http://example.com/v1", "codex"),
            ("b", "http://example.com/v1", "xai"),
            ("c", "Other", "codex"),
        ] {
            let mut record =
                super::super::tests::sample_record(id, "2026-10-13T00:00:00Z", "model");
            record.auth_index = id.into();
            record.provider = provider.into();
            records.push(record);
            registry.insert(
                id.into(),
                Source {
                    label: label.into(),
                    provider: provider.into(),
                },
            );
        }
        insert_usage_records(&mut db, &records).unwrap();
        let mut analysis =
            load_usage_analysis_with_notes(&db, &UsageQuery::default(), &config, &HashMap::new())
                .unwrap();
        apply_analysis(&mut analysis, &registry);
        assert_eq!(analysis.upstreams.len(), 2);
        let group = analysis
            .upstreams
            .iter()
            .find(|entry| entry.category.label == "http://example.com/v1")
            .unwrap();
        assert_eq!(group.category.requests, 2);
        let query = UsageQuery {
            upstream: Some(group.category.key.clone()),
            ..Default::default()
        };
        let page = load_usage_events_with_notes(&db, &query, &config, &HashMap::new()).unwrap();
        assert_eq!(page.total, 2);
        assert!(page
            .items
            .iter()
            .all(|item| item.auth_index == "a" || item.auth_index == "b"));
        assert_eq!(load_usage_overview(&db, &query).unwrap().total_requests, 2);
        let mut filtered_analysis =
            load_usage_analysis_with_notes(&db, &query, &config, &HashMap::new()).unwrap();
        apply_analysis(&mut filtered_analysis, &registry);
        assert_eq!(filtered_analysis.upstreams.len(), 1);
        assert_eq!(filtered_analysis.upstreams[0].category.requests, 2);
        let invalid = UsageQuery {
            upstream: Some("group:not-json".into()),
            ..Default::default()
        };
        assert_eq!(
            load_usage_events_with_notes(&db, &invalid, &config, &HashMap::new())
                .unwrap()
                .total,
            0
        );
    }
}
