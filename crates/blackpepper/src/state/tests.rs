use super::{
    ensure_workspace_ports, remove_workspace_ports, rename_workspace_ports, workspace_port_env,
    PORT_BLOCK_SIZE,
};
use std::env;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

use crate::test_utils::env_lock;

fn with_state_path<T>(path: &Path, action: impl FnOnce() -> T) -> T {
    let _guard = env_lock();
    let key = "BLACKPEPPER_STATE_PATH";
    let previous = env::var(key).ok();
    env::set_var(key, path);
    let result = action();
    match previous {
        Some(value) => env::set_var(key, value),
        None => env::remove_var(key),
    }
    result
}

#[test]
fn workspace_ports_allocate_and_reuse() {
    let dir = TempDir::new().expect("temp dir");
    let state_path = dir.path().join("state.toml");
    let work_one = dir.path().join("work-one");
    let work_two = dir.path().join("work-two");
    let work_three = dir.path().join("work-three");
    fs::create_dir_all(&work_one).expect("create work-one");
    fs::create_dir_all(&work_two).expect("create work-two");
    fs::create_dir_all(&work_three).expect("create work-three");

    with_state_path(&state_path, || {
        let base_one = ensure_workspace_ports(&work_one).expect("allocate work-one");
        let base_one_again = ensure_workspace_ports(&work_one).expect("reuse work-one");
        assert_eq!(base_one, base_one_again);

        let base_two = ensure_workspace_ports(&work_two).expect("allocate work-two");
        assert_eq!(base_two, base_one + PORT_BLOCK_SIZE);

        remove_workspace_ports(&work_one).expect("remove work-one");
        let base_three = ensure_workspace_ports(&work_three).expect("allocate work-three");
        assert_eq!(base_three, base_one);

        let env_vars = workspace_port_env(base_three);
        assert_eq!(env_vars.len() as u16, PORT_BLOCK_SIZE);
        assert_eq!(env_vars[0].0, "WORKSPACE_PORT_0");
        assert_eq!(env_vars[0].1, base_three.to_string());
    });
}

#[test]
fn workspace_ports_rename_moves_assignment() {
    let dir = TempDir::new().expect("temp dir");
    let state_path = dir.path().join("state.toml");
    let work_one = dir.path().join("work-one");
    let work_two = dir.path().join("work-two");
    fs::create_dir_all(&work_one).expect("create work-one");
    fs::create_dir_all(&work_two).expect("create work-two");

    with_state_path(&state_path, || {
        let base_one = ensure_workspace_ports(&work_one).expect("allocate work-one");
        rename_workspace_ports(&work_one, &work_two).expect("rename work-two");

        let base_two = ensure_workspace_ports(&work_two).expect("allocate work-two");
        assert_eq!(base_two, base_one);

        let base_one_reused = ensure_workspace_ports(&work_one).expect("reallocate work-one");
        assert_eq!(base_one_reused, base_one + PORT_BLOCK_SIZE);
    });
}
