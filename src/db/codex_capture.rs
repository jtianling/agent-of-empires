#[cfg(any(test, not(any(target_os = "macos", target_os = "linux"))))]
use crate::process::commands_in_tree;
use std::collections::HashSet;
use std::path::Path;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::codex_xats::{self, BindingRequest};
use crate::db::reconcile::{AssignedPane, ObservedPane};
use crate::db::{AgentSlot, Store};
use crate::session::Instance;

#[derive(Debug, PartialEq, Eq)]
struct Launch {
    pane_pid: u32,
    tree: LaunchTree,
    cwd: String,
    session_name: String,
}

#[derive(Debug, PartialEq, Eq)]
struct LaunchTree {
    launch_id: String,
    codex_pids: Vec<u32>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    starts: Vec<(u32, u64)>,
}

pub fn maybe_claim_for_pane(
    store: &Store,
    inst: &Instance,
    pane: &AssignedPane,
    is_primary: bool,
    expected: Option<&AgentSlot>,
    observed: &ObservedPane<'_>,
    owns_window: impl Fn() -> Result<bool>,
) {
    if is_primary && (inst.tool != "codex" || inst.has_command_override()) {
        return;
    }
    if let Err(error) = claim_for_pane(
        store,
        inst,
        pane,
        is_primary,
        expected,
        observed,
        owns_window,
    ) {
        tracing::debug!("Codex binding for {} unavailable: {error:#}", pane.pane_id);
    }
}

fn claim_for_pane(
    store: &Store,
    inst: &Instance,
    pane: &AssignedPane,
    is_primary: bool,
    expected: Option<&AgentSlot>,
    observed: &ObservedPane<'_>,
    owns_window: impl Fn() -> Result<bool>,
) -> Result<()> {
    if observed.pane_id != pane.pane_id
        || observed.session_name != crate::tmux::Session::generate_name(&inst.id, &inst.title)
        || observed.root_pid <= 1
    {
        return Ok(());
    }
    let Some(before) = launch_at(observed, false)? else {
        return Ok(());
    };
    // A new or reassigned slot is itself a write based on the worker's inventory.
    if !expected.is_some_and(|slot| slot.agent == "codex" && slot.tmux_pane == pane.pane_id)
        && (read_launch(&pane.pane_id)?.as_ref() != Some(&before) || !owns_window()?)
    {
        return Ok(());
    }
    let Some(slot) = capture_slot(store, inst, pane, is_primary, expected, &before.cwd)? else {
        return Ok(());
    };
    if already_captured(store, &slot, &before.tree.launch_id)? {
        return Ok(());
    }
    let request = BindingRequest {
        protocol_version: 1,
        pane_id: pane.pane_id.clone(),
        launch_id: before.tree.launch_id.clone(),
    };
    let Some(thread_id) = codex_xats::lookup(request)? else {
        return Ok(());
    };
    if read_launch(&pane.pane_id)?.as_ref() != Some(&before) || !owns_window()? {
        return Ok(());
    }
    if !store.capture_codex_binding(&slot, &before.tree.launch_id, &thread_id)? {
        tracing::debug!("Codex binding for {} superseded by restart", pane.pane_id);
    }
    Ok(())
}

// A live launch can switch threads without changing PID or launch UUID.
const BINDING_RECHECK_SECS: i64 = 15;

fn already_captured(store: &Store, slot: &AgentSlot, launch: &str) -> Result<bool> {
    if slot.xats_identity_key.is_empty() || uuid::Uuid::try_parse(&slot.native_session_id).is_err()
    {
        return Ok(false);
    }
    let now = crate::db::now_unix();
    Ok(store.conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_slot s
         JOIN codex_binding b USING(instance_id, slot)
         JOIN pane_live p ON p.tmux_pane = s.tmux_pane
         WHERE s.instance_id = ?1 AND s.slot = ?2 AND s.tmux_pane = ?3
         AND s.agent = 'codex' AND s.cross_agent_team = 1
         AND s.xats_runtime_generation = ?4 AND b.generation = s.xats_runtime_generation
         AND s.xats_identity_key = ?5 AND b.identity_key = s.xats_identity_key
         AND s.native_session_id = ?6 AND b.thread_id = s.native_session_id
         AND b.state = 'verified' AND b.launch_id = ?7
         AND p.agent = s.agent AND p.native_session_id = b.thread_id AND p.cwd = s.cwd
         AND p.updated_at = s.last_seen_at AND p.updated_at > ?8 AND p.updated_at <= ?9)",
        rusqlite::params![
            slot.instance_id,
            slot.slot,
            slot.tmux_pane,
            slot.xats_runtime_generation,
            slot.xats_identity_key,
            slot.native_session_id,
            launch,
            now - BINDING_RECHECK_SECS,
            now
        ],
        |row| row.get(0),
    )?)
}

fn capture_slot(
    store: &Store,
    inst: &Instance,
    pane: &AssignedPane,
    primary: bool,
    expected: Option<&AgentSlot>,
    cwd: &str,
) -> Result<Option<AgentSlot>> {
    let Some(previous) = expected else {
        let mut config = inst.primary_pane_config().clone();
        config.tool = "codex".to_string();
        config.working_dir = cwd.to_string();
        let key = if primary {
            inst.xats_identity_key.as_deref()
        } else {
            None
        };
        store.record_launched_slot_config_if_absent(
            &inst.id,
            pane.slot,
            &config,
            &pane.pane_id,
            key.unwrap_or_default(),
            crate::db::now_unix(),
        )?;
        return Ok(store
            .read_slots_for_instance(&inst.id)?
            .into_iter()
            .find(|slot| {
                slot.slot == pane.slot
                    && slot.tmux_pane == pane.pane_id
                    && slot.agent == "codex"
                    && slot.xats_runtime_generation == 0
                    && slot.native_session_id.is_empty()
            }));
    };
    if previous.agent == "codex" && previous.tmux_pane == pane.pane_id {
        return Ok(Some(previous.clone()));
    }
    if !store.rebind_codex_slot(previous, &pane.pane_id, cwd)? {
        return Ok(None);
    }
    Ok(Some(AgentSlot {
        agent: "codex".to_string(),
        tmux_pane: pane.pane_id.clone(),
        cwd: cwd.to_string(),
        native_session_id: String::new(),
        xats_runtime_generation: previous.xats_runtime_generation + 1,
        ..previous.clone()
    }))
}

pub(crate) fn recover_explicit_binding(
    store: &Store,
    slot: &AgentSlot,
    session: &str,
    endpoint: &str,
    thread: &str,
) -> Result<()> {
    uuid::Uuid::try_parse(thread).context("Invalid original Codex thread UUID")?;
    anyhow::ensure!(
        slot.agent == "codex" && slot.cross_agent_team,
        "Recovery requires a cross-agent Codex slot"
    );
    anyhow::ensure!(
        slot.native_session_id.is_empty() || slot.native_session_id == thread,
        "Slot already records a different conversation"
    );
    let Some(before) = read_launch_checked(&slot.tmux_pane, true)? else {
        return recover_offline_binding(store, slot, session, endpoint, thread);
    };
    anyhow::ensure!(
        before.session_name == session,
        "Codex pane belongs to another session"
    );
    verify_selected_thread(&slot.tmux_pane, &before, thread)?;
    crate::codex_permissions::verify_recovery_thread(endpoint, thread, &before.cwd)?;
    verify_selected_thread(&slot.tmux_pane, &before, thread)?;
    anyhow::ensure!(
        read_launch(&slot.tmux_pane)?.as_ref() == Some(&before),
        "Codex launch changed during recovery"
    );
    anyhow::ensure!(
        store.recover_codex_binding(slot, &before.tree.launch_id, thread)?,
        "Codex slot changed during recovery; no binding was changed"
    );
    tracing::info!(
        instance_id = slot.instance_id,
        slot = slot.slot,
        thread_id = thread,
        "Recovered explicitly selected Codex thread binding"
    );
    Ok(())
}

fn recover_offline_binding(
    store: &Store,
    slot: &AgentSlot,
    session: &str,
    endpoint: &str,
    thread: &str,
) -> Result<()> {
    verify_offline_location(&slot.tmux_pane, session)?;
    crate::codex_permissions::verify_recovery_thread(endpoint, thread, &slot.cwd)?;
    anyhow::ensure!(
        read_launch_checked(&slot.tmux_pane, true)?.is_none(),
        "Codex launched during recovery; no binding was changed"
    );
    verify_offline_location(&slot.tmux_pane, session)?;
    anyhow::ensure!(
        store.recover_offline_codex_binding(slot, thread)?,
        "Codex slot changed during recovery; no binding was changed"
    );
    tracing::info!(
        instance_id = slot.instance_id,
        slot = slot.slot,
        thread_id = thread,
        "Recovered explicitly selected offline Codex thread"
    );
    Ok(())
}

fn verify_offline_location(pane: &str, session: &str) -> Result<()> {
    if let Some(location) = crate::tmux::managed_pane::locate(pane)? {
        anyhow::ensure!(
            location.session == session && location.marker.is_none(),
            "Offline Codex pane moved or is controlled by another owner"
        );
    }
    Ok(())
}

fn verify_selected_thread(pane: &str, launch: &Launch, thread: &str) -> Result<()> {
    for pid in &launch.tree.codex_pids {
        let args = crate::process::arguments(*pid)?;
        anyhow::ensure!(
            launch_from_args(&args).as_deref() == Some(&launch.tree.launch_id),
            "Live Codex launch arguments changed during recovery"
        );
        if let Some(resuming) = resume_thread_from_args(&args) {
            anyhow::ensure!(
                resuming == thread,
                "Live Codex command resumes a different thread"
            );
        }
    }
    if let Some(bound) = codex_xats::lookup_for_resume(BindingRequest {
        protocol_version: 1,
        pane_id: pane.into(),
        launch_id: launch.tree.launch_id.clone(),
    })? {
        anyhow::ensure!(
            bound == thread,
            "Live Codex is bound to a different conversation"
        );
    }
    Ok(())
}

pub(crate) fn verify_live_resume(
    store: &Store,
    slot: &crate::db::AgentSlot,
    session: &str,
) -> Result<()> {
    let Some(before) = read_launch_checked(&slot.tmux_pane, true)? else {
        return Ok(());
    };
    if before.session_name != session {
        bail!("Codex resume pane belongs to another session");
    }
    let thread = codex_xats::lookup_for_resume(BindingRequest {
        protocol_version: 1,
        pane_id: slot.tmux_pane.clone(),
        launch_id: before.tree.launch_id.clone(),
    })?;
    match thread {
        Some(thread) if thread == slot.native_session_id => {}
        Some(_) => bail!("live Codex conversation changed; wait for its binding before resuming"),
        None => {
            if !store.codex_resume_launch_verified(
                slot,
                &before.tree.launch_id,
                resume_thread_in_processes(&before)?.as_deref(),
            )? {
                bail!("live Codex conversation is not yet verified; wait before resuming");
            }
            tracing::info!(
                "Resuming Codex pane {} from its persisted binding while xats binding is unavailable",
                slot.tmux_pane
            );
        }
    }
    if read_launch(&slot.tmux_pane)?.as_ref() != Some(&before) {
        bail!("live Codex conversation changed; wait for its binding before resuming");
    }
    Ok(())
}

pub(crate) fn current_launch_id(pane: &str) -> Result<Option<String>> {
    Ok(read_launch(pane)?.map(|launch| launch.tree.launch_id))
}

pub(crate) async fn current_launch_id_async(pane: &str) -> Result<Option<String>> {
    let timeout = std::time::Duration::from_secs(5);
    let sampled_before = crate::process::pane_sample_time()?;
    let mut tmux = crate::tmux::tmux_command();
    tmux.args(["display-message", "-p", "-t", pane, "#{pane_pid}"]);
    let output = crate::process::output_with_timeout(tmux, timeout).await?;
    if !output.status.success() {
        bail!(
            "Reading Codex name pane failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let Some(pane_pid) = parse_pane_pid(String::from_utf8(output.stdout)?.trim())? else {
        return Ok(None);
    };
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let read =
            tokio::task::spawn_blocking(move || read_launch_tree(pane_pid, false, sampled_before));
        let tree = tokio::time::timeout(timeout, read)
            .await
            .context("Timed out reading Codex process tree")?
            .context("Joining Codex process query")??;
        Ok(tree.map(|t| t.launch_id))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let mut ps = Command::new("ps");
        ps.args(["-Ao", "pid=,ppid=,command="]);
        let output = crate::process::output_with_timeout(ps, timeout).await?;
        if !output.status.success() {
            bail!(
                "Reading Codex name process tree failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        launch_in_tree(&commands_in_tree(
            &String::from_utf8(output.stdout)?,
            pane_pid,
        ))
    }
}

fn read_launch(pane: &str) -> Result<Option<Launch>> {
    read_launch_checked(pane, false)
}

fn read_launch_checked(pane: &str, require_bound: bool) -> Result<Option<Launch>> {
    let sampled_before = crate::process::pane_sample_time()?;
    let Some((root_pid, session_name, cwd)) = live_location(pane)? else {
        return Ok(None);
    };
    launch_at(
        &ObservedPane {
            pane_id: pane,
            root_pid,
            sampled_before,
            session_name: &session_name,
            cwd: &cwd,
        },
        require_bound,
    )
}

pub(super) fn live_location(pane: &str) -> Result<Option<(u32, String, String)>> {
    let output = crate::tmux::control::output(&[
        "display-message",
        "-p",
        "-t",
        pane,
        "#{pane_pid}\t#{session_name}\t#{pane_current_path}",
    ])
    .context("reading Codex pane process")?;
    if !output.status.success() {
        tracing::debug!(
            "Reading Codex pane location for {pane} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return Ok(None);
    }
    let text = String::from_utf8(output.stdout)?;
    let (pid, location) = text
        .trim_end_matches('\n')
        .split_once('\t')
        .context("invalid Codex pane process response")?;
    let (session_name, cwd) = location
        .split_once('\t')
        .context("invalid Codex pane location response")?;
    let Some(pane_pid) = parse_pane_pid(pid)? else {
        return Ok(None);
    };
    Ok(Some((pane_pid, session_name.to_owned(), cwd.to_owned())))
}

fn launch_at(observed: &ObservedPane<'_>, require_bound: bool) -> Result<Option<Launch>> {
    let tree = read_launch_tree(observed.root_pid, require_bound, observed.sampled_before)?;
    Ok(tree.map(|tree| Launch {
        pane_pid: observed.root_pid,
        tree,
        cwd: observed.cwd.to_string(),
        session_name: observed.session_name.to_string(),
    }))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read_launch_tree(
    pane_pid: u32,
    require_bound: bool,
    sampled_before: u64,
) -> Result<Option<LaunchTree>> {
    let processes = crate::process::pane_processes(pane_pid, sampled_before)?;
    native_launch_tree(pane_pid, require_bound, &processes)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn native_launch_tree(
    pane_pid: u32,
    require_bound: bool,
    processes: &[crate::process::PaneProcess],
) -> Result<Option<LaunchTree>> {
    let mut launches = HashSet::new();
    let mut codex_pids = Vec::new();
    let mut starts = Vec::new();
    for process in processes {
        let codex = codex_args_start(&process.args).is_some();
        if process.pid == pane_pid || codex {
            starts.push((process.pid, process.start));
        }
        if codex {
            codex_pids.push(process.pid);
            launches.extend(launch_from_args(&process.args));
        }
    }
    starts.sort_unstable();
    codex_pids.sort_unstable();
    let launch_id = unique_launch(launches)?;
    check_bound(require_bound, &launch_id, &codex_pids)?;
    Ok(launch_id.map(|launch_id| LaunchTree {
        launch_id,
        codex_pids,
        starts,
    }))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_launch_tree(
    pane_pid: u32,
    require_bound: bool,
    _sampled_before: u64,
) -> Result<Option<LaunchTree>> {
    let output = Command::new("ps")
        .args(["-Ao", "pid=,ppid=,command="])
        .output()
        .context("reading Codex process tree")?;
    anyhow::ensure!(
        output.status.success(),
        "Reading Codex process tree: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing = String::from_utf8(output.stdout)?;
    let commands = commands_in_tree(&listing, pane_pid);
    let launch_id = launch_in_tree(&commands)?;
    let mut codex_pids: Vec<_> = commands
        .into_iter()
        .filter(|(_, command)| is_codex_command(command))
        .map(|(pid, _)| pid)
        .collect();
    codex_pids.sort_unstable();
    check_bound(require_bound, &launch_id, &codex_pids)?;
    Ok(launch_id.map(|launch_id| LaunchTree {
        launch_id,
        codex_pids,
    }))
}

fn check_bound(required: bool, launch: &Option<String>, pids: &[u32]) -> Result<()> {
    if required && launch.is_none() && !pids.is_empty() {
        bail!("live Codex has no verified launch identity; cannot resume stored history");
    }
    Ok(())
}

fn parse_pane_pid(value: &str) -> Result<Option<u32>> {
    if value.is_empty() {
        return Ok(None);
    }
    let pid = value.parse::<u32>().context("Invalid Codex pane PID")?;
    anyhow::ensure!(
        pid != 1 && pid <= i32::MAX as u32,
        "Invalid Codex pane PID {pid}"
    );
    Ok((pid != 0).then_some(pid))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn is_codex_command(command: &str) -> bool {
    let mut words = command.split_whitespace();
    let name = words.next().and_then(|word| Path::new(word).file_name());
    name.is_some_and(|name| name == "codex")
        || (name.is_some_and(|name| name == "node")
            && words
                .next()
                .and_then(|word| Path::new(word).file_name())
                .is_some_and(|name| name == "codex" || name == "codex.js"))
}

fn resume_thread_in_processes(launch: &Launch) -> Result<Option<String>> {
    let mut threads = HashSet::new();
    for pid in &launch.tree.codex_pids {
        let args = crate::process::arguments(*pid)?;
        if launch_from_args(&args).as_deref() != Some(&launch.tree.launch_id) {
            bail!("live Codex launch arguments changed; cannot resume stored history");
        }
        let Some(thread) = resume_thread_from_args(&args) else {
            return Ok(None);
        };
        threads.insert(thread);
    }
    Ok((threads.len() == 1)
        .then(|| threads.into_iter().next())
        .flatten())
}

fn resume_thread_from_args(words: &[String]) -> Option<String> {
    let mut index = codex_args_start(words)?;
    while let Some(word) = words.get(index) {
        match word.as_str() {
            "-c" | "--config" | "-C" | "--cd" | "-p" | "--profile" | "--remote" => {
                index += 2;
            }
            "--yolo" | "--dangerously-bypass-approvals-and-sandbox" | "--approve-for-me" => {
                index += 1;
            }
            "resume" => break,
            _ => return None,
        }
    }
    let thread = words.get(index + 1)?;
    uuid::Uuid::try_parse(thread).ok()?;
    if words[index + 2..].iter().any(|word| {
        matches!(word.as_str(), "--last" | "--all" | "--fork") || word.starts_with("--fork=")
    }) {
        return None;
    }
    Some(thread.clone())
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "linux"))))]
fn launch_in_tree(commands: &[(u32, &str)]) -> Result<Option<String>> {
    let launches = commands
        .iter()
        .filter_map(|(_, command)| launch_from_command(command))
        .collect::<HashSet<_>>();
    unique_launch(launches)
}

fn unique_launch(launches: HashSet<String>) -> Result<Option<String>> {
    if launches.len() > 1 {
        bail!("multiple Codex launches in one pane");
    }
    Ok(launches.into_iter().next())
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "linux"))))]
fn launch_from_command(command: &str) -> Option<String> {
    let words = shell_words::split(command).ok()?;
    launch_from_args(&words)
}

fn codex_args_start(words: &[String]) -> Option<usize> {
    match Path::new(words.first()?).file_name()?.to_str()? {
        "codex" => Some(1),
        "node"
            if matches!(
                Path::new(words.get(1)?).file_name()?.to_str()?,
                "codex" | "codex.js"
            ) =>
        {
            Some(2)
        }
        _ => None,
    }
}

fn launch_from_args(words: &[String]) -> Option<String> {
    codex_args_start(words)?;
    let mut launches = words.windows(2).filter_map(|pair| {
        if !matches!(pair[0].as_str(), "-c" | "--config") {
            return None;
        }
        pair[1]
            .strip_prefix("xats.agent_id=")
            .map(|value| value.trim_matches('"'))
    });
    let first = launches.next()?;
    uuid::Uuid::try_parse(first).ok()?;
    launches.next().is_none().then(|| first.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    const LEFT: &str = "00000000-0000-4000-8000-000000000001";
    const RIGHT: &str = "00000000-0000-4000-8000-000000000002";

    #[test]
    fn missing_and_dead_panes_have_no_process() {
        for invalid in ["invalid", "1", "4294967295"] {
            assert!(parse_pane_pid(invalid).is_err());
        }
        assert_eq!(parse_pane_pid("").unwrap(), None);
        assert_eq!(parse_pane_pid("0").unwrap(), None);
        assert_eq!(parse_pane_pid("42").unwrap(), Some(42));
    }

    #[test]
    fn resume_thread_requires_an_exact_unambiguous_codex_resume() {
        let args = [
            "node",
            "/bin/codex",
            "--remote",
            "ws://localhost:8799",
            "resume",
            LEFT,
        ]
        .map(str::to_string);
        assert_eq!(resume_thread_from_args(&args).as_deref(), Some(LEFT));
        for command in [
            format!("codex resume {LEFT} --fork"),
            format!("codex resume {LEFT} --last"),
            "codex resume --last".to_string(),
            format!("codex -- resume {LEFT}"),
            "codex resume invalid-thread".to_string(),
        ] {
            let args = shell_words::split(&command).unwrap();
            assert!(resume_thread_from_args(&args).is_none(), "{command}");
        }
        for args in [
            vec!["codex".to_string(), format!("explain resume {LEFT}")],
            vec!["codex".to_string(), format!("resume {LEFT}")],
            ["codex", "-C", "resume", LEFT].map(str::to_string).to_vec(),
            ["codex", "exec", "resume", LEFT]
                .map(str::to_string)
                .to_vec(),
            ["shell", "resume", LEFT].map(str::to_string).to_vec(),
        ] {
            assert!(resume_thread_from_args(&args).is_none(), "{args:?}");
        }
    }

    #[test]
    fn reconcile_only_skips_an_exact_verified_capture() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::ensure_schema(&conn).unwrap();
        let store = Store {
            conn,
            _inventory_snapshot: None,
        };
        let config = crate::session::PaneConfig::new("codex", "/project", true, true);
        store
            .record_launched_slot_config_if_absent("i", 0, &config, "%1", "key", 1)
            .unwrap();
        let slot = || store.read_slots_for_instance("i").unwrap().remove(0);
        assert!(!already_captured(&store, &slot(), LEFT).unwrap());
        assert!(store.capture_codex_binding(&slot(), LEFT, RIGHT).unwrap());
        let verified = slot();
        assert!(already_captured(&store, &verified, LEFT).unwrap());
        assert!(!already_captured(&store, &verified, RIGHT).unwrap());
        for stale in [
            AgentSlot {
                tmux_pane: "%2".into(),
                ..verified.clone()
            },
            AgentSlot {
                xats_identity_key: "other".into(),
                ..verified.clone()
            },
            AgentSlot {
                xats_runtime_generation: verified.xats_runtime_generation + 1,
                ..verified.clone()
            },
            AgentSlot {
                native_session_id: LEFT.into(),
                ..verified.clone()
            },
        ] {
            assert!(!already_captured(&store, &stale, LEFT).unwrap());
        }
        for state in ["awaiting_launch", "restarting"] {
            store
                .conn
                .execute("UPDATE codex_binding SET state = ?1", [state])
                .unwrap();
            assert!(!already_captured(&store, &verified, LEFT).unwrap());
            assert!(!already_captured(&store, &verified, RIGHT).unwrap());
        }
        store
            .conn
            .execute("UPDATE codex_binding SET state = 'verified'", [])
            .unwrap();
        store
            .conn
            .execute("UPDATE pane_live SET updated_at = 1", [])
            .unwrap();
        store
            .conn
            .execute("UPDATE agent_slot SET last_seen_at = 1", [])
            .unwrap();
        assert!(!already_captured(&store, &verified, LEFT).unwrap());
        assert!(store.capture_codex_binding(&verified, LEFT, RIGHT).unwrap());
        store.delete_pane_live("%1").unwrap();
        assert!(!already_captured(&store, &verified, LEFT).unwrap());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn native_launch_rechecks_pid_incarnations_and_preserves_argv() {
        use crate::process::PaneProcess;
        let shell = PaneProcess {
            pid: 10,
            parent: 2,
            start: 12,
            args: vec![
                "sh".into(),
                "-c".into(),
                format!("codex -c xats.agent_id={RIGHT}"),
            ],
        };
        let codex = PaneProcess {
            pid: 11,
            parent: 10,
            start: 34,
            args: vec![
                "node".into(),
                "/with spaces/codex.js".into(),
                "-c".into(),
                format!("xats.agent_id={LEFT}"),
            ],
        };
        let mut invalid = codex.args.clone();
        invalid.extend(["-c".into(), format!("xats.agent_id={LEFT}�")]);
        assert_eq!(launch_from_args(&invalid), None);
        let mut tree = [shell, codex];
        let observe =
            |processes: &[PaneProcess]| native_launch_tree(10, true, processes).unwrap().unwrap();
        let before = observe(&tree);
        assert_eq!(before.launch_id, LEFT);
        assert_eq!(before.codex_pids, [11]);
        tree.reverse();
        assert_eq!(before, observe(&tree));
        tree[0].start += 1;
        assert_ne!(before, observe(&tree));
        tree[0].args.truncate(2);
        assert!(native_launch_tree(10, true, &tree).is_err());
    }

    #[test]
    fn wrappers_are_not_codex_and_multiple_launches_are_rejected() {
        assert!(
            launch_from_command(&format!("sh -c 'exec codex -c xats.agent_id={LEFT}'")).is_none()
        );
        let listing = format!(
            "10 1 codex -c xats.agent_id={LEFT}\n\
             11 10 codex -c xats.agent_id={RIGHT}"
        );
        assert!(launch_in_tree(&commands_in_tree(&listing, 10)).is_err());
        assert_eq!(
            launch_in_tree(&commands_in_tree("10 1 codex resume old-thread", 10)).unwrap(),
            None
        );
    }
}
