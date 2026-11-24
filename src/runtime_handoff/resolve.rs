use std::time::Duration;

use anyhow::{Context, Result};

use super::{
    client,
    pending::{self, Pending},
    types::{Response, Runtime, State},
};

pub(super) fn commit_with_recovery(
    profile: &str,
    pending: &mut Pending,
    runtime: Runtime,
) -> Result<Response> {
    let mut attempts = 0;
    loop {
        let storage = crate::session::Storage::new(profile)?;
        let lock = storage.operation_lock()?;
        let current = pending::load(profile, &pending.instance_id, pending.slot)?
            .context("Runtime handoff disappeared during commit")?;
        anyhow::ensure!(
            current.request_id == pending.request_id && current.attempt_id == pending.attempt_id,
            "Runtime handoff attempt changed"
        );
        *pending = current;
        let error = match super::commit(profile, pending, runtime.clone()) {
            Ok(response) => return Ok(response),
            Err(error) => error,
        };
        attempts += 1;
        tracing::warn!("Runtime handoff commit needs confirmation: {error:#}");
        if attempts == 1 {
            eprintln!("Runtime handoff is pending: {error:#}");
        }
        if let Some(response) = status(profile, pending)? {
            if response.state == State::Committed && points_to(&response, &runtime) {
                super::capture_native(profile, pending, &response)?;
                return Ok(response);
            }
            if response.state != State::Prepared {
                return Err(error);
            }
            let retryable = error
                .downcast_ref::<client::BusinessFailure>()
                .map_or(true, |failure| failure.retryable);
            if (attempts >= 3 || !retryable) && abort(profile, pending, &runtime)? {
                return Err(error);
            }
        }
        drop(lock);
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn status(profile: &str, pending: &mut Pending) -> Result<Option<Response>> {
    match client::call("status", super::control(pending)?, &pending.identity_key) {
        Ok(response) => {
            pending.validate_response(&response)?;
            pending.controlled_identity |= response.state == State::Committed;
            pending.response = Some(response.clone());
            pending::save(profile, pending)?;
            Ok(Some(response))
        }
        Err(error) => {
            tracing::warn!("Cannot confirm runtime handoff outcome; preserving runtime: {error:#}");
            Ok(None)
        }
    }
}

fn abort(profile: &str, pending: &mut Pending, _runtime: &Runtime) -> Result<bool> {
    match client::call("abort", super::control(pending)?, &pending.identity_key) {
        Ok(response) => {
            pending.validate_response(&response)?;
            if response.state == State::Aborted {
                pending.response = Some(response);
                pending.target_runtime = None;
                pending.attempt_id = None;
                pending::save(profile, pending)?;
                return Ok(true);
            }
            Ok(false)
        }
        Err(error) => {
            tracing::warn!("Cannot confirm runtime handoff abort; preserving runtime: {error:#}");
            Ok(false)
        }
    }
}

fn points_to(response: &Response, runtime: &Runtime) -> bool {
    response
        .current
        .as_ref()
        .and_then(|identity| identity.runtime.as_ref())
        .is_some_and(|current| super::same_runtime(current, runtime))
}
