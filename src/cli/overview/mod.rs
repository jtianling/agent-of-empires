mod panes;

use crate::session::{tui_instances, Storage};
use anyhow::Result;
use clap::Args;
use serde_json::{json, Value};

#[derive(Args)]
pub struct OverviewArgs {
    /// Inspect every existing profile instead of the selected profile
    #[arg(long)]
    pub all_profiles: bool,
}

pub async fn run(profile: String, args: OverviewArgs) -> Result<()> {
    report(collect(profile, args.all_profiles).await)
}

pub fn report(result: Result<Value>) -> Result<()> {
    match result {
        Ok(data) => {
            println!("{}", json!({"ok":true,"data":data}));
            Ok(())
        }
        Err(error) => {
            let message = format!("{error:#}");
            tracing::error!("Read-only inventory failed: {message}");
            println!(
                "{}",
                json!({"ok":false,"error":{
                "code":"inventory_failed","message":message}})
            );
            Err(error)
        }
    }
}

async fn collect(profile: String, all: bool) -> Result<Value> {
    let names = if all {
        tui_instances::profiles()?
    } else {
        tui_instances::validate_profile(&profile)?;
        vec![profile]
    };
    let mut profiles = Vec::new();
    for name in names {
        profiles.push(collect_profile(&name).await?);
    }
    panes::enrich(&mut profiles).await;
    Ok(json!({"observed_at":chrono::Utc::now().to_rfc3339(),
        "partial":profiles.iter().any(|p| p["partial"] == true),
        "profiles":profiles}))
}

async fn collect_profile(profile: &str) -> Result<Value> {
    let observed_at = chrono::Utc::now().to_rfc3339();
    let storage = Storage::read_only(profile)?;
    let mut errors = Vec::new();
    let instances = read(storage.read_only_instances(), "sessions", &mut errors);
    let groups = read(storage.read_only_groups(), "groups", &mut errors);
    let store = read(
        crate::db::Store::open_read_only(profile),
        "slots",
        &mut errors,
    )
    .flatten();
    let live = read(
        crate::tmux::inventory::observe(profile).await,
        "tmux",
        &mut errors,
    );
    let mut sessions = Vec::new();
    if let Some(instances) = instances.as_ref() {
        for instance in instances {
            let slots = match store.as_ref() {
                Some(store) => read(store.inventory_slots(&instance.id), "slots", &mut errors),
                None => None,
            };
            let name = crate::tmux::Session::generate_name(&instance.id, &instance.title);
            let observed = live.as_ref().map(|panes| {
                panes
                    .iter()
                    .filter(|pane| pane.session == name)
                    .cloned()
                    .collect::<Vec<_>>()
            });
            let pane_list = panes::list(
                instance,
                slots.as_deref(),
                observed.as_deref(),
                &observed_at,
            );
            sessions.push(json!({"id":instance.id,"title":instance.title,
                "path":instance.project_path,"group":instance.group_path,
                "tool":instance.tool,"managed":instance.managed_owner.is_some(),
                "status":format!("{:?}",instance.status).to_lowercase(),
                "status_source":"stored","status_observed_at":observed_at,
                "status_updated_at":null,"live":observed.as_ref().map(|p|!p.is_empty()),
                "live_source":"tmux","live_observed_at":observed_at,"panes":pane_list}));
        }
    }
    let groups = groups.map(|groups| {
        groups
            .into_iter()
            .map(|group| {
                json!({"name":group.name,"path":group.path,
            "default_directory":group.default_directory})
            })
            .collect::<Vec<_>>()
    });
    Ok(
        json!({"profile":profile,"observed_at":observed_at,"source":"stored+tmux+xats",
        "partial":!errors.is_empty(),"groups":groups,
        "sessions":instances.map(|_|sessions),"errors":errors}),
    )
}

fn read<T>(result: Result<T>, source: &str, errors: &mut Vec<Value>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            record_error(errors, source, &error);
            None
        }
    }
}

fn record_error(errors: &mut Vec<Value>, source: &str, error: &anyhow::Error) {
    let message = format!("{error:#}");
    tracing::warn!(source, "Read-only inventory observation failed: {message}");
    if let Some(existing) = errors
        .iter_mut()
        .find(|entry| entry["source"] == source && entry["message"] == message)
    {
        existing["affected_count"] = json!(existing["affected_count"].as_u64().unwrap_or(1) + 1);
        return;
    }
    errors.push(json!({"source":source,"message":message,
        "affected_count":1,"observed_at":chrono::Utc::now().to_rfc3339()}));
}
