use super::*;

#[test]
fn permissions_rules_load_in_layer_order_and_validate_globs() {
    let temp_root = make_temp_dir("permissions");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let global = temp_root.join("global.toml");
    let user = temp_root.join("user.toml");
    let project = cwd.join("rustcode.toml");

    write_config(
        &global,
        r#"
[[permissions]]
permission = "write"
action = "allow"
pattern = "docs/**"
"#,
    );
    write_config(
        &user,
        &format!(
            r#"
[[permissions]]
permission = "exec"
action = "deny"
pattern = "rm"

[trust]
projects = ["{}"]
"#,
            cwd.display()
        ),
    );
    write_config(
        &project,
        r#"
[[permissions]]
permission = "write"
action = "ask"
pattern = "src/**"
"#,
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.global_config_path = Some(global);
    sources.user_config_path = Some(user);
    sources.project_config_path = Some(project);

    let cfg = ConfigLoader::load(&sources).expect("config should load");
    assert_eq!(cfg.permission_rules.len(), 3);
    assert_eq!(cfg.permission_rules[0].permission, "write");
    assert_eq!(cfg.permission_rules[0].pattern, "docs/**");
    assert_eq!(cfg.permission_rules[1].permission, "exec");
    assert_eq!(cfg.permission_rules[1].pattern, "rm");
    assert_eq!(cfg.permission_rules[2].permission, "write");
    assert_eq!(cfg.permission_rules[2].pattern, "src/**");
}

#[test]
fn permissions_rules_reject_invalid_glob_pattern() {
    let temp_root = make_temp_dir("permissions-invalid");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let user = temp_root.join("user.toml");
    write_config(
        &user,
        r#"
[[permissions]]
permission = "write"
action = "allow"
pattern = "["
"#,
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.user_config_path = Some(user);

    let err = ConfigLoader::load(&sources).expect_err("must reject invalid glob");
    assert!(
        err.to_string().contains("permissions[0].pattern"),
        "err={err}"
    );
}
