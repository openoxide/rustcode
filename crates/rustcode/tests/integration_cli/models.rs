use super::*;

#[test]
fn models_command_reads_custom_models_index() {
    let models_path = make_temp_file_path("models-index");
    std::fs::write(
        &models_path,
        r#"{
  "alpha": { "name": "Alpha Provider", "models": { "m1": {}, "m2": {} } },
  "beta": { "name": "Beta Provider", "models": { "x1": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["models", "alpha"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode models");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("alpha/m1"), "stdout: {stdout}");
    assert!(stdout.contains("alpha/m2"), "stdout: {stdout}");
}

#[test]
fn models_summary_includes_diagnostics_columns() {
    let models_path = make_temp_file_path("models-summary");
    std::fs::write(
        &models_path,
        r#"{
  "openrouter": { "name": "OpenRouter", "models": { "m1": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["models"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode models");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("protocol="), "stdout: {stdout}");
    assert!(stdout.contains("api_key_source="), "stdout: {stdout}");
    assert!(stdout.contains("missing="), "stdout: {stdout}");
    assert!(stdout.contains("policy_score="), "stdout: {stdout}");
    assert!(stdout.contains("policy_selected="), "stdout: {stdout}");
    assert!(stdout.contains("policy_available="), "stdout: {stdout}");
}

#[test]
fn models_json_summary_is_parseable() {
    let models_path = make_temp_file_path("models-json-summary");
    std::fs::write(
        &models_path,
        r#"{
  "openrouter": { "name": "OpenRouter", "models": { "m1": {} } },
  "openai": { "name": "OpenAI", "models": { "gpt-5": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["--json", "models"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode models --json");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let parsed: Value = serde_json::from_str(stdout.trim()).expect("stdout must be valid json");
    assert_eq!(parsed["schema_version"].as_u64(), Some(1));
    assert_eq!(parsed["providers"].as_array().map(Vec::len), Some(2));
    assert!(parsed["providers"]
        .as_array()
        .expect("providers must be array")
        .iter()
        .all(|provider| provider.get("policy_score").is_some()));
}

#[test]
fn models_json_summary_falls_back_to_builtin_presets_when_models_index_missing() {
    let missing_path = make_temp_file_path("models-json-summary-missing");
    std::fs::write(&missing_path, "not-json").expect("must write broken models file");

    let output = Command::new(rustcode_bin())
        .args(["--json", "models"])
        .env("RUSTCODE_MODELS_PATH", &missing_path)
        .output()
        .expect("must run rustcode models --json");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let parsed: Value = serde_json::from_str(stdout.trim()).expect("stdout must be valid json");
    assert_eq!(parsed["schema_version"].as_u64(), Some(1));
    assert_eq!(
        parsed["warning"].as_str(),
        Some("models index unavailable; showing builtin presets only")
    );
    assert!(parsed["providers"].as_array().is_some());
}

#[test]
fn models_json_provider_detail_includes_models_array() {
    let models_path = make_temp_file_path("models-json-provider");
    std::fs::write(
        &models_path,
        r#"{
  "alpha": { "name": "Alpha Provider", "models": { "m1": {}, "m2": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["--json", "models", "alpha"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode models alpha --json");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let parsed: Value = serde_json::from_str(stdout.trim()).expect("stdout must be valid json");
    assert_eq!(parsed["schema_version"].as_u64(), Some(1));
    assert_eq!(parsed["provider"]["id"].as_str(), Some("alpha"));
    assert_eq!(
        parsed["provider"]["models"].as_array().map(Vec::len),
        Some(2)
    );
    assert!(parsed["provider"]["policy_score"].is_null());
    assert_eq!(parsed["provider"]["policy_selected"].as_bool(), Some(false));
    assert!(parsed["provider"]["models"]
        .as_array()
        .expect("models must be array")
        .iter()
        .any(|item| item.as_str() == Some("alpha/m1")));
}

#[test]
fn models_provider_detail_works_for_known_provider_missing_from_models_index() {
    let models_path = make_temp_file_path("models-missing-provider");
    std::fs::write(
        &models_path,
        r#"{
  "alpha": { "name": "Alpha Provider", "models": { "m1": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .arg("--json")
        .arg("models")
        .arg("github-copilot-enterprise")
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run models provider detail");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid json");
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["provider"]["id"], "github-copilot-enterprise");
    assert!(payload["provider"]["models"].is_array());
    assert!(payload["provider"]["warning"].is_string());
}

#[test]
fn models_provider_detail_succeeds_without_models_index() {
    let missing_path = make_temp_file_path("models-index-missing");
    std::fs::write(&missing_path, "not-json").expect("must write broken models file");

    let output = Command::new(rustcode_bin())
        .args(["--json", "models", "github-copilot-enterprise"])
        .env("RUSTCODE_MODELS_PATH", &missing_path)
        .output()
        .expect("must run models provider detail");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let payload: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid json");
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["provider"]["id"], "github-copilot-enterprise");
    assert!(payload["provider"]["models"].as_array().is_some());
    assert_eq!(
        payload["provider"]["warning"].as_str(),
        Some("models index unavailable; diagnostics only")
    );
}
