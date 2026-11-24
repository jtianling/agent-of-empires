use anyhow::{bail, ensure, Context, Result};

use crate::session::Instance;
use crate::xats_control::{self, ControlPlane};
use crate::xats_identity::KeyHolder;

mod client;
mod guard;
mod targets;
pub(crate) use guard::ReleaseGuard;

pub(crate) fn warn_on_failure<T>(result: Result<T>, session: &str, operation: &str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            let message =
                format!("Session {session}: {operation} failed; continuing deletion: {error:#}");
            tracing::warn!("{message}");
            if let Err(log_error) = append_warning(&message) {
                tracing::error!("Cannot persist deletion warning: {log_error:#}");
            }
            None
        }
    }
}

fn append_warning(message: &str) -> Result<()> {
    use std::io::Write;
    let path = crate::session::get_app_dir()?.join("debug.log");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{} WARN {message}", chrono::Utc::now())?;
    Ok(())
}

pub(crate) fn release_session(profile: &str, instance: &Instance) -> Result<ReleaseGuard> {
    let profile = profile.to_string();
    let instance = instance.clone();
    let result = xats_control::block_on_control("xats-pane-release", move || {
        Box::pin(async move { release(&profile, &instance).await })
    });
    if let Err(error) = &result {
        tracing::error!("Cannot release session xats identities: {error:#}");
    }
    result
}

async fn release(profile: &str, instance: &Instance) -> Result<ReleaseGuard> {
    let slots = crate::db::Store::open_with_schema(profile)?
        .read_slots_for_instance_with_diagnostics(&instance.id)?;
    ensure!(
        slots.skipped == 0,
        "Cannot release xats with unreadable pane metadata"
    );
    let session = crate::tmux::Session::generate_name(&instance.id, &instance.title);
    let panes = crate::tmux::inventory::observe_session(profile, &session).await?;
    let plan = targets::plan(instance, &slots.slots, &panes);
    if plan.targets.is_empty() && plan.missing_keys.is_empty() {
        return Ok(ReleaseGuard::default());
    }
    let guard = if plan
        .targets
        .iter()
        .any(|target| target.allow_dead_if_identity_unclaimed)
    {
        ReleaseGuard::new(profile, &instance.id, &session, &panes, &slots.slots)
    } else {
        ReleaseGuard::default()
    };
    guard.verify_async().await?;
    let result = match validate_plan(&plan).and_then(|()| xats_control::discover_control_plane()) {
        Ok(control) => redact(release_plan(&control, &plan).await, &plan, &control),
        Err(error) if !plan.requires_daemon && missing_daemon(&error) => Ok(()),
        Err(error) => Err(error).context("Cannot contact xats before session deletion"),
    };
    warn_on_failure(result, &instance.id, "xats identity release");
    guard.verify_async().await?;
    Ok(guard)
}

fn redact(result: Result<()>, plan: &targets::Plan, control: &ControlPlane) -> Result<()> {
    let mut secrets = plan
        .targets
        .iter()
        .filter_map(|target| target.identity_key.as_deref())
        .chain(plan.missing_keys.iter().map(String::as_str))
        .chain(control.token.iter().map(|token| token.expose()))
        .collect::<Vec<_>>();
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    result.map_err(|error| {
        let detail = secrets.iter().fold(format!("{error:#}"), |detail, secret| {
            detail.replace(secret, "***")
        });
        anyhow::anyhow!(detail)
    })
}

fn validate_plan(plan: &targets::Plan) -> Result<()> {
    ensure!(
        plan.targets.len() <= targets::MAX_TARGETS,
        "Cannot release more than {} xats panes atomically",
        targets::MAX_TARGETS
    );
    for target in &plan.targets {
        target.tmux.validate()?;
        if let Some(key) = &target.identity_key {
            xats_control::validate_identity_key(key)?;
        }
        if let Some(session) = &target.opencode_session_id {
            crate::opencode_runtime::validate_session_id(session)?;
        }
    }
    for key in &plan.missing_keys {
        xats_control::validate_identity_key(key)?;
    }
    Ok(())
}

async fn release_plan(control: &ControlPlane, plan: &targets::Plan) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(xats_control::CONTROL_TIMEOUT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("Building xats pane release client")?;
    for key in &plan.missing_keys {
        match crate::xats_identity::lookup_with(control, &client, key).await {
            KeyHolder::NotFound => {}
            KeyHolder::Held { .. } => bail!(
                "A stored xats identity still has a holder but its pane is missing; \
                 restore the pane or unregister that member before deleting the session"
            ),
            KeyHolder::Unsupported => bail!("Xats identity lookup is unsupported (HTTP 404)"),
            KeyHolder::Unavailable(error) => bail!("Xats identity lookup failed: {error}"),
        }
    }
    if !plan.targets.is_empty() {
        client::release(control, &client, &plan.targets).await?;
    }
    Ok(())
}

fn missing_daemon(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

#[cfg(test)]
mod tests;
