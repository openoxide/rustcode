use super::*;

#[test]
fn defaults_validate() {
    let temp_root = make_temp_dir("defaults");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;

    let cfg = ConfigLoader::load(&sources).expect("config should load");
    assert!(!cfg.profile.is_empty());
    assert!(!cfg.model.is_empty());
}

#[test]
fn applies_layered_precedence_and_merges_env_and_plugins() {
    let temp_root = make_temp_dir("precedence");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let global = temp_root.join("global.toml");
    let user = temp_root.join("user.toml");
    let project = cwd.join("rustcode.toml");

    write_config(
        &global,
        r#"
profile = "global"
model = "global-model"
allow_network = false
plugins = ["g1"]

[env]
A = "global-a"
B = "global-b"
"#,
    );

    write_config(
        &user,
        &format!(
            r#"
profile = "user"
plugins = ["u1", "g1"]

[env]
B = "user-b"
C = "user-c"

[trust]
projects = ["{}"]
"#,
            cwd.display()
        ),
    );

    write_config(
        &project,
        r#"
model = "project-model"
allow_network = true
plugins = ["p1"]

[env]
A = "project-a"
"#,
    );

    let mut sources = ConfigSources::new(cwd.clone());
    sources.read_process_env = false;
    sources.global_config_path = Some(global);
    sources.user_config_path = Some(user);
    sources.project_config_path = Some(project.clone());

    let cfg = ConfigLoader::load(&sources).expect("config should load");

    assert_eq!(cfg.profile, "user");
    assert_eq!(cfg.model, "project-model");
    assert!(cfg.allow_network);
    assert_eq!(cfg.plugins, vec!["g1", "u1", "p1"]);
    assert_eq!(cfg.env.get("A").expect("A must exist"), "project-a");
    assert_eq!(cfg.env.get("B").expect("B must exist"), "user-b");
    assert_eq!(cfg.env.get("C").expect("C must exist"), "user-c");
    assert_eq!(cfg.project_config_path, Some(project));
    assert!(cfg.project_config_trusted);
}

#[test]
fn provider_filters_layer_and_enforce_allow_list() {
    let temp_root = make_temp_dir("provider-filters");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let global = temp_root.join("global.toml");
    let user = temp_root.join("user.toml");

    write_config(
        &global,
        r#"
enabled_providers = ["openrouter", "openai"]
"#,
    );

    write_config(
        &user,
        r#"
disabled_providers = ["openai"]
"#,
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.global_config_path = Some(global);
    sources.user_config_path = Some(user);

    let cfg = ConfigLoader::load(&sources).expect("config should load");
    assert!(cfg.provider_allowed("openrouter"));
    assert!(!cfg.provider_allowed("openai"));
    assert!(
        !cfg.provider_allowed("anthropic"),
        "enabled_providers allow-list should exclude non-listed providers"
    );
}

#[test]
fn process_env_can_override_allow_network() {
    let _guard = ENV_LOCK.lock().expect("env lock");

    let temp_root = make_temp_dir("env-allow-network");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let global = temp_root.join("global.toml");
    let user = temp_root.join("user.toml");

    std::env::set_var("RUSTCODE_GLOBAL_CONFIG", global.as_os_str());
    std::env::set_var("RUSTCODE_USER_CONFIG", user.as_os_str());
    std::env::set_var("RUSTCODE_ALLOW_NETWORK", "1");
    std::env::remove_var("RUSTCODE_PROFILE");
    std::env::remove_var("RUSTCODE_MODEL");
    std::env::remove_var("RUSTCODE_LLM_PROVIDER");
    std::env::remove_var("RUSTCODE_LLM_BASE_URL");
    std::env::remove_var("RUSTCODE_LLM_API_KEY_ENV");

    let sources = ConfigSources::new(cwd);
    let cfg = ConfigLoader::load(&sources).expect("config should load");
    assert!(cfg.allow_network);

    std::env::remove_var("RUSTCODE_ALLOW_NETWORK");
    std::env::remove_var("RUSTCODE_GLOBAL_CONFIG");
    std::env::remove_var("RUSTCODE_USER_CONFIG");
}

#[test]
fn provider_filter_rejects_invalid_id() {
    let temp_root = make_temp_dir("provider-filter-invalid");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let user = temp_root.join("user.toml");
    write_config(
        &user,
        r#"
enabled_providers = ["OpenAI"]
"#,
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.user_config_path = Some(user);

    let err = ConfigLoader::load(&sources).expect_err("invalid provider id should fail");
    assert!(err.to_string().contains("[a-z0-9-]+"), "err={err}");
}

#[test]
fn rejects_untrusted_project_config() {
    let temp_root = make_temp_dir("untrusted");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let user = temp_root.join("user.toml");
    let project = cwd.join("rustcode.toml");

    write_config(&user, "profile = \"user\"\n");
    write_config(&project, "model = \"project\"\n");

    let mut sources = ConfigSources::new(cwd.clone());
    sources.read_process_env = false;
    sources.user_config_path = Some(user);
    sources.project_config_path = Some(project);

    let err = ConfigLoader::load(&sources).expect_err("must reject untrusted config");
    match err {
        ConfigError::Trust(message) => {
            assert!(message.contains("not trusted"));
            assert!(message.contains(&cwd.display().to_string()));
        }
        _ => panic!("expected trust error"),
    }
}

#[test]
fn cli_overrides_are_highest_precedence() {
    let temp_root = make_temp_dir("cli-overrides");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let user = temp_root.join("user.toml");
    let project = cwd.join("rustcode.toml");

    write_config(
        &user,
        &format!(
            r#"
profile = "user"
model = "user-model"

[trust]
projects = ["{}"]
"#,
            cwd.display()
        ),
    );
    write_config(
        &project,
        "profile = \"project\"\nmodel = \"project-model\"\n",
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.user_config_path = Some(user);
    sources.project_config_path = Some(project);
    sources.profile_override = Some("cli-profile".to_string());
    sources.model_override = Some("cli-model".to_string());

    let cfg = ConfigLoader::load(&sources).expect("config should load");
    assert_eq!(cfg.profile, "cli-profile");
    assert_eq!(cfg.model, "cli-model");
}

#[test]
fn llm_config_layers_and_cli_override_apply() {
    let temp_root = make_temp_dir("llm-config");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let user = temp_root.join("user.toml");
    let project = cwd.join("rustcode.toml");

    write_config(
        &user,
        &format!(
            r#"
[llm]
provider = "openrouter"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"

[trust]
projects = ["{}"]
"#,
            cwd.display()
        ),
    );
    write_config(
        &project,
        r#"
[llm]
provider = "anthropic"
baseURL = "https://api.anthropic.com"
apiKeyEnv = "ANTHROPIC_API_KEY"
"#,
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.user_config_path = Some(user);
    sources.project_config_path = Some(project);
    sources.llm_provider_override = Some("ollama".to_string());

    let cfg = ConfigLoader::load(&sources).expect("config should load");
    assert_eq!(cfg.llm_provider, "ollama");
    assert_eq!(
        cfg.llm_base_url.as_deref(),
        Some("https://api.anthropic.com")
    );
    assert_eq!(cfg.llm_api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
}
