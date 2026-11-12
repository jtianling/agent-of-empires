use super::*;

#[test]
fn overlapping_keys_and_bearer_are_redacted_longest_first() {
    let directory = tempfile::tempdir().unwrap();
    xats_control::test_support::write_pid_file(directory.path(), 8123, std::process::id());
    let control = xats_control::discover_control_plane_in(
        directory.path(),
        Some("short-key-long-key-token-tail".into()),
    )
    .unwrap();
    let plan = targets::Plan {
        targets: Vec::new(),
        missing_keys: vec!["short-key".into(), "short-key-long-key".into()],
        requires_daemon: true,
    };
    let error = redact(
        Err(anyhow::anyhow!(
            "short-key short-key-long-key short-key-long-key-token-tail"
        )),
        &plan,
        &control,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "*** *** ***");
}
