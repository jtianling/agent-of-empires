use std::collections::{HashMap, HashSet};

use anyhow::{ensure, Context, Result};

mod wire;

pub struct ObservedPane<'a> {
    pub pane_id: &'a str,
    pub session_name: &'a str,
    pub root_pid: u32,
    pub cwd: &'a str,
    pub sampled_before: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Server {
    pid: u32,
    pub started: i64,
}

#[derive(PartialEq, Eq)]
pub(super) struct Window {
    server: Server,
    session_id: u32,
    window_id: u32,
    pub panes: Vec<(u32, String)>,
    pub primary: Option<String>,
    pub layout: String,
    processes: HashMap<String, (u32, Option<String>)>,
}

impl Window {
    pub fn observe<'a>(
        &'a self,
        session: &'a str,
        pane: &'a str,
        sampled_before: u64,
    ) -> Option<ObservedPane<'a>> {
        let (pid, cwd) = self.processes.get(pane)?;
        Some(ObservedPane {
            pane_id: pane,
            session_name: session,
            root_pid: *pid,
            cwd: cwd.as_deref()?,
            sampled_before,
        })
    }

    pub fn server_started(&self) -> i64 {
        self.server.started
    }

    pub fn still_owned(&self, profile: &str, name: &str) -> Result<bool> {
        let current = read(profile, Some(name))?;
        Ok(current.windows.get(name) == Some(self))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PaneGeneration {
    server: Server,
    session_id: u32,
    window_id: u32,
    root_pid: u32,
    root_start: Option<String>,
}

impl PaneGeneration {
    pub(crate) fn respawn_condition(&self, primary: bool) -> String {
        format!(
            "#{{==:#{{pid}}|#{{start_time}}|#{{session_id}}|#{{window_id}}|#{{pane_pid}}|\
             #{{==:#{{@aoe_agent_pane}},#{{pane_id}}}},{0}|{1}|${2}|@{3}|{4}|{5}}}",
            self.server.pid,
            self.server.started,
            self.session_id,
            self.window_id,
            self.root_pid,
            u8::from(primary)
        )
    }

    pub(crate) fn observe(profile: &str, name: &str, pane: &str, primary: bool) -> Result<Self> {
        let snapshot = read(profile, Some(name))?;
        let window = snapshot
            .windows
            .get(name)
            .context("Target tmux session disappeared")?;
        ensure!(
            (window.primary.as_deref() == Some(pane)) == primary,
            "Target pane primary assignment changed"
        );
        let (pid, _) = window
            .processes
            .get(pane)
            .context("Pane no longer belongs to target window")?;
        let root_start = if *pid == 0 {
            None
        } else {
            Some(crate::process::start_identity(*pid)?.context("Pane root process disappeared")?)
        };
        Ok(Self {
            server: window.server,
            session_id: window.session_id,
            window_id: window.window_id,
            root_pid: *pid,
            root_start,
        })
    }

    pub(crate) fn has_live_root(&self) -> bool {
        !crate::process::is_unsafe_kill_root(self.root_pid) && self.root_start.is_some()
    }

    /// Terminate the previously observed pane process tree after `respawn-pane -k`
    /// has atomically replaced the pane root. Verifies `start_identity` so a
    /// recycled PID is never signalled, and waits up to `grace` for a clean exit.
    pub(crate) fn terminate_previous_process_tree(&self, grace: std::time::Duration) {
        if crate::process::is_unsafe_kill_root(self.root_pid) {
            return;
        }
        let Some(expected) = self.root_start.as_deref() else {
            return;
        };
        if let Ok(Some(actual)) = crate::process::start_identity(self.root_pid) {
            if actual == expected {
                crate::process::kill_process_tree_gracefully(self.root_pid, grace);
            }
        }
    }
}

pub(super) struct Snapshot {
    pub sampled_before: u64,
    pub windows: HashMap<String, Window>,
    pub live: Option<HashSet<String>>,
    pub server: Option<Server>,
}

pub(super) fn read(profile: &str, session: Option<&str>) -> Result<Snapshot> {
    let target;
    let args: &[&str] = if let Some(name) = session {
        target = format!("={name}:");
        &["list-panes", "-t", &target, "-F", wire::FORMAT]
    } else {
        &["list-panes", "-a", "-F", wire::FORMAT]
    };
    let sampled_before = crate::process::pane_sample_time()?;
    let output = crate::tmux::control::profile_output(profile, args)
        .context("Reading tmux pane inventory")?;
    ensure!(
        output.status.success(),
        "tmux list-panes failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(parse(&output.stdout, sampled_before))
}

struct Group<'a> {
    first: wire::Row<'a>,
    rows: Vec<wire::Row<'a>>,
}

impl<'a> Group<'a> {
    fn finish(self) -> Result<Option<Window>> {
        let first = self.first;
        ensure!(
            self.rows.len() == first.pane_count,
            "Incomplete tmux window"
        );
        let mut window = Window {
            server: first.server,
            session_id: first.sid,
            window_id: first.wid,
            panes: Vec::new(),
            primary: None,
            layout: first.layout.to_owned(),
            processes: HashMap::new(),
        };
        for row in self.rows {
            ensure!(row.coherent_with(&first), "Inconsistent tmux window");
            ensure!(
                window
                    .processes
                    .insert(row.id.into(), (row.pid, row.cwd.decode()))
                    .is_none(),
                "Duplicate pane in tmux window"
            );
            window.panes.push((row.index, row.id.into()));
            if row.primary {
                ensure!(
                    window.primary.replace(row.id.into()).is_none(),
                    "Multiple primary panes"
                );
            }
        }
        if !first.active {
            return Ok(None);
        }
        window.panes.sort_by_key(|(index, _)| *index);
        ensure!(
            window.panes.windows(2).all(|p| p[0].0 != p[1].0),
            "Duplicate pane index"
        );
        let ids = crate::tmux::layout::pane_ids(&window.layout)?;
        ensure!(
            ids.len() == window.panes.len()
                && ids.iter().all(|id| window.processes.contains_key(id)),
            "Pane inventory differs from window layout"
        );
        Ok(Some(window))
    }
}

fn parse(bytes: &[u8], sampled_before: u64) -> Snapshot {
    let mut groups = HashMap::new();
    let mut live = HashSet::new();
    let mut complete = bytes.ends_with(b"\n");
    let mut totals = None;
    let mut counts = HashMap::new();
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        let Some(line) = line.strip_suffix(b"\n") else {
            break;
        };
        match wire::row(line) {
            Ok(row) => {
                let total = (row.server, row.session_count);
                complete &= *totals.get_or_insert(total) == total;
                let (expected, windows) = counts
                    .entry(row.sid)
                    .or_insert_with(|| (row.window_count, HashSet::new()));
                complete &= *expected == row.window_count;
                windows.insert(row.wid);
                live.insert(row.id.to_owned());
                groups
                    .entry((row.sid, row.wid))
                    .or_insert_with(|| Group {
                        first: row,
                        rows: Vec::new(),
                    })
                    .rows
                    .push(row);
            }
            Err(e) => {
                tracing::debug!("Invalid tmux inventory row: {e:#}");
                complete = false;
            }
        }
    }
    complete &= totals.is_some_and(|(_, n)| n == counts.len())
        && counts.values().all(|(n, windows)| *n == windows.len());
    let mut windows = HashMap::new();
    let mut ambiguous = HashSet::new();
    for group in groups.into_values() {
        let name = group.first.name.decode();
        match group.finish() {
            Ok(Some(window)) => {
                if let Some(name) = name {
                    if windows.insert(name.to_owned(), window).is_some() {
                        ambiguous.insert(name.to_owned());
                        complete = false;
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::debug!("Skipping inconsistent tmux window {name:?}: {e:#}");
                complete = false;
            }
        }
    }
    windows.retain(|name, _| !ambiguous.contains(name));
    Snapshot {
        sampled_before,
        windows,
        live: complete.then_some(live),
        server: totals.map(|(server, _)| server),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antigravity_rejects_reused_server_window_or_process_at_respawn() {
        let observed = || PaneGeneration {
            server: Server {
                pid: 10,
                started: 20,
            },
            session_id: 1,
            window_id: 2,
            root_pid: 30,
            root_start: Some("birth".into()),
        };
        let target = observed();
        assert_eq!(target, observed());
        assert_eq!(
            target.respawn_condition(true),
            "#{==:#{pid}|#{start_time}|#{session_id}|#{window_id}|#{pane_pid}|\
             #{==:#{@aoe_agent_pane},#{pane_id}},10|20|$1|@2|30|1}"
        );
        let mut current = observed();
        current.root_pid = 0;
        current.root_start = None;
        assert_ne!(target, current);
        let mut current = observed();
        current.server.started += 1;
        assert_ne!(target, current);
        let mut current = observed();
        current.window_id += 1;
        assert_ne!(target, current);
        let mut current = observed();
        current.root_start = Some("reused".into());
        assert_ne!(target, current);
    }

    #[test]
    fn only_the_active_window_assigns_slots_but_both_protect_captures() {
        let data = concat!(
            "12 101 1 $1 2 @4 1 1 0000,80x24,0,0,7 1 0 %7 55 3 8|家|/a%0A%7C%25家\n",
            "12 101 1 $1 2 @5 0 1 0000,80x24,0,0,8 0 0 %8 56 3 2|家|/\\377\n",
        );
        let snapshot = parse(data.as_bytes(), 123);
        let window = &snapshot.windows["家"];
        let pane = window.observe("家", "%7", snapshot.sampled_before).unwrap();
        assert_eq!(pane.cwd, "/a\n|%家");
        assert_eq!(pane.sampled_before, 123);
        assert_eq!(window.panes, [(0, "%7".into())]);
        assert_eq!(snapshot.live.unwrap(), ["%7".into(), "%8".into()].into());
    }

    #[test]
    fn empty_observer_protects_gc_without_a_live_process() {
        let data = concat!(
            "12 101 2 $1 1 @4 1 1 0000,80x24,0,0,7 1 0 %7 55 1 2|A|/a\n",
            "12 101 2 $2 1 @5 1 1 0000,80x24,0,0,8 0 0 %8 0 8 0|observer|\n",
        );
        let snapshot = parse(data.as_bytes(), 123);
        assert_eq!(snapshot.windows["observer"].panes, [(0, "%8".into())]);
        assert_eq!(
            snapshot.windows["A"]
                .observe("A", "%7", 123)
                .unwrap()
                .root_pid,
            55
        );
        assert_eq!(snapshot.live.unwrap(), ["%7".into(), "%8".into()].into());
    }

    #[test]
    fn unknown_cwd_is_local_and_missing_windows_inhibit_gc() {
        let data = concat!(
            "12 101 2 $1 1 @4 1 1 0000,80x24,0,0,7 1 0 %7 55 1 2|A|/\\377\n",
            "12 101 2 $2 2 @5 1 1 0000,80x24,0,0,8 0 0 %8 56 1 5|B|/\\377\n",
        );
        let snapshot = parse(data.as_bytes(), 123);
        assert!(snapshot.windows["A"].observe("A", "%7", 123).is_none());
        assert_eq!(
            snapshot.windows["B"].observe("B", "%8", 123).unwrap().cwd,
            "/\\377"
        );
        assert!(snapshot.live.is_none());
    }
}
