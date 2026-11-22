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
