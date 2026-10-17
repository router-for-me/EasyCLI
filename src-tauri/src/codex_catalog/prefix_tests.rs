use super::*;

fn public_runtime(ids: &[&str]) -> Vec<CodexRuntimeModel> {
    parse_public_models(&serde_json::json!({
        "data": ids.iter().map(|id| serde_json::json!({"id": id})).collect::<Vec<_>>()
    }))
    .unwrap()
}

fn generated(runtime: &[CodexRuntimeModel], state: &CatalogState) -> Value {
    let prepared =
        prepare_catalog_with_customizations(runtime, &state.sources, &state.customizations)
            .unwrap();
    serde_json::from_str(&prepared.json).unwrap()
}

fn model<'a>(root: &'a Value, slug: &str) -> &'a Value {
    root["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["slug"] == slug)
        .unwrap_or_else(|| panic!("Missing catalog model {slug}"))
}

fn embedded_state() -> CatalogState {
    CatalogState {
        sources: parse_sources(MODEL_CATALOG_JSON).unwrap(),
        json: MODEL_CATALOG_JSON.to_string(),
        customizations: Default::default(),
    }
}

fn assert_inherited_model(actual: &Value, bare: &Value, slug: &str) {
    let mut expected = bare.clone();
    expected["slug"] = serde_json::json!(slug);
    expected["display_name"] = serde_json::json!(slug);
    let actual = actual.as_object().unwrap();
    let expected = expected.as_object().unwrap();
    assert_eq!(
        actual.len(),
        expected.len(),
        "{slug}: complete template field set"
    );
    for (field, value) in expected {
        assert!(
            actual.get(field) == Some(value),
            "{slug}: inherited field {field} differs from the bare template"
        );
    }
}

fn sources_with_prefixed_template(slug: &str) -> CatalogSources {
    let mut catalog: Value = serde_json::from_str(MODEL_CATALOG_JSON).unwrap();
    let mut extra = catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["slug"] == "gpt-6-astra")
        .unwrap()
        .clone();
    extra["slug"] = serde_json::json!(slug);
    extra["base_instructions"] = serde_json::json!("Exact slash template prompt");
    extra["model_messages"]["instructions_template"] =
        serde_json::json!("Exact slash template prompt");
    extra["supports_search_tool"] = Value::Bool(false);
    catalog["models"].as_array_mut().unwrap().push(extra);
    parse_sources(&serde_json::to_string(&catalog).unwrap()).unwrap()
}

#[test]
fn prefixed_official_ids_preserve_complete_template_behavior() {
    let state = embedded_state();
    for (prefixed, bare) in [
        ("Fy/gpt-6.1-sol", "gpt-6.1-sol"),
        ("Plus-Yyf/gpt-6.1-sol", "gpt-6.1-sol"),
        ("Any-Test/gpt-6-astra", "gpt-6-astra"),
    ] {
        let bare_catalog = generated(&public_runtime(&[bare]), &state);
        let baseline = model(&bare_catalog, bare);
        assert_eq!(baseline["supports_search_tool"], true);
        assert!(!baseline["base_instructions"].as_str().unwrap().is_empty());
        let runtime = public_runtime(&[prefixed]);
        let prepared = prepare_catalog_with_sources(&runtime, &state.sources).unwrap();
        let catalog: Value = serde_json::from_str(&prepared.json).unwrap();
        assert_inherited_model(model(&catalog, prefixed), baseline, prefixed);
        assert_eq!(prepared.models[0].name, prefixed);
        assert!(prepared.models[0].alias.is_none());
    }
}

#[test]
fn prefixed_lookup_is_case_insensitive_and_preserves_runtime_context() {
    let state = embedded_state();
    let runtime = parse_public_models(&serde_json::json!({"data": [{
        "id": "  fY/GPt-6.1-SoL \n",
        "context_window": 64_000,
        "max_context_window": 128_000
    }]}))
    .unwrap();
    let bare_runtime = parse_public_models(&serde_json::json!({"data": [{
        "id": "gpt-6.1-sol",
        "context_window": 64_000,
        "max_context_window": 128_000
    }]}))
    .unwrap();
    let catalog = generated(&runtime, &state);
    let baseline = generated(&bare_runtime, &state);
    let slug = "fY/GPt-6.1-SoL";
    assert_inherited_model(model(&catalog, slug), model(&baseline, "gpt-6.1-sol"), slug);
    assert_eq!(model(&catalog, slug)["context_window"], 64_000);
    assert_eq!(model(&catalog, slug)["max_context_window"], 128_000);
    let snapshot =
        serde_json::to_value(customizations::snapshot_for_state(&runtime, &state).unwrap())
            .unwrap();
    assert_eq!(model(&snapshot, slug)["hasOfficialTemplate"], true);
    assert_eq!(model(&snapshot, slug)["contextSource"], "client");
    assert!(state.sources.template_for(" \nfY/GPt-6.1-SoL \t").is_some());
}

#[test]
fn exact_prefixed_template_takes_priority_over_bare_inheritance() {
    let mut state = embedded_state();
    state.sources = sources_with_prefixed_template("Fy/gpt-6.1-sol");
    let slug = "fY/GPt-6.1-SoL";
    let runtime = public_runtime(&[slug]);
    let catalog = generated(&runtime, &state);
    let exact = model(&catalog, slug);
    assert_eq!(exact["base_instructions"], "Exact slash template prompt");
    assert_eq!(
        exact["model_messages"]["instructions_template"],
        "Exact slash template prompt"
    );
    assert_eq!(exact["supports_search_tool"], false);
    assert_eq!(exact["context_window"], 272_000);
    assert_eq!(exact["slug"], slug);
    assert_eq!(exact["display_name"], slug);
}

#[test]
fn prefix_lookup_rejects_empty_unknown_and_misleading_suffixes() {
    let state = embedded_state();
    let ids = [
        "/gpt-6.1-sol",
        "Fy/",
        "Fy/unknown-model",
        "Fy/ gpt-6.1-sol",
        "Fy/other/gpt-6.1-sol",
        "Fy//gpt-6.1-sol",
        "Fy/gpt-6.1-sol/extra",
        "Fy/gpt-6.1-sol-extended",
        "almost-gpt-6.1-sol",
    ];
    let runtime = public_runtime(&ids);
    let catalog = generated(&runtime, &state);
    let snapshot =
        serde_json::to_value(customizations::snapshot_for_state(&runtime, &state).unwrap())
            .unwrap();
    for slug in ids {
        assert_eq!(model(&catalog, slug)["slug"], slug);
        assert_eq!(
            model(&catalog, slug)["supports_search_tool"],
            false,
            "{slug}"
        );
        assert_eq!(
            model(&catalog, slug)["web_search_tool_type"],
            "text",
            "{slug}"
        );
        assert_eq!(
            model(&snapshot, slug)["hasOfficialTemplate"],
            false,
            "{slug}"
        );
    }
    assert!(state.sources.template_for("").is_none());
    assert!(state.sources.template_for(" \t").is_none());
}

#[test]
fn removing_one_prefix_preserves_the_complete_remaining_slash_identity() {
    let mut state = embedded_state();
    state.sources = sources_with_prefixed_template("vendor/gpt-6-astra");
    let runtime = public_runtime(&[
        "Channel/vendor/gpt-6-astra",
        "Channel/other/vendor/gpt-6-astra",
    ]);
    let catalog = generated(&runtime, &state);
    let inherited = model(&catalog, "Channel/vendor/gpt-6-astra");
    assert_eq!(
        inherited["base_instructions"],
        "Exact slash template prompt"
    );
    assert_ne!(
        model(&catalog, "Channel/other/vendor/gpt-6-astra")["base_instructions"],
        "Exact slash template prompt"
    );
    let snapshot =
        serde_json::to_value(customizations::snapshot_for_state(&runtime, &state).unwrap())
            .unwrap();
    assert_eq!(
        model(&snapshot, "Channel/vendor/gpt-6-astra")["hasOfficialTemplate"],
        true
    );
    assert_eq!(
        model(&snapshot, "Channel/other/vendor/gpt-6-astra")["hasOfficialTemplate"],
        false
    );
}

#[test]
fn prefixed_customizations_are_isolated_across_refresh_and_local_state_restart() {
    let mut state = embedded_state();
    let ids = ["gpt-6.1-sol", "Fy/gpt-6.1-sol", "Plus-Yyf/gpt-6.1-sol"];
    let runtime = public_runtime(&ids);
    for (slug, prompt, context) in [
        (ids[0], "Bare user prompt", 64_000),
        (ids[1], "Fy user prompt", 128_000),
        (ids[2], "Plus user prompt", 256_000),
    ] {
        state.customizations.insert(
            normalize_id(slug),
            serde_json::json!({
                "base_instructions": prompt,
                "context_window": context,
                "max_context_window": context
            })
            .as_object()
            .unwrap()
            .clone(),
        );
    }
    let initial = generated(&runtime, &state);
    assert_eq!(
        model(&initial, ids[0])["base_instructions"],
        "Bare user prompt"
    );
    assert_eq!(
        model(&initial, ids[1])["base_instructions"],
        "Fy user prompt"
    );
    assert_eq!(
        model(&initial, ids[2])["base_instructions"],
        "Plus user prompt"
    );

    let mut refreshed: Value = serde_json::from_str(MODEL_CATALOG_JSON).unwrap();
    refreshed["catalog_revision"] = serde_json::json!(state.sources.revision + 1);
    let template = refreshed["models"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|model| model["slug"] == "gpt-6.1-sol")
        .unwrap();
    template["context_window"] = serde_json::json!(600_000);
    template["description"] = serde_json::json!("Refreshed official description");
    let refreshed = serde_json::to_string(&refreshed).unwrap();
    assert!(state.activate(&refreshed, parse_sources(&refreshed).unwrap()));

    let saved = serde_json::to_vec(&state.customizations).unwrap();
    let restarted = CatalogState {
        sources: parse_sources(&refreshed).unwrap(),
        json: refreshed,
        customizations: serde_json::from_slice(&saved).unwrap(),
    };
    assert_eq!(restarted.customizations.len(), 3);
    for current in [&state, &restarted] {
        let catalog = generated(&runtime, current);
        let snapshot =
            serde_json::to_value(customizations::snapshot_for_state(&runtime, current).unwrap())
                .unwrap();
        for (slug, prompt, context) in [
            (ids[0], "Bare user prompt", 64_000),
            (ids[1], "Fy user prompt", 128_000),
            (ids[2], "Plus user prompt", 256_000),
        ] {
            let generated = model(&catalog, slug);
            let editor = model(&snapshot, slug);
            assert_eq!(generated["base_instructions"], prompt);
            assert_eq!(generated["model_messages"]["instructions_template"], prompt);
            assert_eq!(generated["context_window"], context);
            assert_eq!(generated["description"], "Refreshed official description");
            assert_eq!(generated["supports_search_tool"], true);
            assert_eq!(editor["configuration"]["base_instructions"], prompt);
            assert_eq!(editor["defaults"]["context_window"], 600_000);
            assert_eq!(editor["customized"], true);
            assert_eq!(editor["hasOfficialTemplate"], true);
        }
    }
    let before =
        serde_json::to_value(customizations::snapshot_for_state(&runtime, &restarted).unwrap())
            .unwrap();
    let mut changed = restarted.clone();
    changed
        .customizations
        .get_mut("fy/gpt-6.1-sol")
        .unwrap()
        .insert(
            "base_instructions".into(),
            serde_json::json!("Updated Fy prompt"),
        );
    let after =
        serde_json::to_value(customizations::snapshot_for_state(&runtime, &changed).unwrap())
            .unwrap();
    assert_ne!(before["revision"], after["revision"]);
    assert_eq!(
        model(&after, ids[2])["configuration"],
        model(&before, ids[2])["configuration"]
    );
}

#[test]
fn prefixed_editor_defaults_and_official_flags_match_generated_catalog() {
    let state = embedded_state();
    let runtime = public_runtime(&[
        "Fy/gpt-6.1-sol",
        "Plus-Yyf/gpt-6.1-sol",
        "Any-Test/gpt-6-astra",
        "Fy/unknown-model",
    ]);
    let catalog = generated(&runtime, &state);
    let snapshot =
        serde_json::to_value(customizations::snapshot_for_state(&runtime, &state).unwrap())
            .unwrap();
    for current in &runtime {
        let generated = model(&catalog, &current.slug);
        let editor = model(&snapshot, &current.slug);
        assert_eq!(
            editor["hasOfficialTemplate"],
            current.slug != "Fy/unknown-model"
        );
        assert_eq!(editor["customized"], false);
        assert_eq!(editor["contextSource"], "template");
        for field in customizations::EDITABLE_FIELDS {
            let expected = generated.get(field).cloned().unwrap_or_else(|| {
                if field == "effective_context_window_percent" {
                    serde_json::json!(95)
                } else {
                    Value::Null
                }
            });
            assert_eq!(
                editor["defaults"][field], expected,
                "{}: {field}",
                current.slug
            );
            assert_eq!(
                editor["configuration"][field], expected,
                "{}: {field}",
                current.slug
            );
        }
    }
}
