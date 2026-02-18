mod callback;
mod browser;
mod device;

pub use browser::{
    complete_browser_oauth_flow, complete_mcp_browser_oauth_flow, normalize_domain,
    start_browser_oauth_flow, start_mcp_browser_oauth_flow, BrowserOAuthFlowStart,
    McpBrowserOAuthFlowStart,
};
pub use device::{
    poll_device_code_flow_for_api_key, poll_device_code_flow_for_credential,
    start_device_code_flow, DeviceCodeFlowCredential, DeviceCodeFlowStart,
};

#[cfg(test)]
pub(crate) use browser::{extract_openai_account_id_from_jwt, GITLAB_BUNDLED_CLIENT_ID};
#[cfg(test)]
pub(crate) use device::deserialize_u64_string_or_number;
