use super::*;

#[test]
fn core_failure_messages_expose_outcomes_without_secret_source_text() {
    for detail in [
        "YAML source: key: secret-token",
        "Original configuration restored secret-token",
        "Automatic restoration failed secret-token",
        "Configuration changed secret-token",
    ] {
        let rendered = agent_core_error(detail.into());
        assert!(!rendered.contains("secret-token"));
        if detail.contains("Automatic restoration failed") {
            assert!(rendered.contains("rollback also failed"));
        }
    }
}

#[test]
fn core_failure_messages_distinguish_alias_and_configuration_failures() {
    for (detail, expected) in [
        (
            "Already used by another model: claude-sonnet-5 secret-token",
            "Choose a different alias",
        ),
        (
            "Unable to determine model private-model CPA configuration source; cannot create Claude Desktop alias secret-token",
            "valid access source",
        ),
        (
            "Updated kernel configuration does not match the expected value (path: secret-token); write rejected",
            "format compatibility check failed",
        ),
        (
            "Failed to validate updated kernel configuration: secret-token",
            "original configuration was not written",
        ),
        (
            "Failed to parse kernel YAML configuration: secret-token",
            "Invalid kernel YAML configuration format",
        ),
        ("Management API error (401): secret-token", "authentication failed"),
        ("Management API error (403): secret-token", "authentication failed"),
    ] {
        let rendered = agent_core_error(detail.into());
        assert!(rendered.contains(expected), "{rendered}");
        assert!(!rendered.contains("secret-token"));
        assert!(!rendered.contains("private-model"));
    }
}

#[tokio::test]
async fn template_confirmation_is_bound_to_files_and_generated_content() {
    let home = crate::unique_test_temp_path("cpa-template");
    fs::create_dir_all(&home).unwrap();
    let paths = config_paths("pi", &home).unwrap();
    let before = config_images(&paths).unwrap();
    let updates = build_pi_template_updates(&home, 8317, "secret", "model").unwrap();
    let after = prepare_config_updates("pi", &paths, &before, &updates, true).unwrap();
    let plan = || TemplatePlan {
        client: "pi".into(),
        paths: paths.clone(),
        before: before.clone(),
        after: after.clone(),
        mappings: None,
        mapping_revision: String::new(),
        model: "model".into(),
        core: None,
        preview: TemplatePreview {
            revision: "reviewed".into(),
            files: Vec::new(),
        },
    };
    assert!(
        execute_template_plan(&GuiConfigFile::default(), plan(), "outdated")
            .await
            .is_err()
    );
    assert_eq!(before, config_images(&paths).unwrap());
    fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
    fs::write(&paths[0], "external edit").unwrap();
    assert!(
        execute_template_plan(&GuiConfigFile::default(), plan(), "reviewed")
            .await
            .is_err()
    );
    assert_eq!(fs::read_to_string(&paths[0]).unwrap(), "external edit");
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn incomplete_or_invalid_template_never_writes_any_file() {
    let home = crate::unique_test_temp_path("cpa-template-invalid");
    let paths = config_paths("pi", &home).unwrap();
    let before = config_images(&paths).unwrap();
    let mut updates = build_pi_template_updates(&home, 8317, "secret", "model").unwrap();
    assert!(prepare_config_updates("pi", &paths, &before, &updates[..1], true).is_err());
    updates[1].after = "invalid".into();
    assert!(prepare_config_updates("pi", &paths, &before, &updates, true).is_err());
    assert_eq!(before, config_images(&paths).unwrap());
}
