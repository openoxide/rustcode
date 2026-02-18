use std::time::Duration;

use super::{CommandContext, Engine, ExecutionError};

/// Maximum response body size for web search (64 KB).
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Default timeout for web search requests (15 seconds).
const DEFAULT_TIMEOUT_SECS: u64 = 15;

impl Engine {
    /// Search the web using a search engine API.
    ///
    /// Currently uses a simple HTTP fetch approach — queries a search engine
    /// and returns summarized results. For production use, integrate with
    /// a dedicated search API (e.g. Brave Search, Tavily, SearXNG).
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

        // Use DuckDuckGo Lite as the default search backend (no API key required)
        let encoded_query = urlencoding::encode(query);
        let url = format!("https://lite.duckduckgo.com/lite/?q={encoded_query}");

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

        // Extract search results from DDG Lite HTML (simple text extraction)
        let results = extract_ddg_results(&text, num_results);

        if results.is_empty() {
            return Ok(format!(
                "No results found for query: \"{query}\"\n\
                 Try refining your search terms."
            ));
        }

        let mut output = format!("Search results for \"{query}\":\n\n");
        for (idx, result) in results.iter().enumerate() {
            output.push_str(&format!(
                "{}. {}\n   {}\n\n",
                idx + 1,
                result.title,
                result.snippet
            ));
        }

        Ok(output)
    }
}

struct SearchResult {
    title: String,
    snippet: String,
}

/// Extract search results from DuckDuckGo Lite HTML.
///
/// DDG Lite returns a simple HTML table with results. We extract
/// titles and snippets using basic string matching.
fn extract_ddg_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    // DDG Lite uses <a> tags for result links and <td> for snippets
    // Simple extraction: find result links and their following text
    let mut pos = 0;
    while results.len() < max_results {
        // Find next result link
        let link_start = match html[pos..].find("class=\"result-link\"") {
            Some(idx) => pos + idx,
            None => break,
        };

        // Extract title from the <a> tag
        let title = if let Some(close) = html[link_start..].find('>') {
            let after = link_start + close + 1;
            if let Some(end) = html[after..].find("</a>") {
                strip_html_tags(&html[after..after + end]).trim().to_string()
            } else {
                pos = link_start + 1;
                continue;
            }
        } else {
            pos = link_start + 1;
            continue;
        };

        // Find snippet — usually in the next <td class="result-snippet">
        let snippet_search_start = link_start + 50;
        let snippet = if snippet_search_start < html.len() {
            if let Some(snippet_idx) = html[snippet_search_start..].find("result-snippet") {
                let snip_start = snippet_search_start + snippet_idx;
                if let Some(close) = html[snip_start..].find('>') {
                    let after = snip_start + close + 1;
                    if let Some(end) = html[after..].find("</td>") {
                        strip_html_tags(&html[after..after + end])
                            .trim()
                            .to_string()
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if !title.is_empty() {
            results.push(SearchResult { title, snippet });
        }

        pos = link_start + 1;
    }

    results
}

/// Strip HTML tags from a string, returning plain text.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }
    // Normalize whitespace
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_tags_basic() {
        assert_eq!(strip_html_tags("<b>hello</b>"), "hello");
        assert_eq!(strip_html_tags("no tags"), "no tags");
        assert_eq!(strip_html_tags("<a href=\"x\">link</a> text"), "link text");
    }

    #[test]
    fn extract_empty_html() {
        let results = extract_ddg_results("", 5);
        assert!(results.is_empty());
    }
}
