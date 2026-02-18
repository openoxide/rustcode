use super::*;

#[test]
fn session_new_list_show_and_fork_round_trip() {
    let sessions_dir = make_temp_dir_path("sessions-round-trip");

    let new_output = Command::new(rustcode_bin())
        .args(["session", "new", "--title", "t1"])
        .env("RUSTCODE_SESSIONS_DIR", &sessions_dir)
        .output()
        .expect("run rustcode session new");
    assert!(new_output.status.success());
    let stdout = String::from_utf8(new_output.stdout).expect("stdout must be utf8");
    let id = stdout
        .lines()
        .find_map(|line| line.strip_prefix("id=").map(str::to_string))
        .expect("must print id=");

    let list_output = Command::new(rustcode_bin())
        .args(["session", "list"])
        .env("RUSTCODE_SESSIONS_DIR", &sessions_dir)
        .output()
        .expect("run rustcode session list");
    assert!(list_output.status.success());
    let stdout = String::from_utf8(list_output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains(&format!("id={id}\t")), "stdout={stdout}");

    let show_output = Command::new(rustcode_bin())
        .args(["session", "show", &id])
        .env("RUSTCODE_SESSIONS_DIR", &sessions_dir)
        .output()
        .expect("run rustcode session show");
    assert!(show_output.status.success());
    let stdout = String::from_utf8(show_output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains(&format!("id={id}")), "stdout={stdout}");
    assert!(stdout.contains("title=t1"), "stdout={stdout}");

    let fork_output = Command::new(rustcode_bin())
        .args(["session", "fork", &id, "--title", "forked"])
        .env("RUSTCODE_SESSIONS_DIR", &sessions_dir)
        .output()
        .expect("run rustcode session fork");
    assert!(fork_output.status.success());
    let stdout = String::from_utf8(fork_output.stdout).expect("stdout must be utf8");
    let fork_id = stdout
        .lines()
        .find_map(|line| line.strip_prefix("id=").map(str::to_string))
        .expect("must print id=");

    let fork_show_output = Command::new(rustcode_bin())
        .args(["session", "show", &fork_id])
        .env("RUSTCODE_SESSIONS_DIR", &sessions_dir)
        .output()
        .expect("run rustcode session show fork");
    assert!(fork_show_output.status.success());
    let stdout = String::from_utf8(fork_show_output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains(&format!("id={fork_id}")), "stdout={stdout}");
    assert!(stdout.contains("title=forked"), "stdout={stdout}");
    assert!(
        stdout.contains(&format!("parent_id={id}")),
        "stdout={stdout}"
    );
}
