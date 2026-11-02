use std::collections::HashMap;
use std::path::Path;

use crate::session::Instance;

pub(super) fn detect_branches(instances: &[Instance]) -> HashMap<String, String> {
    instances
        .iter()
        .filter(|instance| {
            instance.worktree_info.is_none()
                && instance.workspace_info.is_none()
                && !instance.is_sandboxed()
        })
        .filter_map(|instance| match linked_branch(&instance.project_path) {
            Ok(Some(branch)) => Some((instance.id.clone(), branch)),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(
                    "Failed to detect worktree branch for {} at {}: {}",
                    instance.id,
                    instance.project_path,
                    error
                );
                None
            }
        })
        .collect()
}

fn linked_branch(path: &str) -> Result<Option<String>, git2::Error> {
    let repo = match crate::git::open_repo_at(Path::new(path)) {
        Ok(repo) => repo,
        Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !repo.is_worktree() {
        return Ok(None);
    }
    let head = repo.head()?;
    if !head.is_branch() {
        return Ok(None);
    }
    head.shorthand()
        .map(|branch| Some(branch.to_string()))
        .ok_or_else(|| git2::Error::from_str("Worktree branch is not valid UTF-8"))
}
