use crate::session::Instance;
use crate::tmux::inventory::Pane;
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};

type Slot = (i64, String, String, String);

pub(super) fn list(
    instance: &Instance,
    slots: Option<&[Slot]>,
    live: Option<&[Pane]>,
    observed_at: &str,
) -> Vec<Value> {
    let mut result = stored_panes(instance, slots, live, observed_at);
    for pane in live.unwrap_or_default() {
        if result
            .iter()
            .any(|entry| entry["tmux_pane"] == pane.target.pane_id)
        {
            continue;
        }
        let slot = slots
            .unwrap_or_default()
            .iter()
            .find(|slot| slot.3 == pane.target.pane_id);
        let mut entry = entry(
            None,
            slot.map(|s| s.0),
            Some(pane),
            false,
            slot.map(|s| s.1.as_str()),
            slot.map(|s| s.2.as_str()),
            observed_at,
            true,
        );
        entry["primary"] = json!(pane.primary);
        result.push(entry);
    }
    result
}

pub(super) async fn enrich(profiles: &mut [Value]) {
    let mut targets = Vec::new();
    for (profile_index, profile) in profiles.iter().enumerate() {
        let Some(sessions) = profile["sessions"].as_array() else {
            continue;
        };
        for (session_index, session) in sessions.iter().enumerate() {
            let Some(panes) = session["panes"].as_array() else {
                continue;
            };
            for (pane_index, pane) in panes.iter().enumerate() {
                if let Ok(target) = serde_json::from_value::<crate::xats_pane_lookup::PaneTarget>(
                    pane["tmux"].clone(),
                ) {
                    targets.push(((profile_index, session_index, pane_index), target));
                }
            }
        }
    }
    let mut lookups = stream::iter(targets.into_iter().map(|(index, target)| async move {
        (index, crate::xats_pane_lookup::lookup(&target).await)
    }))
    .buffer_unordered(8);
    while let Some(((p, s, i), lookup)) = lookups.next().await {
        let value = match lookup {
            Ok(mut value) => {
                value["source"] = json!("xats_pane_lookup");
                value["observed_at"] = value["checked_at"].clone();
                value
            }
            Err(error) => {
                let errors = profiles[p]["errors"]
                    .as_array_mut()
                    .expect("inventory errors");
                super::record_error(errors, "xats_pane_lookup", &error);
                profiles[p]["partial"] = json!(true);
                json!({"source":"xats_pane_lookup",
                    "observed_at":chrono::Utc::now().to_rfc3339(),
                    "identity":{"status":"unavailable","agent":null,
                        "reason":format!("{error:#}")},
                    "runtime":{"status":"unknown","reason":"Identity lookup unavailable"}})
            }
        };
        profiles[p]["sessions"][s]["panes"][i]["xats"] = value;
    }
}

fn stored_panes(
    instance: &Instance,
    slots: Option<&[Slot]>,
    live: Option<&[Pane]>,
    at: &str,
) -> Vec<Value> {
    let primary_slot = slots.unwrap_or_default().iter().find(|slot| slot.0 == 0);
    let primary = live.unwrap_or_default().iter().find(|pane| {
        pane.primary
            && primary_slot.map_or(true, |slot| slot.3 == pane.target.pane_id)
            && (!instance.managed_primary_marked
                || pane.marker.as_deref() == Some(instance.primary_pane_id()))
    });
    let mut main = entry(
        (instance.managed_owner.is_some()
            || instance.managed_primary_marked
            || instance.managed_primary_id.is_some())
        .then(|| instance.primary_pane_id()),
        Some(0),
        primary,
        instance.managed_owner.is_some(),
        Some(&instance.tool),
        Some(&instance.project_path),
        at,
        live.is_some(),
    );
    main["primary"] = json!(true);
    let mut result = vec![main];
    for child in &instance.managed_panes {
        let observed = live.unwrap_or_default().iter().find(|pane| {
            child.tmux_pane.as_deref() == Some(&pane.target.pane_id)
                && pane.marker.as_deref() == Some(&child.id)
        });
        result.push(entry(
            Some(&child.id),
            child.slot,
            observed,
            child.owner.is_some(),
            Some(&child.config.tool),
            Some(&child.config.working_dir),
            at,
            live.is_some(),
        ));
    }
    for slot in slots.unwrap_or_default() {
        if slot.0 == 0
            || instance
                .managed_panes
                .iter()
                .any(|child| child.slot == Some(slot.0))
        {
            continue;
        }
        if result.iter().any(|entry| entry["tmux_pane"] == slot.3) {
            continue;
        }
        let observed = live
            .unwrap_or_default()
            .iter()
            .find(|pane| pane.target.pane_id == slot.3);
        result.push(entry(
            None,
            Some(slot.0),
            observed,
            false,
            Some(&slot.1),
            Some(&slot.2),
            at,
            live.is_some(),
        ));
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn entry(
    id: Option<&str>,
    slot: Option<i64>,
    pane: Option<&Pane>,
    managed: bool,
    tool: Option<&str>,
    path: Option<&str>,
    at: &str,
    observed: bool,
) -> Value {
    json!({"pane_id":id,"slot":slot,"primary":false,"managed":managed,
        "tool":tool,"tool_source":"stored","path":path,
        "live":observed.then_some(pane.is_some()),"live_source":"tmux",
        "live_observed_at":at,"tmux_pane":pane.map(|p| &p.target.pane_id),
        "tmux":pane.map(|p| &p.target),"index":pane.map(|p|p.index),"left":pane.map(|p|p.left),"top":pane.map(|p|p.top),
        "xats":{"source":"none","observed_at":at,
            "identity":{"status":"unknown","agent":null,"reason":"No observed pane"},
            "runtime":{"status":"unknown","reason":"No observed pane"}}})
}
