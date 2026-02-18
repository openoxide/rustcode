use super::*;

#[test]
fn backend_selection_policy_layers_and_validates() {
    let temp_root = make_temp_dir("backend-selection-policy");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let global = temp_root.join("global.toml");
    let project = cwd.join("rustcode.toml");

    write_config(
        &global,
        r"
[policy.backend_selection]
provider_agnostic_weight = 4
automation_skills_weight = 2
",
    );

    write_config(
        &project,
        &format!(
            r#"
[policy.backend_selection]
privacy_weight = 5
subscription_penalty = 3

[trust]
projects = ["{}"]
"#,
            cwd.display()
        ),
    );

    let mut sources = ConfigSources::new(cwd.clone());
    sources.read_process_env = false;
    sources.global_config_path = Some(global);
    sources.project_config_path = Some(project);
    sources.trust_project = true;

    let cfg = ConfigLoader::load(&sources).expect("config should load");
    assert_eq!(cfg.backend_selection.provider_agnostic_weight, 4);
    assert_eq!(cfg.backend_selection.automation_skills_weight, 2);
    assert_eq!(cfg.backend_selection.open_source_weight, 1);
    assert_eq!(cfg.backend_selection.lsp_support_weight, 1);
    assert_eq!(cfg.backend_selection.privacy_weight, 5);
    assert_eq!(cfg.backend_selection.subscription_penalty, 3);
}

#[test]
fn backend_selection_policy_rejects_negative_weight() {
    let temp_root = make_temp_dir("backend-selection-policy-negative");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");
    let global = temp_root.join("global.toml");

    write_config(
        &global,
        r"
[policy.backend_selection]
privacy_weight = -1
",
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.global_config_path = Some(global);

    let err = ConfigLoader::load(&sources).expect_err("must reject negative weight");
    match err {
        ConfigError::Validation(message) => {
            assert!(message.contains("privacy_weight"));
            assert!(message.contains("must not be negative"));
        }
        _ => panic!("expected validation error"),
    }
}
