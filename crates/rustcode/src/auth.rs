use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rustcode_auth::{
    complete_browser_oauth_flow, methods_for_provider, poll_device_code_flow_for_credential,
    start_browser_oauth_flow, start_device_code_flow, AuthMethod, AuthStore, DeviceCodeFlowCredential,
    StoredCredential,
};

use crate::cli::AuthCommand;
use crate::utils::{is_interactive_terminal, write_stdout_line, write_stdout_raw};

mod catalog;

pub use catalog::{AuthMethodRow, ModelsProvider, load_models_index};

use catalog::{
    list_auth_login_providers, list_auth_methods, methods_as_csv, resolve_auth_method_rows,
};

pub async fn handle_auth_command(command: AuthCommand, json_output: bool) -> Result<()> {
    let store = AuthStore::open_default();

    match command {
        AuthCommand::List => handle_auth_list(&store, json_output)?,
        AuthCommand::Status { provider } => handle_auth_status(&store, &provider, json_output)?,
        AuthCommand::Methods { provider } => {
            if let Some(provider) = provider {
                handle_auth_methods_provider(&provider, json_output)?;
            } else {
                list_auth_methods(json_output)?;
            }
        }
        AuthCommand::SetKey {
            provider,
            from_env,
            domain,
        } => {
            let key = std::env::var(&from_env).context(format!("env var {from_env} not set"))?;
            store.set_api_key_with_domain(&provider, &key, domain.as_deref())?;

            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "auth.set-key",
                    "provider": provider,
                    "action": "set_key",
                    "credential": "stored:api_key",
                    "auth_file": store.path().display().to_string(),
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line(&format!("stored api key for provider={provider}"))?;
            }
        }
        AuthCommand::SetOauth {
            provider,
            access_env,
            refresh_env,
            expires_unix,
            account_id,
        } => {
            let access =
                std::env::var(&access_env).context(format!("env var {access_env} not set"))?;
            let refresh = refresh_env
                .and_then(|name| std::env::var(&name).ok())
                .filter(|value| !value.trim().is_empty());

            store.set_oauth(
                &provider,
                &access,
                refresh.as_deref(),
                expires_unix,
                account_id.as_deref(),
            )?;

            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "auth.set-oauth",
                    "provider": provider,
                    "action": "set_oauth",
                    "credential": "stored:oauth",
                    "auth_file": store.path().display().to_string(),
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line(&format!("stored oauth for provider={provider}"))?;
            }
        }
        AuthCommand::Remove { provider } => {
            let removed = store.remove(&provider)?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "auth.remove",
                    "provider": provider,
                    "action": "remove",
                    "removed": removed,
                    "auth_file": store.path().display().to_string(),
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else if removed {
                write_stdout_line(&format!("removed credential for provider={provider}"))?;
            } else {
                write_stdout_line(&format!("no credential for provider={provider}"))?;
            }
        }
        AuthCommand::Login {
            provider,
            method,
            domain,
            from_env,
            no_wait,
            timeout_secs,
            oauth_port,
        } => {
            if provider.is_none() {
                if from_env.is_some() {
                    anyhow::bail!("`--from-env` requires a provider");
                }
                list_auth_login_providers(json_output)?;
                return Ok(());
            }

            let provider = provider.expect("checked above");
            let (rows, warning) = resolve_auth_method_rows();
            let row = rows
                .into_iter()
                .find(|item| item.id == provider)
                .context(format!(
                    "unknown provider {provider}{}",
                    warning.map(|value| format!(" ({value})")).unwrap_or_default()
                ))?;

            let method = resolve_login_method_with_context(method.as_deref(), from_env.as_deref(), &row)?;

            match method {
                AuthMethod::ApiKey => {
                    let (key, source) = if let Some(env_var) = from_env {
                        let key =
                            std::env::var(&env_var).context(format!("env var {env_var} not set"))?;
                        (key, "env")
                    } else if is_interactive_terminal() {
                        print_provider_api_key_hint(&provider)?;
                        (prompt_for_api_key(&provider)?, "prompt")
                    } else {
                        anyhow::bail!(
                            "api_key login requires --from-env in non-interactive mode"
                        );
                    };

                    store.set_api_key(&provider, &key)?;
                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "command": "auth.login",
                            "provider": provider,
                            "method": "api_key",
                            "stage": "authorized",
                            "source": source,
                            "credential": "stored:api_key",
                            "auth_file": store.path().display().to_string(),
                        });
                        write_stdout_line(&serde_json::to_string(&payload)?)?;
                    } else {
                        write_stdout_line(&format!("stored api key for provider={provider}"))?;
                    }
                }
                AuthMethod::OAuthDeviceCode => {
                    let flow = start_device_code_flow(&provider, domain.as_deref())
                        .await
                        .context("failed to start device code flow")?;

                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "command": "auth.login",
                            "provider": provider,
                            "method": "oauth_device_code",
                            "stage": "awaiting_user_authorization",
                            "verification_uri": flow.verification_uri,
                            "user_code": flow.user_code,
                            "expires_in": flow.expires_in_secs,
                        });
                        write_stdout_line(&serde_json::to_string(&payload)?)?;
                    } else {
                        write_stdout_line(&format!("provider={provider}"))?;
                        write_stdout_line("method=oauth_device_code")?;
                        write_stdout_line(&format!("verification_uri={}", flow.verification_uri))?;
                        write_stdout_line(&format!("user_code={}", flow.user_code))?;
                        write_stdout_line("status=awaiting_user_authorization")?;
                    }

                    if no_wait {
                        return Ok(());
                    }

                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "command": "auth.login",
                            "provider": provider,
                            "method": "oauth_device_code",
                            "stage": "polling_for_token",
                        });
                        write_stdout_line(&serde_json::to_string(&payload)?)?;
                    }

                    let credential =
                        poll_device_code_flow_for_credential(&flow, Duration::from_secs(timeout_secs))
                            .await
                            .context("failed to poll for token")?;
                    persist_oauth_credential(&store, &provider, &credential)?;

                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "command": "auth.login",
                            "provider": provider,
                            "method": "oauth_device_code",
                            "stage": "authorized",
                            "credential": "stored:oauth",
                            "auth_file": store.path().display().to_string(),
                        });
                        write_stdout_line(&serde_json::to_string(&payload)?)?;
                    } else {
                        write_stdout_line("status=authorized")?;
                    }
                }
                AuthMethod::OAuthBrowser => {
                    let browser_client_id = browser_client_id_for_provider(&provider, domain.as_deref());
                    let flow = start_browser_oauth_flow(
                        &provider,
                        domain.as_deref(),
                        browser_client_id.as_deref(),
                        oauth_port,
                    )
                    .context("failed to start browser oauth flow")?;

                    if json_output {
                        let challenge = serde_json::json!({
                            "schema_version": 1,
                            "command": "auth.login",
                            "provider": provider,
                            "method": "oauth_browser",
                            "stage": "challenge",
                            "authorize_url": flow.authorize_url,
                            "redirect_uri": flow.redirect_uri,
                            "oauth_port": oauth_port,
                        });
                        write_stdout_line(&serde_json::to_string(&challenge)?)?;

                        let awaiting = serde_json::json!({
                            "schema_version": 1,
                            "command": "auth.login",
                            "provider": provider,
                            "method": "oauth_browser",
                            "stage": "awaiting_browser_callback",
                        });
                        write_stdout_line(&serde_json::to_string(&awaiting)?)?;
                    } else {
                        write_stdout_line(&format!("provider={provider}"))?;
                        write_stdout_line("method=oauth_browser")?;
                        write_stdout_line(&format!("authorize_url={}", flow.authorize_url))?;
                        write_stdout_line(&format!("redirect_uri={}", flow.redirect_uri))?;
                        write_stdout_line("status=awaiting_browser_callback")?;
                    }

                    if no_wait {
                        return Ok(());
                    }

                    let credential = complete_browser_oauth_flow(
                        &flow,
                        Duration::from_secs(timeout_secs),
                        None,
                    )
                    .await
                    .context("failed to complete browser oauth flow")?;
                    persist_oauth_credential(&store, &provider, &credential)?;

                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "command": "auth.login",
                            "provider": provider,
                            "method": "oauth_browser",
                            "stage": "authorized",
                            "credential": "stored:oauth",
                            "auth_file": store.path().display().to_string(),
                        });
                        write_stdout_line(&serde_json::to_string(&payload)?)?;
                    } else {
                        write_stdout_line("status=authorized")?;
                    }
                }
            }
        }
    }

    Ok(())
}

fn handle_auth_list(store: &AuthStore, json_output: bool) -> Result<()> {
    let mut providers = store.providers()?;
    providers.sort();

    if json_output {
        let rows = providers
            .iter()
            .map(|provider| {
                let credential = render_stored_credential(store.get(provider).ok().flatten().as_ref());
                serde_json::json!({
                    "id": provider,
                    "credential": credential,
                })
            })
            .collect::<Vec<_>>();
        let payload = serde_json::json!({
            "schema_version": 1,
            "auth_file": store.path().display().to_string(),
            "providers": rows,
        });
        write_stdout_line(&serde_json::to_string(&payload)?)?;
    } else {
        write_stdout_line(&format!("providers={}", providers.len()))?;
        for provider in providers {
            let credential = render_stored_credential(store.get(&provider)?.as_ref());
            write_stdout_line(&format!("provider={provider}\tcredential={credential}"))?;
        }
    }

    Ok(())
}

fn handle_auth_status(store: &AuthStore, provider: &str, json_output: bool) -> Result<()> {
    let credential = store.get(provider)?;
    let (rows, _) = resolve_auth_method_rows();
    let methods = rows
        .into_iter()
        .find(|row| row.id == provider).map_or_else(|| methods_for_provider(provider), |row| row.methods);

    if json_output {
        let payload = serde_json::json!({
            "schema_version": 1,
            "command": "auth.status",
            "provider": provider,
            "credential": render_stored_credential(credential.as_ref()),
            "methods": methods.iter().map(|method| method.as_str()).collect::<Vec<_>>(),
            "auth_file": store.path().display().to_string(),
        });
        write_stdout_line(&serde_json::to_string(&payload)?)?;
    } else {
        write_stdout_line(&format!("provider={provider}"))?;
        write_stdout_line(&format!(
            "credential={}",
            render_stored_credential(credential.as_ref())
        ))?;
        write_stdout_line(&format!("methods={}", methods_as_csv(&methods)))?;
        write_stdout_line(&format!("auth_file={}", store.path().display()))?;
    }

    Ok(())
}

fn handle_auth_methods_provider(provider: &str, json_output: bool) -> Result<()> {
    let (rows, _) = resolve_auth_method_rows();
    let methods = rows
        .into_iter()
        .find(|row| row.id == provider).map_or_else(|| methods_for_provider(provider), |row| row.methods);

    if json_output {
        let payload = serde_json::json!({
            "schema_version": 1,
            "command": "auth.methods",
            "provider": provider,
            "methods": methods.iter().map(|method| method.as_str()).collect::<Vec<_>>(),
        });
        write_stdout_line(&serde_json::to_string(&payload)?)?;
    } else {
        write_stdout_line(&format!("provider={provider}\tmethods={}", methods_as_csv(&methods)))?;
    }

    Ok(())
}

pub fn render_stored_credential(credential: Option<&StoredCredential>) -> &'static str {
    match credential {
        Some(StoredCredential::ApiKey { .. }) => "stored:api_key",
        Some(StoredCredential::OAuth { .. }) => "stored:oauth",
        None => "none",
    }
}

pub fn classify_auth_error(err: &anyhow::Error) -> &'static str {
    let message = err.to_string().to_ascii_lowercase();
    if message.contains("validation") || message.contains("requires") || message.contains("not set")
    {
        "validation"
    } else if message.contains("network") || message.contains("dns") || message.contains("connect") {
        "network"
    } else {
        "provider"
    }
}

pub fn resolve_login_method(method: Option<&str>, available: &[AuthMethod]) -> Result<AuthMethod> {
    if let Some(method) = method {
        return match method {
            "api_key" => Ok(AuthMethod::ApiKey),
            "oauth_device_code" => Ok(AuthMethod::OAuthDeviceCode),
            "oauth_browser" => Ok(AuthMethod::OAuthBrowser),
            other => anyhow::bail!(
                "invalid auth method: {other} (expected one of: api_key, oauth_device_code, oauth_browser)"
            ),
        };
    }

    if available.len() == 1 {
        return Ok(available[0]);
    }

    anyhow::bail!("multiple auth methods available; use --method <method>")
}

pub fn resolve_login_method_with_context(
    method: Option<&str>,
    from_env: Option<&str>,
    row: &AuthMethodRow,
) -> Result<AuthMethod> {
    if from_env.is_some() {
        if let Some(raw) = method {
            if raw != "api_key" {
                anyhow::bail!("--from-env is only compatible with --method api_key");
            }
        }
        return Ok(AuthMethod::ApiKey);
    }

    if let Some(raw) = method {
        return resolve_login_method(Some(raw), &row.methods);
    }

    if is_interactive_terminal() {
        prompt_for_login_method(&row.id, &row.methods)
    } else {
        resolve_login_method(None, &row.methods)
    }
}

pub fn prompt_for_login_method(provider: &str, available: &[AuthMethod]) -> Result<AuthMethod> {
    write_stdout_line(&format!("Select authentication method for {provider}:"))?;
    for (index, method) in available.iter().enumerate() {
        write_stdout_line(&format!("  {}. {}", index + 1, method.as_str()))?;
    }
    write_stdout_raw("Selection: ")?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if let Ok(index) = input.parse::<usize>() {
        if index > 0 && index <= available.len() {
            return Ok(available[index - 1]);
        }
    }

    for method in available {
        if method.as_str() == input {
            return Ok(*method);
        }
    }

    anyhow::bail!("invalid selection")
}

pub fn prompt_for_api_key(provider: &str) -> Result<String> {
    let input = rpassword::prompt_password(format!("Enter API key for {provider}: "))?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        anyhow::bail!("api key cannot be empty");
    }
    Ok(trimmed.to_string())
}

pub fn print_provider_api_key_hint(provider: &str) -> Result<()> {
    match provider {
        "opencode" => {
            write_stdout_line("Get your API key at: https://opencode.ai/auth")?;
        }
        "vercel" => {
            write_stdout_line("Get your API key at: https://vercel.link/ai-gateway-token")?;
        }
        _ => {}
    }
    Ok(())
}

fn persist_oauth_credential(
    store: &AuthStore,
    provider: &str,
    credential: &DeviceCodeFlowCredential,
) -> Result<()> {
    let expires_unix = credential.expires_in_secs.and_then(|expires| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        i64::try_from(now.saturating_add(expires)).ok()
    });

    store.set_oauth(
        provider,
        &credential.access_token,
        credential.refresh_token.as_deref(),
        expires_unix,
        credential.account_id.as_deref(),
    )?;
    Ok(())
}

fn browser_client_id_for_provider(provider: &str, domain: Option<&str>) -> Option<String> {
    if provider != "gitlab" {
        return None;
    }

    match domain {
        None => None,
        Some(raw) if raw.eq_ignore_ascii_case("gitlab.com") => None,
        Some(_) => std::env::var("GITLAB_OAUTH_CLIENT_ID").ok(),
    }
}
