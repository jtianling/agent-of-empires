use super::*;

fn fixture() -> (Instance, Server) {
    let mut instance = Instance::new("owned", "/tmp");
    instance.source_profile = "owned-codex-test".into();
    instance.tool = "codex".into();
    let mut identity =
        crate::managed_xats::ManagedXats::new("worker".into(), "test".into()).unwrap();
    let launch = identity.launch_instance_id.clone();
    identity.claim = Some(
        serde_json::from_value(serde_json::json!({
            "fence": {"claim_id": uuid::Uuid::new_v4().to_string(),
                "instance_id": launch, "generation": 1},
            "mcp_token": "test-token", "runtime": null
        }))
        .unwrap(),
    );
    instance.managed_xats = Some(identity);
    let server = Server {
        pane_id: instance.primary_pane_id().into(),
        launch_id: launch,
        pid: std::process::id(),
        start_identity: "incorrect-start-identity".into(),
        endpoint: Some("ws://127.0.0.1:45678/".into()),
        members: Vec::new(),
    };
    let _lock = lock(&instance).unwrap();
    (instance, server)
}

#[test]
#[serial_test::serial]
fn reused_pid_is_rejected_without_removing_record() {
    let directory = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(directory.path());
    let (instance, server) = fixture();
    write_record(&instance, &server).unwrap();
    let original = crate::process::start_identity(server.pid).unwrap();
    assert!(stop(&instance, false)
        .unwrap_err()
        .to_string()
        .contains("identity changed"));
    assert_eq!(
        crate::process::start_identity(server.pid).unwrap(),
        original
    );
    assert!(record_path(&instance).unwrap().exists());
}

#[test]
#[serial_test::serial]
fn retiring_wrapper_cannot_stop_new_launch() {
    let directory = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(directory.path());
    let (instance, mut server) = fixture();
    server.launch_id = uuid::Uuid::new_v4().to_string();
    write_record(&instance, &server).unwrap();
    stop(&instance, true).unwrap();
    assert_eq!(
        read_record(&instance).unwrap().unwrap().launch_id,
        server.launch_id
    );
}

#[test]
#[serial_test::serial]
fn foreign_endpoint_is_rejected_before_process_signal() {
    let directory = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(directory.path());
    let (instance, server) = fixture();
    write_record(&instance, &server).unwrap();
    assert!(verify(&instance, "ws://127.0.0.1:45679/")
        .unwrap_err()
        .to_string()
        .contains("endpoint changed"));
    assert!(record_path(&instance).unwrap().exists());
}

#[test]
#[serial_test::serial]
fn unsafe_process_and_other_pane_records_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(directory.path());
    let (instance, mut server) = fixture();
    server.pid = 1;
    write_record(&instance, &server).unwrap();
    assert!(read_record(&instance).is_err());
    server.pid = std::process::id();
    server.pane_id = "0123456789abcdef".into();
    write_record(&instance, &server).unwrap();
    assert!(read_record(&instance).is_err());
}

#[test]
#[serial_test::serial]
fn server_header_override_is_valid_without_exposing_token_or_runtime_profile_flag() {
    let directory = tempfile::tempdir().unwrap();
    let _home = crate::session::scoped_test_home(directory.path());
    let log = File::create(directory.path().join("stderr.log")).unwrap();
    let (instance, _) = fixture();
    let mut prepared = crate::managed_codex::PreparedThread {
        thread_id: String::new(),
        provider: "test-provider".into(),
        server_name: "xats".into(),
        headers: serde_json::json!({}),
        yolo: true,
        model: None,
        effort: None,
        error: None,
    };
    let command = server_command(
        std::path::Path::new("/not-executed/codex"),
        &instance,
        &prepared,
        &url::Url::parse("http://127.0.0.1:1234/mcp").unwrap(),
        log,
    )
    .unwrap();
    let args = command
        .get_args()
        .map(|value| value.to_str().unwrap())
        .collect::<Vec<_>>();
    assert!(!args.contains(&"-p"));
    let trust = crate::codex_permissions::project_trust_override(&instance.project_path);
    assert!(args.contains(&trust.as_str()));
    assert!(command.get_envs().any(|(key, value)| key
        == crate::managed_runtimes::MANAGER_CONTEXT_ENV
        && value.is_none()));
    assert!(command
        .get_envs()
        .any(|(key, value)| key == crate::managed_runtimes::MCP_TOKEN_ENV && value.is_none()));
    assert!(!args.iter().any(|value| value.contains("test-token")));
    let header = args
        .iter()
        .find(|value| value.contains(".env_http_headers."))
        .unwrap();
    let key = header.split_once('=').unwrap().0;
    let fields = key.split('.').collect::<Vec<_>>();
    assert_eq!(
        fields,
        [
            "mcp_servers",
            "xats",
            "env_http_headers",
            super::super::HEADER
        ]
    );
    assert!(reqwest::header::HeaderName::from_bytes(fields[3].as_bytes()).is_ok());
    assert!(command
        .get_envs()
        .any(|(key, value)| key == TOKEN_ENV && value == Some(std::ffi::OsStr::new("test-token"))));
    prepared.yolo = false;
    let log = File::create(directory.path().join("non-yolo.log")).unwrap();
    let command = server_command(
        std::path::Path::new("/not-executed/codex"),
        &instance,
        &prepared,
        &url::Url::parse("http://127.0.0.1:1234/mcp").unwrap(),
        log,
    )
    .unwrap();
    assert!(!command
        .get_args()
        .any(|value| value == std::ffi::OsStr::new(&trust)));
}
