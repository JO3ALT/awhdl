//! Test roots: a sanitized fixture project inside the repository, and the
//! optional deployment project that surrounds `awhdl/` on a development machine.
use std::path::PathBuf;

pub fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project")
}

pub fn deployment_root() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    root.join("config/runtime.toml").is_file().then_some(root)
}
