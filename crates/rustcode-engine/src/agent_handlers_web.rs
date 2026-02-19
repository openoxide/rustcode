use super::{CommandContext, Duration, Engine, ExecutionError, Regex, StreamExt, CONTENT_TYPE};

const WEBFETCH_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const WEBFETCH_DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const WEBFETCH_MAX_TIMEOUT: Duration = Duration::from_secs(120);
const WEBFETCH_MAX_BODY_BYTES: usize = 1_000_000;

const CODESEARCH_ENDPOINT: &str = "https://mcp.exa.ai/mcp";
const CODESEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const CODESEARCH_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

impl Engine {
    pub(crate) async fn agent_tool_webfetch(
        &self,
        url: &str,
        format: Option<&str>,
        timeout_secs: Option<u64>,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        if !context.config.allow_network {
            return Err(ExecutionError::Dispatch(
                "network access is disabled; set allow_network=true (or RUSTCODE_ALLOW_NETWORK=1) to enable webfetch"
                    .to_string(),
            ));
        }

        let parsed = reqwest::Url::parse(url)
            .map_err(|err| ExecutionError::Dispatch(format!("webfetch url is invalid: {err}")))?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => {
                return Err(ExecutionError::Dispatch(format!(
                    "webfetch url must use http or https (got scheme={other})"
                )));
            }
        }

        let timeout = timeout_secs
            .map_or(WEBFETCH_DEFAULT_TIMEOUT, Duration::from_secs)
            .clamp(Duration::from_secs(1), WEBFETCH_MAX_TIMEOUT);

        let client = reqwest::Client::builder()
            .connect_timeout(WEBFETCH_CONNECT_TIMEOUT)
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::limited(10))
            .no_proxy()
            .build()
            .map_err(|err| {
                ExecutionError::Executor(format!("failed to build webfetch http client: {err}"))
            })?;

        let response = tokio::select! {
            () = context.cancellation.cancelled() => {
                return Err(ExecutionError::Cancelled);
            }
            result = client.get(parsed.clone()).send() => {
                result.map_err(|err| ExecutionError::Executor(format!("webfetch request failed: {err}")))?
            }
        };

        let status = response.status();
        let final_url = response.url().clone();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();

        let is_html = content_type.to_ascii_lowercase().contains("text/html");
        let output_format = format.unwrap_or("markdown").trim().to_ascii_lowercase();

        let mut stream = response.bytes_stream();
        let mut body: Vec<u8> = Vec::new();
        let mut body_truncated = false;
        while let Some(chunk) = tokio::select! {
            () = context.cancellation.cancelled() => {
                return Err(ExecutionError::Cancelled);
            }
            next = stream.next() => next
        } {
            let chunk = chunk.map_err(|err| {
                ExecutionError::Executor(format!("webfetch response stream failed: {err}"))
            })?;
            if body.len().saturating_add(chunk.len()) > WEBFETCH_MAX_BODY_BYTES {
                let remaining = WEBFETCH_MAX_BODY_BYTES.saturating_sub(body.len());
                body.extend_from_slice(&chunk[..remaining.min(chunk.len())]);
                body_truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }

        let raw = String::from_utf8_lossy(&body).to_string();
        let content = if is_html && output_format != "html" {
            html_to_plainish_text(&raw)
        } else {
            raw
        };

        let mut rendered = String::new();
        rendered.push_str("status=");
        rendered.push_str(status.as_str());
        rendered.push('\n');
        rendered.push_str("final_url=");
        rendered.push_str(final_url.as_str());
        rendered.push('\n');
        if !content_type.is_empty() {
            rendered.push_str("content_type=");
            rendered.push_str(&content_type);
            rendered.push('\n');
        }
        rendered.push_str("body_truncated=");
        rendered.push_str(if body_truncated { "true" } else { "false" });
        rendered.push('\n');
        rendered.push_str("content:\n");
        rendered.push_str(&content);
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }

        if status.is_success() {
            Ok(rendered)
        } else {
            Err(ExecutionError::Executor(rendered))
        }
    }
}

impl Engine {
    /// Search for code context, SDK documentation, and library examples using
    /// the Exa MCP code search API.
    ///
    /// Sends a JSON-RPC `tools/call` request to the Exa MCP endpoint and parses
    /// the SSE response, returning the first matching code context block.
    ///
    /// # Errors
    /// Returns `ExecutionError::Dispatch` if network access is disabled or the
    /// query is empty.  Returns `ExecutionError::Executor` on HTTP or parse
    /// failures.
    pub(crate) async fn agent_tool_codesearch(
        &self,
        query: &str,
        tokens_num: Option<u32>,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        if !context.config.allow_network {
            return Err(ExecutionError::Dispatch(
                "codesearch requires network access; set allow_network=true".to_string(),
            ));
        }

        let query = query.trim();
        if query.is_empty() {
            return Err(ExecutionError::Dispatch(
                "codesearch requires a non-empty query".to_string(),
            ));
        }

        let tokens = tokens_num.unwrap_or(5000).clamp(1000, 50_000);

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "get_code_context_exa",
                "arguments": {
                    "query": query,
                    "tokensNum": tokens
                }
            }
        });

        let client = reqwest::Client::builder()
            .connect_timeout(CODESEARCH_CONNECT_TIMEOUT)
            .timeout(CODESEARCH_TIMEOUT)
            .build()
            .map_err(|e| {
                ExecutionError::Executor(format!("failed to build codesearch client: {e}"))
            })?;

        let response = tokio::select! {
            () = context.cancellation.cancelled() => {
                return Err(ExecutionError::Cancelled);
            }
            result = client
                .post(CODESEARCH_ENDPOINT)
                .header("accept", "application/json, text/event-stream")
                .header("content-type", "application/json")
                .json(&body)
                .send() => {
                result.map_err(|e| {
                    ExecutionError::Executor(format!("codesearch request failed: {e}"))
                })?
            }
        };

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ExecutionError::Executor(format!(
                "codesearch error ({status}): {text}"
            )));
        }

        let body_text = tokio::select! {
            () = context.cancellation.cancelled() => {
                return Err(ExecutionError::Cancelled);
            }
            result = response.text() => {
                result.map_err(|e| {
                    ExecutionError::Executor(format!("codesearch response read failed: {e}"))
                })?
            }
        };

        // The Exa endpoint returns SSE: each line may be `data: <json>`.
        // Parse the first result block containing code context.
        for line in body_text.lines() {
            if let Some(json_str) = line.strip_prefix("data: ") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(text) = val
                        .pointer("/result/content/0/text")
                        .and_then(serde_json::Value::as_str)
                    {
                        return Ok(format!("Code search results for: {query}\n\n{text}"));
                    }
                }
            }
        }

        Ok(
            "No code context found for the given query. Try rephrasing, being more \
             specific about the library name, or adjusting the token count."
                .to_string(),
        )
    }
}

fn html_to_plainish_text(html: &str) -> String {
    use std::sync::OnceLock;

    static RE_SCRIPT: OnceLock<Regex> = OnceLock::new();
    static RE_STYLE: OnceLock<Regex> = OnceLock::new();
    static RE_BR: OnceLock<Regex> = OnceLock::new();
    static RE_BLOCK_END: OnceLock<Regex> = OnceLock::new();
    static RE_LI: OnceLock<Regex> = OnceLock::new();
    static RE_TAGS: OnceLock<Regex> = OnceLock::new();
    static RE_WS: OnceLock<Regex> = OnceLock::new();
    static RE_MANY_NEWLINES: OnceLock<Regex> = OnceLock::new();

    let re_script = RE_SCRIPT
        .get_or_init(|| Regex::new(r"(?is)<script[^>]*>.*?</script>").expect("static regex"));
    let re_style =
        RE_STYLE.get_or_init(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").expect("static regex"));
    let re_br = RE_BR.get_or_init(|| Regex::new(r"(?is)<br\s*/?>").expect("static regex"));
    let re_block_end = RE_BLOCK_END.get_or_init(|| {
        Regex::new(r"(?is)</\s*(p|div|li|h[1-6]|tr|table|ul|ol)\s*>").expect("static regex")
    });
    let re_li = RE_LI.get_or_init(|| Regex::new(r"(?is)<\s*li\b[^>]*>").expect("static regex"));
    let re_tags = RE_TAGS.get_or_init(|| Regex::new(r"(?is)<[^>]+>").expect("static regex"));
    let re_ws = RE_WS.get_or_init(|| Regex::new(r"[ \t\x0B\x0C\r]+\n").expect("static regex"));
    let re_many_newlines =
        RE_MANY_NEWLINES.get_or_init(|| Regex::new(r"\n{3,}").expect("static regex"));

    let mut s = re_script.replace_all(html, "").to_string();
    s = re_style.replace_all(&s, "").to_string();
    s = re_br.replace_all(&s, "\n").to_string();
    s = re_block_end.replace_all(&s, "\n").to_string();
    s = re_li.replace_all(&s, "- ").to_string();
    s = re_tags.replace_all(&s, "").to_string();

    s = s
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");

    s = re_ws.replace_all(&s, "\n").to_string();
    s = re_many_newlines.replace_all(&s, "\n\n").to_string();

    s.trim().to_string()
}
