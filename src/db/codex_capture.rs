use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::codex_xats::{self, BindingRequest};
use crate::db::{reconcile::AssignedPane, AgentSlot, Store};
use crate::session::Instance;

#[derive(Debug, PartialEq, Eq)]
struct Launch {
    pane_pid: u32,
    launch_id: String,
    codex_pids: Vec<u32>,
    cwd: String,
    session_name: String,
}

pub fn maybe_claim_for_pane(
    store: &Store,
    inst: &Instance,
    pane: &AssignedPane,
    is_primary: bool,
    expected: Option<&AgentSlot>,
) {
    if is_primary && (inst.tool != "codex" || inst.has_command_override()) {
        return;
    }
    if let Err(error) = claim_for_pane(store, inst, pane, is_primary, expected) {
        tracing::debug!("Codex binding for {} unavailable: {error:#}", pane.pane_id);
    }
}

fn claim_for_pane(
    store: &Store,
    inst: &Instance,
    pane: &AssignedPane,
    is_primary: bool,
    expected: Option<&AgentSlot>,
) -> Result<()> {
    let Some(before) = read_launch(&pane.pane_id)? else {
        return Ok(());
    };
    if before.session_name != crate::tmux::Session::generate_name(&inst.id, &inst.title) {
        return Ok(());
    }
    let Some(slot) = capture_slot(store, inst, pane, is_primary, expected, &before.cwd)? else {
        return Ok(());
    };
    let Some(thread_id) = resolve_thread(&pane.pane_id, &before)? else {
        return Ok(());
    };
    if read_launch(&pane.pane_id)?.as_ref() != Some(&before) {
        return Ok(());
    }
    if !store.capture_codex_binding(&slot, &before.launch_id, &thread_id)? {
        tracing::debug!("Codex binding for {} superseded by restart", pane.pane_id);
    }
    Ok(())
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

fn resolve_thread(pane: &str, launch: &Launch) -> Result<Option<String>> {
    codex_xats::lookup(BindingRequest {
        protocol_version: 1,
        pane_id: pane.to_string(),
        launch_id: launch.launch_id.clone(),
    })
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
    let before = read_launch_checked(&slot.tmux_pane, true)?
        .context("Codex launch is unavailable; no binding was changed")?;
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
        store.recover_codex_binding(slot, &before.launch_id, thread)?,
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

fn verify_selected_thread(pane: &str, launch: &Launch, thread: &str) -> Result<()> {
    for pid in &launch.codex_pids {
        let args = crate::process::arguments(*pid)?;
        anyhow::ensure!(
            launch_from_args(&args).as_deref() == Some(&launch.launch_id),
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
        launch_id: launch.launch_id.clone(),
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
        launch_id: before.launch_id.clone(),
    })?;
    match thread {
        Some(thread) if thread == slot.native_session_id => {}
        Some(_) => bail!("live Codex conversation changed; wait for its binding before resuming"),
        None => {
            if !store.codex_resume_launch_verified(
                slot,
                &before.launch_id,
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
    Ok(read_launch(pane)?.map(|launch| launch.launch_id))
}

pub(crate) async fn current_launch_id_async(pane: &str) -> Result<Option<String>> {
    let timeout = std::time::Duration::from_secs(5);
    let mut tmux = crate::tmux::tmux_command();
    tmux.args(["display-message", "-p", "-t", pane, "#{pane_pid}"]);
    let output = crate::process::output_with_timeout(tmux, timeout).await?;
    if !output.status.success() {
        bail!(
            "Reading Codex name pane failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let pane_pid = String::from_utf8(output.stdout)?
        .trim()
        .parse::<u32>()
        .context("Invalid Codex name pane PID")?;
    let mut ps = Command::new("ps");
    ps.args(["-Ao", "pid=,ppid=,command="]);
    let output = crate::process::output_with_timeout(ps, timeout).await?;
    if !output.status.success() {
        bail!(
            "Reading Codex name process tree failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    launch_in_tree(&String::from_utf8(output.stdout)?, pane_pid)
}

fn read_launch(pane: &str) -> Result<Option<Launch>> {
    read_launch_checked(pane, false)
}

fn read_launch_checked(pane: &str, require_bound: bool) -> Result<Option<Launch>> {
    let output = crate::tmux::tmux_command()
        .args([
            "display-message",
            "-p",
            "-t",
            pane,
            "#{pane_pid}\t#{session_name}\t#{pane_current_path}",
        ])
        .output()
        .context("reading Codex pane process")?;
    if !output.status.success() {
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
    let pane_pid = pid.parse::<u32>().context("invalid Codex pane pid")?;
    let output = Command::new("ps")
        .args(["-Ao", "pid=,ppid=,command="])
        .output()
        .context("reading Codex process tree")?;
    if !output.status.success() {
        bail!("cannot read Codex process tree");
    }
    let listing = String::from_utf8(output.stdout)?;
    let launch_id = launch_in_tree(&listing, pane_pid)?;
    if require_bound
        && launch_id.is_none()
        && commands_in_tree(&listing, pane_pid)
            .iter()
            .any(|(_, command)| is_codex_command(command))
    {
        bail!("live Codex has no verified launch identity; cannot resume stored history");
    }
    let mut codex_pids: Vec<_> = commands_in_tree(&listing, pane_pid)
        .into_iter()
        .filter(|(_, command)| is_codex_command(command))
        .map(|(pid, _)| pid)
        .collect();
    codex_pids.sort_unstable();
    Ok(launch_id.map(|launch_id| Launch {
        pane_pid,
        launch_id,
        codex_pids,
        cwd: cwd.to_string(),
        session_name: session_name.to_string(),
    }))
}

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
    for pid in &launch.codex_pids {
        let args = crate::process::arguments(*pid)?;
        if launch_from_args(&args).as_deref() != Some(&launch.launch_id) {
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

fn launch_in_tree(listing: &str, root: u32) -> Result<Option<String>> {
    let launches = commands_in_tree(listing, root)
        .into_iter()
        .filter_map(|(_, command)| launch_from_command(command))
        .collect::<HashSet<_>>();
    if launches.len() > 1 {
        bail!("multiple Codex launches in one pane");
    }
    Ok(launches.into_iter().next())
}

fn commands_in_tree(listing: &str, root: u32) -> Vec<(u32, &str)> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut commands = HashMap::new();
    for line in listing.lines() {
        let mut fields = line.trim().splitn(2, char::is_whitespace);
        let Some(pid) = fields.next().and_then(|v| v.parse::<u32>().ok()) else {
            continue;
        };
        let mut fields = fields
            .next()
            .unwrap_or_default()
            .trim_start()
            .splitn(2, char::is_whitespace);
        let Some(parent) = fields.next().and_then(|v| v.parse::<u32>().ok()) else {
            continue;
        };
        children.entry(parent).or_default().push(pid);
        commands.insert(pid, fields.next().unwrap_or_default().trim_start());
    }
    let mut queue = vec![root];
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    while let Some(pid) = queue.pop() {
        if !seen.insert(pid) {
            continue;
        }
        if let Some(command) = commands.get(&pid) {
            found.push((pid, *command));
        }
        queue.extend(children.get(&pid).into_iter().flatten());
    }
    found
}

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
        let value = pair[1].strip_prefix("xats.agent_id=")?.trim_matches('"');
        uuid::Uuid::try_parse(value).ok().map(|_| value.to_string())
    });
    let first = launches.next()?;
    launches.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    const LEFT: &str = "00000000-0000-4000-8000-000000000001";
    const RIGHT: &str = "00000000-0000-4000-8000-000000000002";

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
    fn siblings_in_same_directory_do_not_share_launches() {
        let listing = format!(
            "10 1 zsh\n11 10 codex -C /same -c xats.agent_id=\"{LEFT}\"\n\
             20 1 zsh\n21 20 codex -C /same -c xats.agent_id=\"{RIGHT}\""
        );
        assert_eq!(launch_in_tree(&listing, 10).unwrap().as_deref(), Some(LEFT));
        assert_eq!(
            launch_in_tree(&listing, 20).unwrap().as_deref(),
            Some(RIGHT)
        );
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
        assert!(launch_in_tree(&listing, 10).is_err());
        assert_eq!(
            launch_in_tree("10 1 codex resume old-thread", 10).unwrap(),
            None
        );
    }
}
