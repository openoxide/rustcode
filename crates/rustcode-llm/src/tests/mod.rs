use super::*;

use rustcode_auth::AuthStore;
use rustcode_core::config::ResolvedConfig;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

mod copilot;
mod parsing;
mod provider_resolution;

static ENV_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

struct AuthFileGuard {
    prev: Option<OsString>,
    path: PathBuf,
}

impl AuthFileGuard {
    fn empty(name: &str) -> Self {
        let prev = std::env::var_os("RUSTCODE_AUTH_FILE");
        let path = make_temp_file_path(name);
        let _ = fs::remove_file(&path);
        std::env::set_var("RUSTCODE_AUTH_FILE", &path);
        Self { prev, path }
    }
}

impl Drop for AuthFileGuard {
    fn drop(&mut self) {
        match self.prev.as_ref() {
            Some(value) => {
                std::env::set_var("RUSTCODE_AUTH_FILE", value);
            }
            None => {
                std::env::remove_var("RUSTCODE_AUTH_FILE");
            }
        }
        let _ = fs::remove_file(&self.path);
    }
}

fn make_temp_file_path(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let pid = std::process::id();
    std::env::temp_dir().join(format!("rustcode-llm-{name}-{pid}-{now}.json"))
}
