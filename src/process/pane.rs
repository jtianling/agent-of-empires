use anyhow::{ensure, Context};

pub(crate) type Start = u64;

pub(crate) struct PaneProcess {
    pub pid: u32,
    pub parent: u32,
    pub start: Start,
    pub args: Vec<String>,
}

pub(super) fn collect(
    root: u32,
    sampled_before: u64,
    info: impl Fn(u32) -> anyhow::Result<Option<(u32, Start)>>,
    mut arguments: impl FnMut(u32) -> anyhow::Result<Vec<String>>,
    mut children: impl FnMut(u32) -> anyhow::Result<Vec<u32>>,
) -> anyhow::Result<Vec<PaneProcess>> {
    ensure!(
        root > 1 && root <= i32::MAX as u32,
        "Invalid pane root PID {root}"
    );
    let mut queue = vec![(root, None)];
    let mut seen = std::collections::HashSet::new();
    let mut processes = Vec::new();
    while let Some((pid, parent)) = queue.pop() {
        ensure!(seen.insert(pid), "Process tree changed at PID {pid}");
        let Some((ppid, start)) = info(pid)? else {
            ensure!(pid != root, "Pane root PID {pid} disappeared");
            continue;
        };
        ensure!(
            parent.map_or(true, |p| p == ppid),
            "PID {pid} changed parent"
        );
        ensure!(
            pid != root || start < sampled_before,
            "Pane root PID {pid} was born during or after inventory"
        );
        let args = arguments(pid).with_context(|| format!("Reading argv of PID {pid}"))?;
        let children = children(pid).with_context(|| format!("Listing children of PID {pid}"))?;
        if !children.is_empty() {
            ensure!(
                info(pid)? == Some((ppid, start)),
                "PID {pid} changed before visiting children"
            );
        }
        processes.push(PaneProcess {
            pid,
            parent: ppid,
            start,
            args,
        });
        queue.extend(children.into_iter().map(|child| (child, Some(pid))));
    }
    // Check ancestors last so a recycled parent cannot lend its PID to a new subtree.
    for process in processes.iter().rev() {
        ensure!(
            info(process.pid)? == Some((process.parent, process.start)),
            "PID {} changed during inspection",
            process.pid
        );
    }
    Ok(processes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_and_same_tick_roots_are_rejected_before_reading_argv() {
        for birth in [4, 5] {
            assert!(collect(
                20,
                4,
                |_| Ok(Some((2, birth))),
                |_| panic!("read stale argv"),
                |_| panic!("walked stale root")
            )
            .is_err());
        }
    }

    #[test]
    fn an_ancestor_recycled_while_reading_its_child_invalidates_the_tree() {
        let replaced = std::cell::Cell::new(false);
        let info = |pid| match pid {
            10 => Ok(Some((2, 1 + u64::from(replaced.get())))),
            11 => Ok(Some((10, 2))),
            _ => panic!("unrelated PID {pid}"),
        };
        let arguments = |pid| {
            if pid == 11 {
                replaced.set(true);
            }
            Ok(vec![String::from("codex")])
        };
        assert!(collect(10, 5, info, arguments, |pid| Ok(if pid == 10 {
            vec![11]
        } else {
            vec![]
        }))
        .is_err());
    }
}
