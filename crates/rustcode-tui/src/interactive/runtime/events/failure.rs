pub(super) fn clean_failure_message(msg: &str) -> String {
    let lower = msg.to_lowercase();

    if lower.contains("authfailed")
        || lower.contains("authentication failed")
        || lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
        || lower.contains("unauthorized")
        || lower.contains("api key")
        || (lower.contains("401") && lower.contains("provider"))
    {
        return "Authentication failed -- connect your provider in /providers (Ctrl+A)".to_string();
    }

    if lower.contains("ratelimit")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("quota")
        || lower.contains("429")
    {
        return "Rate limit or usage quota reached -- please wait before retrying".to_string();
    }

    if lower.contains("contextoverflow")
        || lower.contains("context limit")
        || lower.contains("context_length_exceeded")
        || lower.contains("context window")
        || lower.contains("maximum context")
    {
        return "Context limit exceeded -- use /compact or send a shorter message".to_string();
    }

    if lower.contains("serviceunavailable")
        || lower.contains("service unavailable")
        || lower.contains("temporarily unavailable")
        || lower.contains("overloaded")
        || lower.contains("internal server error")
        || lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
    {
        return "Provider temporarily unavailable -- please try again".to_string();
    }

    if lower.contains("invalidrequest") {
        return "Request rejected by provider -- see activity for details".to_string();
    }

    if lower.contains("configuration error")
        || lower.contains("not set")
        || lower.contains("api key env")
        || lower.contains("executor not available")
        || lower.contains("llm init failed")
    {
        return "No provider connected -- use /providers (Ctrl+A) to connect one".to_string();
    }

    if lower.contains("timed out waiting for provider response chunk") {
        return "Stream timed out -- model may be slow to respond, try again".to_string();
    }
    if lower.contains("timed out waiting for provider response headers") {
        return "Connection timed out -- provider did not respond, retries exhausted".to_string();
    }

    if lower.contains("connection reset") || lower.contains("connection refused") {
        return "Connection lost -- check your network and try again".to_string();
    }

    if lower.contains("network error")
        || lower.contains("connection")
        || lower.contains("timed out")
        || lower.contains("transport")
    {
        return "Network error -- check your connection and try again".to_string();
    }

    if let Some(idx) = msg.find(": ") {
        let rest = msg[idx + 3..].trim();
        if !rest.is_empty() && rest.len() < 120 {
            return format!("Request failed: {rest}");
        }
    }

    let capped: String = msg.chars().take(120).collect();
    if capped.len() < msg.len() {
        format!("{capped}...")
    } else {
        capped
    }
}
