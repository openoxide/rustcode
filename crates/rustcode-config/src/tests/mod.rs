use super::*;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

mod loading;
mod mcp;
mod permissions;
mod policy;

static ENV_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn write_config(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("must create parent");
    }
    fs::write(path, contents).expect("must write file");
}

pub(super) fn make_temp_dir(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time must be monotonic")
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("rustcode-config-{name}-{pid}-{now}"));
    fs::create_dir_all(&dir).expect("must create temp dir");
    dir
}
