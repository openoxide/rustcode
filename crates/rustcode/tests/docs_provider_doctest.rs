use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn rustcode_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustcode"))
}

fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // rustcode/crates/rustcode -> rustcode/
    manifest_dir
        .parent()
        .and_then(|dir| dir.parent())
        .expect("crate must be in rustcode/crates/rustcode")
        .to_path_buf()
}

fn make_temp_dir(stem: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rustcode-{stem}-{nanos}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn extract_doctest_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut in_block = false;
    let mut buf = String::new();

    for line in markdown.lines() {
        if !in_block {
            if let Some(info) = line.strip_prefix("```") {
                let info = info.trim();
                if info.contains("rustcode-doctest") {
                    in_block = true;
                    buf.clear();
                }
            }
            continue;
        }

        if line.trim() == "```" {
            in_block = false;
            blocks.push(buf.clone());
            buf.clear();
            continue;
        }

        buf.push_str(line);
        buf.push('\n');
    }

    blocks
}

fn parse_env_and_args(line: &str) -> Option<(BTreeMap<String, String>, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let line = line.strip_prefix("$ ").unwrap_or(line);

    let mut envs = BTreeMap::new();
    let mut args: Vec<String> = Vec::new();
    for token in line.split_whitespace() {
        if args.is_empty() {
            if let Some((k, v)) = token.split_once('=') {
                let key_ok = !k.is_empty()
                    && k.chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
                if key_ok {
                    envs.insert(k.to_string(), v.to_string());
                    continue;
                }
            }
        }
        args.push(token.to_string());
    }

    if args.is_empty() {
        return None;
    }
    Some((envs, args))
}

fn json_expected_for_command(args: &[String]) -> bool {
    // Global flag is `--json`; only treat commands with structured JSON responses as parseable.
    if !args.iter().any(|arg| arg == "--json") {
        return false;
    }

    // args[0] is the literal "rustcode" token from the docs.
    let mut it = args.iter().skip(1);
    // Skip global flags.
    let command = loop {
        let Some(arg) = it.next() else {
            return false;
        };
        if arg.starts_with('-') {
            continue;
        }
        break arg.as_str();
    };

    match command {
        "models" | "auth" | "mcp" => true,
        _ => false,
    }
}

fn validate_json_shape(args: &[String], stdout: &str) {
    let value: Value =
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("invalid json: {e}"));
    assert_eq!(value["schema_version"].as_u64(), Some(1));

    let command = args
        .iter()
        .skip(1)
        .find(|arg| !arg.starts_with('-'))
        .map(|s| s.as_str())
        .unwrap_or("");

    if command == "models" {
        // summary: {schema_version, providers: [...]}
        // detail:  {schema_version, provider: {...}}
        let models_pos = args.iter().position(|arg| arg == "models");
        let provider_id = models_pos
            .and_then(|pos| args.get(pos + 1))
            .filter(|arg| !arg.starts_with('-'))
            .map(|s| s.as_str());

        if let Some(provider_id) = provider_id {
            assert!(
                value.get("provider").is_some(),
                "expected provider detail payload"
            );
            assert_eq!(
                value["provider"]["id"].as_str(),
                Some(provider_id),
                "expected provider.id to match requested provider"
            );
        } else {
            assert!(
                value.get("providers").is_some(),
                "expected providers summary payload"
            );
            assert!(
                value["providers"].as_array().is_some(),
                "expected providers to be an array"
            );
        }
    }
}

#[test]
fn provider_docs_doctests() {
    let root = repo_root();
    let docs_path = root.join("docs/PROVIDER_DOCTESTS.md");
    let markdown = std::fs::read_to_string(&docs_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", docs_path.display()));

    let blocks = extract_doctest_blocks(&markdown);
    assert!(
        !blocks.is_empty(),
        "expected at least one rustcode-doctest block in {}",
        docs_path.display()
    );

    let temp_home = make_temp_dir("docs-provider-doctest");
    let auth_file = temp_home.join("auth.json");
    let cache_home = temp_home.join("cache");
    std::fs::create_dir_all(&cache_home).expect("create cache dir");

    for (block_i, block) in blocks.iter().enumerate() {
        for (line_i, line) in block.lines().enumerate() {
            let Some((envs, args)) = parse_env_and_args(line) else {
                continue;
            };

            assert_eq!(
                args.get(0).map(|s| s.as_str()),
                Some("rustcode"),
                "only rustcode commands are allowed in doctest blocks (block={}, line={}, got={:?})",
                block_i + 1,
                line_i + 1,
                args
            );

            let mut cmd = Command::new(rustcode_bin());
            cmd.current_dir(&root);
            cmd.args(args.iter().skip(1));

            // Force offline + isolated environment. Docs blocks must stay non-networked.
            cmd.env("RUSTCODE_ALLOW_NETWORK", "0");
            cmd.env("RUSTCODE_AUTH_FILE", &auth_file);
            cmd.env("HOME", &temp_home);
            cmd.env("XDG_CACHE_HOME", &cache_home);

            for (k, v) in envs {
                cmd.env(k, v);
            }

            let output = cmd.output().unwrap_or_else(|e| {
                panic!(
                    "run doctest command (block={}, line={}): {e}",
                    block_i + 1,
                    line_i + 1
                )
            });

            assert!(
                output.status.success(),
                "doctest command failed (block={}, line={})\ncmd={:?}\nstdout={}\nstderr={}",
                block_i + 1,
                line_i + 1,
                cmd,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );

            if json_expected_for_command(&args) {
                let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
                validate_json_shape(&args, &stdout);
            }
        }
    }
}

#[test]
fn provider_doctests_fixture_paths_exist() {
    let root = repo_root();
    let fixture = root.join("docs/fixtures/models_min.json");
    assert!(
        fixture.is_file(),
        "missing fixture file: {}",
        fixture.display()
    );

    // Quick sanity check that the JSON parses.
    let contents = std::fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", fixture.display()));
    let _: Value = serde_json::from_str(&contents).expect("fixture must be valid json");
}

#[test]
fn provider_doctests_document_exists() {
    let root = repo_root();
    let doc = root.join("docs/PROVIDER_DOCTESTS.md");
    assert!(
        doc.is_file(),
        "missing provider doctest doc: {}",
        doc.display()
    );
}

#[test]
fn provider_doctests_only_use_rustcode_commands() {
    let root = repo_root();
    let docs_path = root.join("docs/PROVIDER_DOCTESTS.md");
    let markdown = std::fs::read_to_string(&docs_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", docs_path.display()));
    let blocks = extract_doctest_blocks(&markdown);

    for (block_i, block) in blocks.iter().enumerate() {
        for (line_i, line) in block.lines().enumerate() {
            let Some((_envs, args)) = parse_env_and_args(line) else {
                continue;
            };
            assert_eq!(
                args.get(0).map(|s| s.as_str()),
                Some("rustcode"),
                "non-rustcode command in doctest block (block={}, line={}, got={:?})",
                block_i + 1,
                line_i + 1,
                args
            );
        }
    }
}

#[test]
fn provider_doctests_doc_has_at_least_one_doctest_block() {
    let root = repo_root();
    let docs_path = root.join("docs/PROVIDER_DOCTESTS.md");
    let markdown = std::fs::read_to_string(&docs_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", docs_path.display()));
    let blocks = extract_doctest_blocks(&markdown);
    assert!(!blocks.is_empty());
}

#[test]
fn provider_doctests_use_repo_relative_paths() {
    let root = repo_root();
    let docs_path = root.join("docs/PROVIDER_DOCTESTS.md");
    let markdown = std::fs::read_to_string(&docs_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", docs_path.display()));
    let blocks = extract_doctest_blocks(&markdown);

    // Basic sanity: repo-relative docs fixture path should exist.
    assert!(Path::new("docs/fixtures/models_min.json").is_relative());

    for block in blocks {
        for line in block.lines() {
            if line.contains("/Users/") || line.contains("C:\\") {
                panic!("doctest block must not hardcode absolute paths: {line}");
            }
        }
    }
    let _ = root; // keep for future extensions.
}
