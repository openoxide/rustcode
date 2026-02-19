use std::time::Duration;

use super::{CommandContext, Engine, ExecutionError};

/// Maximum response body size for web search (64 KB).
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Default timeout for web search requests (15 seconds).
const DEFAULT_TIMEOUT_SECS: u64 = 15;

impl Engine {
    /// Search the web using `DuckDuckGo`'s Instant Answer API.
    ///
    /// Uses `api.duckduckgo.com` (JSON format, no API key required, no CAPTCHA).
    /// Returns the abstract, answer, and related topics for the query.
    ///
    /// For production use with full-text search results, consider integrating
    /// a dedicated search API (e.g., Brave Search, Tavily, `SearXNG`) via the
    /// `RUSTCODE_SEARCH_API_URL` environment variable.
    pub(crate) async fn agent_tool_websearch(
        &self,
        query: &str,
        num_results: Option<u64>,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        if query.trim().is_empty() {
            return Err(ExecutionError::Dispatch(
                "websearch requires a non-empty query".to_string(),
            ));
        }

        if !context.config.allow_network {
            return Err(ExecutionError::Dispatch(
                "websearch requires network access (allow_network=true)".to_string(),
            ));
        }

        let num_results = num_results.unwrap_or(5).min(10) as usize;

        let encoded_query = urlencoding::encode(query);
        let url = format!(
            "https://api.duckduckgo.com/?q={encoded_query}&format=json&no_redirect=1&no_html=1"
        );

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .user_agent("rustcode/0.1")
            .build()
            .map_err(|err| ExecutionError::Executor(format!("http client error: {err}")))?;

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|err| ExecutionError::Executor(format!("search request failed: {err}")))?;

        if !response.status().is_success() {
            return Err(ExecutionError::Executor(format!(
                "search returned status {}",
                response.status()
            )));
        }

        let body = response
            .bytes()
            .await
            .map_err(|err| ExecutionError::Executor(format!("failed to read response: {err}")))?;

        let text = if body.len() > MAX_RESPONSE_BYTES {
            String::from_utf8_lossy(&body[..MAX_RESPONSE_BYTES]).to_string()
        } else {
            String::from_utf8_lossy(&body).to_string()
        };

        // Parse DDG Instant Answer JSON
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|err| ExecutionError::Executor(format!("failed to parse response: {err}")))?;

        let results = extract_ddg_instant_results(&json, num_results);

        if results.is_empty() {
            return Ok(format!(
                "No results found for query: \"{query}\"\n\
                 DuckDuckGo Instant Answer API may not have results for this topic.\n\
                 Try using the webfetch tool to search a specific URL directly."
            ));
        }

        let mut output = format!("Search results for \"{query}\":\n\n");
        for (idx, result) in results.iter().enumerate() {
            output.push_str(&format!(
                "{}. {}\n   {}\n",
                idx + 1,
                result.title,
                result.snippet
            ));
            if !result.url.is_empty() {
                output.push_str(&format!("   URL: {}\n", result.url));
            }
            output.push('\n');
        }

        Ok(output)
    }
}

struct SearchResult {
    title: String,
    snippet: String,
    url: String,
}

/// Extract search results from DDG Instant Answer JSON response.
///
/// The API returns:
/// - `Abstract`: short summary
/// - `AbstractURL`: source URL
/// - `Answer`: direct answer (e.g., calculations)
/// - `RelatedTopics`: array of related topic objects with `Text` and `FirstURL`
fn extract_ddg_instant_results(json: &serde_json::Value, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    // 1. Check for a direct abstract
    let abstract_text = json["Abstract"].as_str().unwrap_or("");
    let abstract_url = json["AbstractURL"].as_str().unwrap_or("");
    let abstract_source = json["AbstractSource"].as_str().unwrap_or("");
    if !abstract_text.is_empty() {
        results.push(SearchResult {
            title: format!("{abstract_source} (Abstract)"),
            snippet: abstract_text.to_string(),
            url: abstract_url.to_string(),
        });
    }

    // 2. Check for a direct answer
    let answer = json["Answer"].as_str().unwrap_or("");
    if !answer.is_empty() {
        results.push(SearchResult {
            title: "Direct Answer".to_string(),
            snippet: answer.to_string(),
            url: String::new(),
        });
    }

    // 3. Extract related topics
    if let Some(topics) = json["RelatedTopics"].as_array() {
        for topic in topics {
            if results.len() >= max_results {
                break;
            }

            // Topics can be either direct items or sub-categories
            if let Some(text) = topic["Text"].as_str() {
                let url = topic["FirstURL"].as_str().unwrap_or("").to_string();
                // Text often starts with the title followed by description
                let (title, snippet) = if let Some(dash) = text.find(" - ") {
                    (text[..dash].to_string(), text[dash + 3..].to_string())
                } else {
                    (text.chars().take(60).collect::<String>(), text.to_string())
                };
                results.push(SearchResult {
                    title,
                    snippet,
                    url,
                });
            }
            // Sub-categories: { "Name": "...", "Topics": [...] }
            if let Some(sub_topics) = topic["Topics"].as_array() {
                for sub in sub_topics {
                    if results.len() >= max_results {
                        break;
                    }
                    if let Some(text) = sub["Text"].as_str() {
                        let url = sub["FirstURL"].as_str().unwrap_or("").to_string();
                        let (title, snippet) = if let Some(dash) = text.find(" - ") {
                            (text[..dash].to_string(), text[dash + 3..].to_string())
                        } else {
                            (text.chars().take(60).collect::<String>(), text.to_string())
                        };
                        results.push(SearchResult {
                            title,
                            snippet,
                            url,
                        });
                    }
                }
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_abstract_result() {
        let json = serde_json::json!({
            "Abstract": "Rust is a programming language.",
            "AbstractURL": "https://en.wikipedia.org/wiki/Rust",
            "AbstractSource": "Wikipedia",
            "Answer": "",
            "RelatedTopics": []
        });
        let results = extract_ddg_instant_results(&json, 5);
        assert_eq!(results.len(), 1);
        assert!(results[0].title.contains("Wikipedia"));
        assert!(results[0].snippet.contains("programming language"));
    }

    #[test]
    fn extract_answer_result() {
        let json = serde_json::json!({
            "Abstract": "",
            "AbstractURL": "",
            "AbstractSource": "",
            "Answer": "42",
            "RelatedTopics": []
        });
        let results = extract_ddg_instant_results(&json, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Direct Answer");
        assert_eq!(results[0].snippet, "42");
    }

    #[test]
    fn extract_related_topics() {
        let json = serde_json::json!({
            "Abstract": "",
            "AbstractURL": "",
            "AbstractSource": "",
            "Answer": "",
            "RelatedTopics": [
                { "Text": "Topic One - Description of topic one", "FirstURL": "https://example.com/1" },
                { "Text": "Topic Two - Description of topic two", "FirstURL": "https://example.com/2" }
            ]
        });
        let results = extract_ddg_instant_results(&json, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Topic One");
        assert!(results[0].snippet.contains("Description of topic one"));
    }

    #[test]
    fn extract_empty_response() {
        let json = serde_json::json!({
            "Abstract": "",
            "AbstractURL": "",
            "AbstractSource": "",
            "Answer": "",
            "RelatedTopics": []
        });
        let results = extract_ddg_instant_results(&json, 5);
        assert!(results.is_empty());
    }

    #[test]
    fn extract_respects_max_results() {
        let json = serde_json::json!({
            "Abstract": "Some abstract",
            "AbstractURL": "",
            "AbstractSource": "Source",
            "Answer": "42",
            "RelatedTopics": [
                { "Text": "T1 - D1", "FirstURL": "" },
                { "Text": "T2 - D2", "FirstURL": "" },
                { "Text": "T3 - D3", "FirstURL": "" }
            ]
        });
        let results = extract_ddg_instant_results(&json, 3);
        assert_eq!(results.len(), 3); // abstract + answer + 1 topic
    }
}
