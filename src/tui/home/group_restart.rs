use crate::session::{Instance, Status};

pub(super) fn restart_targets(instances: &[Instance], group: &str) -> Vec<String> {
    let prefix = format!("{group}/");
    instances
        .iter()
        .filter(|inst| inst.group_path == group || inst.group_path.starts_with(&prefix))
        .filter(|inst| inst.tool != "shell" && !inst.expects_shell())
        .filter(|inst| inst.status != Status::Deleting && !inst.restart_in_flight)
        .map(|inst| inst.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, group: &str, tool: &str) -> Instance {
        Instance {
            id: id.to_string(),
            group_path: group.to_string(),
            tool: tool.to_string(),
            ..Instance::new(id, "/tmp")
        }
    }

    #[test]
    fn includes_descendants_but_not_similar_names_or_shells() {
        let instances = vec![
            session("direct", "work", "claude"),
            session("nested", "work/api", "codex"),
            session("similar", "workspace", "claude"),
            session("other", "personal", "claude"),
            session("shell", "work", "shell"),
            Instance {
                command: "/bin/zsh".to_string(),
                ..session("custom-shell", "work", "custom")
            },
        ];
        assert_eq!(restart_targets(&instances, "work"), ["direct", "nested"]);
    }

    #[test]
    fn skips_busy_members_and_keeps_stopped_members() {
        let instances = vec![
            Instance {
                status: Status::Deleting,
                ..session("deleting", "work", "claude")
            },
            Instance {
                restart_in_flight: true,
                ..session("restarting", "work", "codex")
            },
            Instance {
                status: Status::Stopped,
                ..session("stopped", "work", "claude")
            },
        ];
        assert_eq!(restart_targets(&instances, "work"), ["stopped"]);
        assert!(restart_targets(&instances, "empty").is_empty());
    }
}
