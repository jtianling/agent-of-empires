use super::*;

fn settings(tier: Option<&str>) -> RuntimeSettings {
    RuntimeSettings {
        model: "gpt-6.1-sol".into(),
        effort: Some("xhigh".into()),
        service_tier: tier.map(str::to_string),
    }
}

fn store() -> Store {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::ensure_schema(&conn).unwrap();
    Store {
        conn,
        _inventory_snapshot: None,
    }
}

#[test]
fn each_pane_keeps_its_tier_when_a_conversation_is_replaced() {
    let store = store();
    for (index, tier) in [None, Some("priority"), Some("ultrafast")]
        .into_iter()
        .enumerate()
    {
        store
            .upsert_agent_slot(
                "instance",
                index as i64,
                "codex",
                &format!("thread-{index}"),
                "/project",
                &format!("%{index}"),
                &format!("identity-{index}"),
                1,
            )
            .unwrap();
        let slot = store
            .read_slots_for_instance("instance")
            .unwrap()
            .pop()
            .unwrap();
        store.save_runtime_settings(&slot, &settings(tier)).unwrap();
    }
    store
        .conn
        .execute(
            "UPDATE agent_slot SET native_session_id = '', \
        xats_runtime_generation = xats_runtime_generation + 1",
            [],
        )
        .unwrap();
    for (slot, tier) in store
        .read_slots_for_instance("instance")
        .unwrap()
        .iter()
        .zip([None, Some("priority"), Some("ultrafast")])
    {
        assert_eq!(store.runtime_settings(slot).unwrap(), Some(settings(tier)));
    }
}

#[test]
fn a_replaced_identity_does_not_inherit_another_agents_settings() {
    let store = store();
    store
        .upsert_agent_slot("instance", 0, "codex", "thread", "/project", "%1", "old", 1)
        .unwrap();
    let slot = store
        .read_slots_for_instance("instance")
        .unwrap()
        .pop()
        .unwrap();
    store
        .save_runtime_settings(&slot, &settings(Some("ultrafast")))
        .unwrap();
    store
        .conn
        .execute("UPDATE agent_slot SET xats_identity_key = 'new'", [])
        .unwrap();
    assert!(store.save_runtime_settings(&slot, &settings(None)).is_err());
    let replaced = store
        .read_slots_for_instance("instance")
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(store.runtime_settings(&replaced).unwrap(), None);
}

#[test]
fn off_is_an_explicit_override_and_wrong_resume_settings_are_rejected() {
    let selected = settings(None);
    assert_eq!(selected.codex_config()["service_tier"], "default");
    let mut response = serde_json::json!({"model": selected.model,
        "reasoningEffort": "xhigh", "serviceTier": null});
    selected.verify_codex_response(&response).unwrap();
    response["serviceTier"] = serde_json::json!("ultrafast");
    assert!(selected.verify_codex_response(&response).is_err());
    response["serviceTier"] = serde_json::json!("default");
    response["reasoningEffort"] = serde_json::json!("low");
    assert!(selected.verify_codex_response(&response).is_err());
    response["reasoningEffort"] = serde_json::json!("xhigh");
    for tier in [
        serde_json::json!(false),
        serde_json::json!(1),
        serde_json::json!({}),
    ] {
        response["serviceTier"] = tier;
        assert!(selected.verify_codex_response(&response).is_err());
    }
}

#[test]
fn invalid_external_settings_cannot_reach_the_shell() {
    for field in ["model", "effort", "service_tier"] {
        let mut value = serde_json::to_value(settings(None)).unwrap();
        value[field] = serde_json::json!("x; echo unsafe");
        let invalid: RuntimeSettings = serde_json::from_value(value).unwrap();
        assert!(invalid.append_command("codex".into(), "codex").is_err());
    }
}

#[test]
fn restoring_a_model_replaces_all_original_model_flag_forms() {
    for old in ["--model old", "--model=old", "-m old", "-mold"] {
        let args = RuntimeSettings::launch_args(&format!("{old} --search -c 'other=1'")).unwrap();
        let command = settings(Some("ultrafast"))
            .append_command(format!("codex {args}"), "codex")
            .unwrap();
        let args = shell_words::split(&command).unwrap();
        assert_eq!(args.iter().filter(|arg| *arg == "--model").count(), 1);
        assert!(!args.iter().any(|arg| arg == "old"));
        assert!(args.contains(&"--search".into()));
        assert!(args.contains(&"service_tier=\"ultrafast\"".into()));
    }
    let literal = RuntimeSettings::launch_args("-- --model literal").unwrap();
    assert_eq!(
        shell_words::split(&literal).unwrap(),
        ["--", "--model", "literal"]
    );
    let command = settings(None)
        .append_command(format!("codex {literal}"), "codex")
        .unwrap();
    let args = shell_words::split(&command).unwrap();
    let separator = args.iter().position(|arg| arg == "--").unwrap();
    assert_eq!(&args[separator + 1..], ["--model", "literal"]);
    assert_eq!(
        args[..separator]
            .iter()
            .filter(|arg| *arg == "--model")
            .count(),
        1
    );
}
