use super::{Duration, Engine, CommandContext, ExecutionError, StreamExt, CONTENT_TYPE, Regex};

const WEBFETCH_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const WEBFETCH_DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const WEBFETCH_MAX_TIMEOUT: Duration = Duration::from_secs(120);
const WEBFETCH_MAX_BODY_BYTES: usize = 1_000_000;

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

fn html_to_plainish_text(html: &str) -> String {
    let re_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let re_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let re_br = Regex::new(r"(?is)<br\s*/?>").unwrap();
    let re_block_end = Regex::new(r"(?is)</\s*(p|div|li|h[1-6]|tr|table|ul|ol)\s*>").unwrap();
    let re_li = Regex::new(r"(?is)<\s*li\b[^>]*>").unwrap();
    let re_tags = Regex::new(r"(?is)<[^>]+>").unwrap();
    let re_ws = Regex::new(r"[ \t\x0B\x0C\r]+\n").unwrap();
    let re_many_newlines = Regex::new(r"\n{3,}").unwrap();

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
