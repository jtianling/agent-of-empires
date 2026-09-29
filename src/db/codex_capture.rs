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

pub(crate) fn verify_live_resume(slot: &crate::db::AgentSlot, session: &str) -> Result<()> {
    let Some(before) = read_launch_checked(&slot.tmux_pane, true)? else {
        return Ok(());
    };
    if before.session_name != session {
        bail!("Codex resume pane belongs to another session");
    }
    let thread = resolve_thread(&slot.tmux_pane, &before)?
        .context("live Codex conversation is not yet verified; wait before resuming")?;
    if thread != slot.native_session_id || read_launch(&slot.tmux_pane)?.as_ref() != Some(&before) {
        bail!("live Codex conversation changed; wait for its binding before resuming");
    }
    Ok(())
}

pub(crate) fn current_launch_id(pane: &str) -> Result<Option<String>> {
    Ok(read_launch(pane)?.map(|launch| launch.launch_id))
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
    Ok(launch_id.map(|launch_id| Launch {
        pane_pid,
        launch_id,
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
    let binary = Path::new(words.first()?).file_name()?.to_str()?;
    let codex = binary == "codex"
        || (binary == "node"
            && matches!(
                Path::new(words.get(1)?).file_name()?.to_str()?,
                "codex" | "codex.js"
            ));
    if !codex {
        return None;
    }
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
