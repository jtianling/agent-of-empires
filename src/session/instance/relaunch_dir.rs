use std::path::Path;

use super::{Instance, PaneConfig};
use crate::db::AgentSlot;

impl Instance {
    /// The pane config a tracked pane relaunches with: the slot's own, except
    /// that a directory that no longer exists (a removed worktree the agent had
    /// moved into, say) is replaced by the directory the pane was set up with.
    /// tmux would otherwise silently start the pane in `$HOME`.
    ///
    /// The replacement is noted in `relaunch_notes` so the session can say so.
    pub(super) fn relaunch_pane_config(&mut self, slot: &AgentSlot) -> PaneConfig {
        let mut pane = slot.pane_config();
        // A sandboxed pane's directory lives in its container, not on this host.
        if self.is_sandboxed() || Path::new(&pane.working_dir).is_dir() {
            return pane;
        }
        // An extra pane goes back to where it was created; the session's own
        // directory backs that up too, since a pane recorded before creation
        // directories were kept may have only the directory it moved into.
        let session_dir = self.primary_pane.working_dir.clone();
        let candidates = if slot.slot == 0 || slot.launch_cwd.is_empty() {
            vec![session_dir]
        } else {
            vec![slot.launch_cwd.clone(), session_dir]
        };
        let note = match candidates.iter().find(|dir| Path::new(dir).is_dir()) {
            Some(dir) => {
                let note = format!(
                    "pane {}: {} no longer exists, started in {}",
                    slot.slot + 1,
                    pane.working_dir,
                    dir
                );
                pane.working_dir = dir.clone();
                note
            }
            None => format!(
                "pane {}: {} no longer exists and neither does {}",
                slot.slot + 1,
                pane.working_dir,
                candidates.join(" or ")
            ),
        };
        tracing::warn!("Relaunching '{}': {}", self.title, note);
        self.relaunch_notes.push(note);
        pane
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(index: i64, cwd: &str, launch_cwd: &str) -> AgentSlot {
        AgentSlot {
            instance_id: "inst".to_string(),
            slot: index,
            agent: "claude".to_string(),
            native_session_id: String::new(),
            cwd: cwd.to_string(),
            launch_cwd: launch_cwd.to_string(),
            tmux_pane: String::new(),
            xats_identity_key: String::new(),
            xats_runtime_generation: 0,
            yolo_mode: false,
            cross_agent_team: false,
            worktree_info: None,
            model: String::new(),
            model_fingerprint: String::new(),
            last_seen_at: 0,
        }
    }

    struct Dirs {
        _root: tempfile::TempDir,
        project: String,
        extra: String,
        gone: String,
    }

    fn dirs() -> Dirs {
        let root = tempfile::tempdir().unwrap();
        let path = |name: &str| {
            let dir = root.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            dir.to_string_lossy().into_owned()
        };
        let project = path("project");
        let extra = path("extra");
        let gone = root.path().join("gone").to_string_lossy().into_owned();
        Dirs {
            _root: root,
            project,
            extra,
            gone,
        }
    }

    fn instance(project: &str) -> Instance {
        let mut inst = Instance::new("test", project);
        inst.tool = "claude".to_string();
        inst.sync_primary_pane_from_legacy();
        inst
    }

    #[test]
    fn existing_directory_is_kept() {
        let d = dirs();
        let mut inst = instance(&d.project);
        let pane = inst.relaunch_pane_config(&slot(1, &d.extra, &d.project));
        assert_eq!(pane.working_dir, d.extra);
        assert!(inst.relaunch_notes.is_empty());
    }

    #[test]
    fn primary_pane_falls_back_to_the_session_directory() {
        let d = dirs();
        let mut inst = instance(&d.project);
        let pane = inst.relaunch_pane_config(&slot(0, &d.gone, &d.extra));
        assert_eq!(pane.working_dir, d.project);
        let note = inst.relaunch_notes.join("; ");
        assert!(
            note.contains(&d.gone) && note.contains(&d.project),
            "{note}"
        );
    }

    #[test]
    fn extra_pane_falls_back_to_where_it_was_created() {
        let d = dirs();
        let mut inst = instance(&d.project);
        let pane = inst.relaunch_pane_config(&slot(1, &d.gone, &d.extra));
        assert_eq!(pane.working_dir, d.extra);
        let pane = inst.relaunch_pane_config(&slot(2, &d.gone, ""));
        assert_eq!(pane.working_dir, d.project);
    }

    #[test]
    fn extra_pane_whose_creation_dir_is_gone_too_uses_the_session_directory() {
        let d = dirs();
        let mut inst = instance(&d.project);
        let pane = inst.relaunch_pane_config(&slot(1, &d.gone, &d.gone));
        assert_eq!(pane.working_dir, d.project);
    }

    #[test]
    fn nothing_left_to_fall_back_to_is_reported() {
        let d = dirs();
        let mut inst = instance(&d.gone);
        let pane = inst.relaunch_pane_config(&slot(1, &d.gone, &d.gone));
        assert_eq!(pane.working_dir, d.gone);
        assert!(inst.relaunch_notes[0].contains("neither"));
    }
}
