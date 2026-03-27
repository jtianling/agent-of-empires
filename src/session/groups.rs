//! Group tree management

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use super::config::SortOrder;
use super::Instance;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub default_directory: Option<String>,
    #[serde(skip)]
    pub children: Vec<Group>,
}

impl Group {
    pub fn new(name: &str, path: &str) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            collapsed: false,
            default_directory: None,
            children: Vec::new(),
        }
    }
}

pub fn expanded_groups(groups: &[Group]) -> Vec<Group> {
    groups
        .iter()
        .cloned()
        .map(|mut group| {
            group.collapsed = false;
            group.children.clear();
            group
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct GroupTree {
    roots: Vec<Group>,
    groups_by_path: HashMap<String, Group>,
    /// Tracks the first-seen insertion order of group paths (used as a stable base for other sorts).
    insertion_order: Vec<String>,
}

impl GroupTree {
    pub fn new_with_groups(instances: &[Instance], existing_groups: &[Group]) -> Self {
        let mut tree = Self {
            roots: Vec::new(),
            groups_by_path: HashMap::new(),
            insertion_order: Vec::new(),
        };

        // Add existing groups in the order they appear on disk (preserves prior save order)
        for group in existing_groups {
            tree.groups_by_path
                .insert(group.path.clone(), group.clone());
            tree.insertion_order.push(group.path.clone());
        }

        // Ensure all instance groups exist
        for inst in instances {
            if !inst.group_path.is_empty() {
                tree.ensure_group_exists(&inst.group_path);
            }
        }

        // Build tree structure
        tree.rebuild_tree();

        tree
    }

    fn ensure_group_exists(&mut self, path: &str) {
        if self.groups_by_path.contains_key(path) {
            return;
        }

        // Create all parent groups
        let parts: Vec<&str> = path.split('/').collect();
        let mut current_path = String::new();

        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                current_path.push('/');
            }
            current_path.push_str(part);

            if !self.groups_by_path.contains_key(&current_path) {
                let group = Group::new(part, &current_path);
                self.groups_by_path.insert(current_path.clone(), group);
                self.insertion_order.push(current_path.clone());
            }
        }
    }

    fn rebuild_tree(&mut self) {
        self.roots.clear();

        // Build root groups in insertion order (no '/' in path); flatten_tree applies sort order.
        let root_paths: Vec<String> = self
            .insertion_order
            .iter()
            .filter(|p| self.groups_by_path.contains_key(*p) && !p.contains('/'))
            .cloned()
            .collect();

        let mut root_groups: Vec<Group> = root_paths
            .iter()
            .filter_map(|p| self.groups_by_path.get(p).cloned())
            .collect();

        for root in &mut root_groups {
            self.build_children(root);
        }

        self.roots = root_groups;
    }

    fn build_children(&self, parent: &mut Group) {
        let prefix = format!("{}/", parent.path);

        // Build children in insertion order
        let child_paths: Vec<String> = self
            .insertion_order
            .iter()
            .filter(|p| {
                self.groups_by_path.contains_key(*p)
                    && p.starts_with(&prefix)
                    && !p[prefix.len()..].contains('/')
            })
            .cloned()
            .collect();

        let mut children: Vec<Group> = child_paths
            .iter()
            .filter_map(|p| self.groups_by_path.get(p).cloned())
            .collect();

        for child in &mut children {
            self.build_children(child);
        }

        parent.children = children;
    }

    pub fn create_group(&mut self, path: &str) {
        self.ensure_group_exists(path);
        self.rebuild_tree();
    }

    pub fn rename_group(&mut self, old_path: &str, new_path: &str) -> Result<()> {
        validate_group_path(new_path)?;

        if old_path == new_path {
            return Ok(());
        }

        if !self.group_exists(old_path) {
            bail!("Group '{old_path}' does not exist");
        }

        if new_path.starts_with(&format!("{old_path}/")) {
            bail!("Cannot move a group into its own subtree");
        }

        if let Some(parent) = parent_group_path(new_path) {
            self.ensure_group_exists(parent);
        }

        let target_exists = self.group_exists(new_path);
        let source_prefix = format!("{old_path}/");
        let existing_groups = self.groups_by_path.clone();
        let existing_order = self.insertion_order.clone();
        let source_paths: HashSet<String> = existing_order
            .iter()
            .filter(|path| **path == old_path || path.starts_with(&source_prefix))
            .cloned()
            .collect();

        let mut groups_by_path = HashMap::new();
        let mut insertion_order = Vec::new();

        for path in existing_order {
            let Some(existing_group) = existing_groups.get(&path) else {
                continue;
            };

            let Some(mapped_path) = remap_group_path(&path, old_path, new_path) else {
                groups_by_path.insert(path.clone(), existing_group.clone());
                insertion_order.push(path);
                continue;
            };

            if path == old_path && target_exists {
                continue;
            }

            if existing_groups.contains_key(&mapped_path) && !source_paths.contains(&mapped_path) {
                continue;
            }

            if groups_by_path.contains_key(&mapped_path) {
                continue;
            }

            let mut renamed_group = existing_group.clone();
            renamed_group.path = mapped_path.clone();
            renamed_group.name = group_name(&mapped_path).to_string();
            renamed_group.children.clear();
            groups_by_path.insert(mapped_path.clone(), renamed_group);
            insertion_order.push(mapped_path);
        }

        self.groups_by_path = groups_by_path;
        self.insertion_order = normalize_insertion_order(insertion_order, &self.groups_by_path);

        // Any source paths skipped due to merge must be removed from the final ordering.
        self.insertion_order
            .retain(|path| !source_paths.contains(path) || self.groups_by_path.contains_key(path));
        self.rebuild_tree();
        Ok(())
    }

    pub fn delete_group(&mut self, path: &str) {
        // Remove group and all children
        let prefix = format!("{}/", path);
        let to_remove: Vec<String> = self
            .groups_by_path
            .keys()
            .filter(|p| *p == path || p.starts_with(&prefix))
            .cloned()
            .collect();

        for p in &to_remove {
            self.groups_by_path.remove(p);
        }
        self.insertion_order.retain(|p| !to_remove.contains(p));

        self.rebuild_tree();
    }

    pub fn group_exists(&self, path: &str) -> bool {
        self.groups_by_path.contains_key(path)
    }

    pub fn set_default_directory(&mut self, path: &str, directory: &str) {
        if let Some(group) = self.groups_by_path.get_mut(path) {
            group.default_directory = Some(directory.to_string());
            self.rebuild_tree();
        }
    }

    pub fn clear_default_directory(&mut self, path: &str) {
        if let Some(group) = self.groups_by_path.get_mut(path) {
            group.default_directory = None;
            self.rebuild_tree();
        }
    }

    pub fn get_default_directory(&self, path: &str) -> Option<&str> {
        self.groups_by_path
            .get(path)
            .and_then(|g| g.default_directory.as_deref())
    }

    pub fn get_group_directories(&self) -> HashMap<String, String> {
        self.groups_by_path
            .iter()
            .filter_map(|(path, group)| {
                group
                    .default_directory
                    .as_ref()
                    .map(|dir| (path.clone(), dir.clone()))
            })
            .collect()
    }

    pub fn get_all_groups(&self) -> Vec<Group> {
        // Return in insertion order so groups.json preserves creation order
        self.insertion_order
            .iter()
            .filter_map(|p| self.groups_by_path.get(p).cloned())
            .collect()
    }

    pub fn get_roots(&self) -> &[Group] {
        &self.roots
    }

    pub fn toggle_collapsed(&mut self, path: &str) {
        if let Some(group) = self.groups_by_path.get_mut(path) {
            group.collapsed = !group.collapsed;
            self.rebuild_tree();
        }
    }

    pub fn move_group(&mut self, path: &str, delta: i32) -> bool {
        if delta == 0 || !self.groups_by_path.contains_key(path) {
            return false;
        }

        let parent = parent_group_path(path).unwrap_or_default().to_string();
        let mut sibling_orders = self.sibling_orders();
        let siblings = match sibling_orders.get_mut(&parent) {
            Some(siblings) => siblings,
            None => return false,
        };

        let idx = match siblings.iter().position(|candidate| candidate == path) {
            Some(idx) => idx,
            None => return false,
        };

        let target_idx = if delta < 0 {
            match idx.checked_sub((-delta) as usize) {
                Some(target_idx) => target_idx,
                None => return false,
            }
        } else {
            let target_idx = idx + delta as usize;
            if target_idx >= siblings.len() {
                return false;
            }
            target_idx
        };

        let moved = siblings.remove(idx);
        siblings.insert(target_idx, moved);
        self.rebuild_insertion_order_from_siblings(&sibling_orders);
        true
    }

    fn sibling_orders(&self) -> HashMap<String, Vec<String>> {
        let mut sibling_orders: HashMap<String, Vec<String>> = HashMap::new();

        for path in &self.insertion_order {
            if !self.groups_by_path.contains_key(path) {
                continue;
            }

            sibling_orders
                .entry(parent_group_path(path).unwrap_or_default().to_string())
                .or_default()
                .push(path.clone());
        }

        sibling_orders
    }

    fn rebuild_insertion_order_from_siblings(
        &mut self,
        sibling_orders: &HashMap<String, Vec<String>>,
    ) {
        fn push_subtree(
            parent: &str,
            sibling_orders: &HashMap<String, Vec<String>>,
            new_order: &mut Vec<String>,
        ) {
            if let Some(children) = sibling_orders.get(parent) {
                for child in children {
                    new_order.push(child.clone());
                    push_subtree(child, sibling_orders, new_order);
                }
            }
        }

        let mut new_order = Vec::new();
        push_subtree("", sibling_orders, &mut new_order);
        self.insertion_order = new_order;
        self.rebuild_tree();
    }

    pub fn set_collapsed(&mut self, path: &str, collapsed: bool) {
        if let Some(group) = self.groups_by_path.get_mut(path) {
            if group.collapsed != collapsed {
                group.collapsed = collapsed;
                self.rebuild_tree();
            }
        }
    }
}

pub fn validate_group_path(path: &str) -> Result<()> {
    if path.is_empty() {
        bail!("Group path cannot be empty");
    }
    if path.starts_with('/') || path.ends_with('/') {
        bail!("Group path cannot start or end with '/'");
    }
    if path.contains("//") {
        bail!("Group path cannot contain consecutive '/' characters");
    }
    Ok(())
}

fn parent_group_path(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(parent, _)| parent)
}

fn group_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn remap_group_path(path: &str, old_path: &str, new_path: &str) -> Option<String> {
    if path == old_path {
        return Some(new_path.to_string());
    }

    path.strip_prefix(&format!("{old_path}/"))
        .map(|suffix| format!("{new_path}/{suffix}"))
}

fn normalize_insertion_order(
    candidates: Vec<String>,
    groups_by_path: &HashMap<String, Group>,
) -> Vec<String> {
    fn push_path(
        path: &str,
        groups_by_path: &HashMap<String, Group>,
        seen: &mut HashSet<String>,
        ordered: &mut Vec<String>,
    ) {
        if seen.contains(path) || !groups_by_path.contains_key(path) {
            return;
        }

        if let Some(parent) = parent_group_path(path) {
            push_path(parent, groups_by_path, seen, ordered);
        }

        seen.insert(path.to_string());
        ordered.push(path.to_string());
    }

    let mut ordered = Vec::new();
    let mut seen = HashSet::new();
    for path in candidates {
        push_path(&path, groups_by_path, &mut seen, &mut ordered);
    }
    ordered
}

/// Item represents either a group or an instance in the flattened tree view
#[derive(Debug, Clone)]
pub enum Item {
    Group {
        path: String,
        name: String,
        depth: usize,
        collapsed: bool,
        session_count: usize,
        /// Which profile this group belongs to (set in all-profiles mode)
        profile: Option<String>,
    },
    Session {
        id: String,
        depth: usize,
    },
    ProfileHeader {
        name: String,
        collapsed: bool,
        session_count: usize,
    },
}

impl Item {
    pub fn depth(&self) -> usize {
        match self {
            Item::Group { depth, .. } => *depth,
            Item::Session { depth, .. } => *depth,
            Item::ProfileHeader { .. } => 0,
        }
    }
}

fn sort_by_name<T, F>(items: &mut [T], sort_order: SortOrder, key: F)
where
    F: Fn(&T) -> &str,
{
    match sort_order {
        SortOrder::AZ => items.sort_by_key(|a| key(a).to_lowercase()),
        SortOrder::ZA => items.sort_by_key(|b| std::cmp::Reverse(key(b).to_lowercase())),
        _ => {}
    }
}

/// Get the most recent created_at among all sessions (direct and nested) in a group.
/// Returns DateTime::MIN_UTC if the group has no sessions.
fn max_created_at_in_group(path: &str, instances: &[Instance]) -> DateTime<Utc> {
    let prefix = format!("{}/", path);
    instances
        .iter()
        .filter(|i| i.group_path == path || i.group_path.starts_with(&prefix))
        .map(|i| i.created_at)
        .max()
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// Get the oldest created_at among all sessions (direct and nested) in a group.
/// Returns DateTime::MAX_UTC if the group has no sessions (so empty groups sink to the bottom).
fn min_created_at_in_group(path: &str, instances: &[Instance]) -> DateTime<Utc> {
    let prefix = format!("{}/", path);
    instances
        .iter()
        .filter(|i| i.group_path == path || i.group_path.starts_with(&prefix))
        .map(|i| i.created_at)
        .min()
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

/// Flatten instances from multiple profiles into a single flat list.
/// Merges all profiles' sessions and groups at depth 0 (no profile headers).
/// Uses per-profile GroupTrees so collapsed state is isolated per profile.
pub fn flatten_tree_all_profiles(
    instances: &[Instance],
    group_trees: &std::collections::HashMap<String, GroupTree>,
    sort_order: SortOrder,
) -> Vec<Item> {
    let mut items = Vec::new();

    // Collect all ungrouped sessions across all profiles
    let mut ungrouped: Vec<&Instance> = instances
        .iter()
        .filter(|i| i.group_path.is_empty())
        .collect();

    match sort_order {
        SortOrder::Oldest => ungrouped.sort_by_key(|i| i.created_at),
        SortOrder::Newest => ungrouped.sort_by_key(|i| Reverse(i.created_at)),
        _ => sort_by_name(&mut ungrouped, sort_order, |i| &i.title),
    }

    for inst in ungrouped {
        items.push(Item::Session {
            id: inst.id.clone(),
            depth: 0,
        });
    }

    // Collect and flatten groups from all profiles at depth 0
    let mut all_roots: Vec<(&str, &Group, Vec<Instance>)> = Vec::new();
    for (profile_name, tree) in group_trees {
        let profile_instances: Vec<Instance> = instances
            .iter()
            .filter(|i| i.source_profile == *profile_name)
            .cloned()
            .collect();
        for root in tree.get_roots() {
            all_roots.push((profile_name, root, profile_instances.clone()));
        }
    }

    match sort_order {
        SortOrder::Oldest => {
            all_roots.sort_by_key(|(_, g, insts)| min_created_at_in_group(&g.path, insts));
        }
        SortOrder::Newest => {
            all_roots.sort_by_key(|(_, g, insts)| Reverse(max_created_at_in_group(&g.path, insts)));
        }
        _ => all_roots.sort_by_key(|(_, g, _)| g.name.to_lowercase()),
    }
    if matches!(sort_order, SortOrder::ZA) {
        all_roots.reverse();
    }

    for (profile_name, root, profile_instances) in &all_roots {
        flatten_group(
            root,
            profile_instances,
            &mut items,
            0,
            sort_order,
            Some(profile_name),
        );
    }

    items
}

pub fn flatten_tree(
    group_tree: &GroupTree,
    instances: &[Instance],
    sort_order: SortOrder,
) -> Vec<Item> {
    let mut items = Vec::new();

    // Add ungrouped sessions first (always at top, sorted if needed)
    let mut ungrouped: Vec<&Instance> = instances
        .iter()
        .filter(|i| i.group_path.is_empty())
        .collect();

    match sort_order {
        SortOrder::Oldest => ungrouped.sort_by_key(|i| i.created_at),
        SortOrder::Newest => ungrouped.sort_by_key(|i| Reverse(i.created_at)),
        SortOrder::Manual => {}
        _ => sort_by_name(&mut ungrouped, sort_order, |i| &i.title),
    }

    for inst in ungrouped {
        items.push(Item::Session {
            id: inst.id.clone(),
            depth: 0,
        });
    }

    // Add groups and their sessions
    let roots = group_tree.get_roots();
    let mut roots_to_iterate: Vec<&Group> = roots.iter().collect();
    match sort_order {
        SortOrder::Oldest => {
            roots_to_iterate.sort_by_key(|g| min_created_at_in_group(&g.path, instances));
        }
        SortOrder::Newest => {
            roots_to_iterate.sort_by_key(|g| Reverse(max_created_at_in_group(&g.path, instances)));
        }
        SortOrder::Manual => {}
        _ => sort_by_name(&mut roots_to_iterate, sort_order, |g| &g.name),
    }

    for root in roots_to_iterate {
        flatten_group(root, instances, &mut items, 0, sort_order, None);
    }

    items
}

fn flatten_group(
    group: &Group,
    instances: &[Instance],
    items: &mut Vec<Item>,
    depth: usize,
    sort_order: SortOrder,
    profile: Option<&str>,
) {
    let session_count = count_sessions_in_group(&group.path, instances);

    items.push(Item::Group {
        path: group.path.clone(),
        name: group.name.clone(),
        depth,
        collapsed: group.collapsed,
        session_count,
        profile: profile.map(|s| s.to_string()),
    });

    if group.collapsed {
        return;
    }

    // Add sessions in this group (direct children only), sorted if needed
    let mut group_sessions: Vec<&Instance> = instances
        .iter()
        .filter(|i| i.group_path == group.path)
        .collect();

    match sort_order {
        SortOrder::Oldest => group_sessions.sort_by_key(|i| i.created_at),
        SortOrder::Newest => group_sessions.sort_by_key(|i| Reverse(i.created_at)),
        SortOrder::Manual => {}
        _ => sort_by_name(&mut group_sessions, sort_order, |i| &i.title),
    }

    for inst in group_sessions {
        items.push(Item::Session {
            id: inst.id.clone(),
            depth: depth + 1,
        });
    }

    // Recursively add child groups (sort them if needed)
    let mut children_to_iterate: Vec<&Group> = group.children.iter().collect();
    match sort_order {
        SortOrder::Oldest => {
            children_to_iterate.sort_by_key(|g| min_created_at_in_group(&g.path, instances));
        }
        SortOrder::Newest => {
            children_to_iterate
                .sort_by_key(|g| Reverse(max_created_at_in_group(&g.path, instances)));
        }
        SortOrder::Manual => {}
        _ => sort_by_name(&mut children_to_iterate, sort_order, |g| &g.name),
    }

    for child in children_to_iterate {
        flatten_group(child, instances, items, depth + 1, sort_order, profile);
    }
}

fn count_sessions_in_group(path: &str, instances: &[Instance]) -> usize {
    let prefix = format!("{}/", path);
    instances
        .iter()
        .filter(|i| i.group_path == path || i.group_path.starts_with(&prefix))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_tree_creation() {
        let mut inst1 = Instance::new("test1", "/tmp/1");
        inst1.group_path = "work".to_string();
        let mut inst2 = Instance::new("test2", "/tmp/2");
        inst2.group_path = "work/frontend".to_string();
        let mut inst3 = Instance::new("test3", "/tmp/3");
        inst3.group_path = "personal".to_string();

        let instances = vec![inst1, inst2, inst3];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        assert!(tree.group_exists("work"));
        assert!(tree.group_exists("work/frontend"));
        assert!(tree.group_exists("personal"));
        assert!(!tree.group_exists("nonexistent"));
    }

    #[test]
    fn test_flatten_tree() {
        let ungrouped = Instance::new("ungrouped", "/tmp/u");
        let mut inst1 = Instance::new("test1", "/tmp/1");
        inst1.group_path = "work".to_string();
        let mut inst2 = Instance::new("test2", "/tmp/2");
        inst2.group_path = "work".to_string();

        let instances = vec![ungrouped, inst1, inst2];
        let tree = GroupTree::new_with_groups(&instances, &[]);
        let items = flatten_tree(&tree, &instances, SortOrder::Oldest);

        assert!(!items.is_empty());

        // First item should be ungrouped session
        assert!(matches!(items[0], Item::Session { .. }));
    }

    #[test]
    fn test_toggle_collapsed() {
        let mut inst = Instance::new("test", "/tmp/t");
        inst.group_path = "work".to_string();
        let instances = vec![inst];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        let group = tree.groups_by_path.get("work").unwrap();
        assert!(!group.collapsed);

        tree.toggle_collapsed("work");

        let group = tree.groups_by_path.get("work").unwrap();
        assert!(group.collapsed);

        tree.toggle_collapsed("work");

        let group = tree.groups_by_path.get("work").unwrap();
        assert!(!group.collapsed);
    }

    #[test]
    fn test_toggle_collapsed_nonexistent_group() {
        let instances: Vec<Instance> = vec![];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);
        tree.toggle_collapsed("nonexistent");
    }

    #[test]
    fn test_collapsed_group_hides_sessions_in_flatten() {
        let mut inst1 = Instance::new("work-session", "/tmp/w");
        inst1.group_path = "work".to_string();
        let instances = vec![inst1];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        let items_expanded = flatten_tree(&tree, &instances, SortOrder::Oldest);
        let session_count_expanded = items_expanded
            .iter()
            .filter(|i| matches!(i, Item::Session { .. }))
            .count();
        assert_eq!(session_count_expanded, 1);

        tree.toggle_collapsed("work");
        let items_collapsed = flatten_tree(&tree, &instances, SortOrder::Oldest);
        let session_count_collapsed = items_collapsed
            .iter()
            .filter(|i| matches!(i, Item::Session { .. }))
            .count();
        assert_eq!(session_count_collapsed, 0);
    }

    #[test]
    fn test_collapsed_group_still_shows_in_flatten() {
        let mut inst = Instance::new("test", "/tmp/t");
        inst.group_path = "work".to_string();
        let instances = vec![inst];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        tree.toggle_collapsed("work");
        let items = flatten_tree(&tree, &instances, SortOrder::Oldest);

        let group_items: Vec<_> = items
            .iter()
            .filter(|i| matches!(i, Item::Group { .. }))
            .collect();
        assert_eq!(group_items.len(), 1);
    }

    #[test]
    fn test_collapsed_state_in_flattened_item() {
        let mut inst = Instance::new("test", "/tmp/t");
        inst.group_path = "work".to_string();
        let instances = vec![inst];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        let items = flatten_tree(&tree, &instances, SortOrder::Oldest);
        if let Some(Item::Group { collapsed, .. }) = items
            .iter()
            .find(|i| matches!(i, Item::Group { path, .. } if path == "work"))
        {
            assert!(!collapsed);
        }

        tree.toggle_collapsed("work");
        let items = flatten_tree(&tree, &instances, SortOrder::Oldest);
        if let Some(Item::Group { collapsed, .. }) = items
            .iter()
            .find(|i| matches!(i, Item::Group { path, .. } if path == "work"))
        {
            assert!(*collapsed);
        }
    }

    #[test]
    fn test_nested_group_collapse_hides_children() {
        let mut inst1 = Instance::new("parent-session", "/tmp/p");
        inst1.group_path = "parent".to_string();
        let mut inst2 = Instance::new("child-session", "/tmp/c");
        inst2.group_path = "parent/child".to_string();
        let instances = vec![inst1, inst2];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        let items = flatten_tree(&tree, &instances, SortOrder::Oldest);
        let group_count = items
            .iter()
            .filter(|i| matches!(i, Item::Group { .. }))
            .count();
        assert_eq!(group_count, 2);

        tree.toggle_collapsed("parent");
        let items = flatten_tree(&tree, &instances, SortOrder::Oldest);
        let group_count_collapsed = items
            .iter()
            .filter(|i| matches!(i, Item::Group { .. }))
            .count();
        assert_eq!(group_count_collapsed, 1);
    }

    #[test]
    fn test_session_count_includes_nested() {
        let mut inst1 = Instance::new("parent-session", "/tmp/p");
        inst1.group_path = "parent".to_string();
        let mut inst2 = Instance::new("child-session", "/tmp/c");
        inst2.group_path = "parent/child".to_string();
        let instances = vec![inst1, inst2];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        let items = flatten_tree(&tree, &instances, SortOrder::Oldest);
        if let Some(Item::Group { session_count, .. }) = items
            .iter()
            .find(|i| matches!(i, Item::Group { path, .. } if path == "parent"))
        {
            assert_eq!(*session_count, 2);
        }
    }

    #[test]
    fn test_delete_group() {
        let mut inst = Instance::new("test", "/tmp/t");
        inst.group_path = "work".to_string();
        let instances = vec![inst];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        assert!(tree.group_exists("work"));
        tree.delete_group("work");
        assert!(!tree.group_exists("work"));
    }

    #[test]
    fn test_delete_group_removes_children() {
        let mut inst1 = Instance::new("parent-session", "/tmp/p");
        inst1.group_path = "parent".to_string();
        let mut inst2 = Instance::new("child-session", "/tmp/c");
        inst2.group_path = "parent/child".to_string();
        let instances = vec![inst1, inst2];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        assert!(tree.group_exists("parent"));
        assert!(tree.group_exists("parent/child"));

        tree.delete_group("parent");

        assert!(!tree.group_exists("parent"));
        assert!(!tree.group_exists("parent/child"));
    }

    #[test]
    fn test_create_group() {
        let instances: Vec<Instance> = vec![];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        assert!(!tree.group_exists("new-group"));
        tree.create_group("new-group");
        assert!(tree.group_exists("new-group"));
    }

    #[test]
    fn test_create_nested_group_creates_parents() {
        let instances: Vec<Instance> = vec![];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        tree.create_group("a/b/c");
        assert!(tree.group_exists("a"));
        assert!(tree.group_exists("a/b"));
        assert!(tree.group_exists("a/b/c"));
    }

    #[test]
    fn test_validate_group_path_rejects_invalid_values() {
        assert!(validate_group_path("").is_err());
        assert!(validate_group_path("/work").is_err());
        assert!(validate_group_path("work/").is_err());
        assert!(validate_group_path("work//frontend").is_err());
        assert!(validate_group_path("work/frontend").is_ok());
    }

    #[test]
    fn test_rename_group_updates_path_and_metadata() {
        let mut inst = Instance::new("test", "/tmp/t");
        inst.group_path = "work".to_string();
        let instances = vec![inst];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);
        tree.toggle_collapsed("work");
        tree.set_default_directory("work", "/tmp/work");

        tree.rename_group("work", "projects").unwrap();

        assert!(!tree.group_exists("work"));
        assert!(tree.group_exists("projects"));

        let renamed = tree.groups_by_path.get("projects").unwrap();
        assert_eq!(renamed.name, "projects");
        assert!(renamed.collapsed);
        assert_eq!(renamed.default_directory.as_deref(), Some("/tmp/work"));
    }

    #[test]
    fn test_rename_group_cascades_to_children() {
        let mut parent = Instance::new("parent", "/tmp/p");
        parent.group_path = "work".to_string();
        let mut child = Instance::new("child", "/tmp/c");
        child.group_path = "work/frontend".to_string();
        let mut grandchild = Instance::new("grandchild", "/tmp/g");
        grandchild.group_path = "work/frontend/react".to_string();
        let instances = vec![parent, child, grandchild];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        tree.rename_group("work", "projects").unwrap();

        assert!(tree.group_exists("projects"));
        assert!(tree.group_exists("projects/frontend"));
        assert!(tree.group_exists("projects/frontend/react"));
        assert!(!tree.group_exists("work"));
        assert!(!tree.group_exists("work/frontend"));
    }

    #[test]
    fn test_rename_group_merges_into_existing_target() {
        let instances: Vec<Instance> = vec![];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);
        tree.create_group("temp/api");
        tree.create_group("temp/api/v2");
        tree.create_group("work/api");
        tree.create_group("work/api/v1");
        tree.toggle_collapsed("temp/api");
        tree.set_default_directory("temp/api", "/tmp/source");
        tree.set_default_directory("work/api", "/tmp/target");

        tree.rename_group("temp/api", "work/api").unwrap();

        assert!(!tree.group_exists("temp/api"));
        assert!(tree.group_exists("work/api"));
        assert!(tree.group_exists("work/api/v1"));
        assert!(tree.group_exists("work/api/v2"));

        let merged = tree.groups_by_path.get("work/api").unwrap();
        assert!(!merged.collapsed);
        assert_eq!(merged.default_directory.as_deref(), Some("/tmp/target"));
    }

    #[test]
    fn test_rename_group_creates_intermediate_groups() {
        let instances: Vec<Instance> = vec![];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);
        tree.create_group("misc");

        tree.rename_group("misc", "work/tools/misc").unwrap();

        assert!(tree.group_exists("work"));
        assert!(tree.group_exists("work/tools"));
        assert!(tree.group_exists("work/tools/misc"));
    }

    #[test]
    fn test_item_depth() {
        let ungrouped = Instance::new("ungrouped", "/tmp/u");
        let mut inst1 = Instance::new("root-level", "/tmp/r");
        inst1.group_path = "root".to_string();
        let mut inst2 = Instance::new("nested", "/tmp/n");
        inst2.group_path = "root/child".to_string();
        let instances = vec![ungrouped, inst1, inst2];
        let tree = GroupTree::new_with_groups(&instances, &[]);
        let items = flatten_tree(&tree, &instances, SortOrder::Oldest);

        for item in &items {
            match item {
                Item::Session { id, depth } if !id.is_empty() => {
                    if *depth == 0 {
                        continue;
                    }
                    assert!(*depth >= 1);
                }
                Item::Group { path, depth, .. } => {
                    if path == "root" {
                        assert_eq!(*depth, 0);
                    } else if path == "root/child" {
                        assert_eq!(*depth, 1);
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn test_get_roots_returns_only_top_level() {
        let mut inst1 = Instance::new("test1", "/tmp/1");
        inst1.group_path = "alpha".to_string();
        let mut inst2 = Instance::new("test2", "/tmp/2");
        inst2.group_path = "alpha/nested".to_string();
        let mut inst3 = Instance::new("test3", "/tmp/3");
        inst3.group_path = "beta".to_string();
        let instances = vec![inst1, inst2, inst3];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        let roots = tree.get_roots();
        assert_eq!(roots.len(), 2);

        let root_names: Vec<_> = roots.iter().map(|g| &g.name).collect();
        assert!(root_names.contains(&&"alpha".to_string()));
        assert!(root_names.contains(&&"beta".to_string()));
    }

    #[test]
    fn test_delete_group_removes_from_insertion_order() {
        let mut inst1 = Instance::new("alpha-session", "/tmp/a");
        inst1.group_path = "alpha".to_string();
        let mut inst2 = Instance::new("beta-session", "/tmp/b");
        inst2.group_path = "beta".to_string();
        let mut inst3 = Instance::new("gamma-session", "/tmp/g");
        inst3.group_path = "gamma".to_string();
        let instances = vec![inst1, inst2, inst3];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        let initial_groups_vec = tree.get_all_groups();
        let initial_groups: Vec<_> = initial_groups_vec.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(initial_groups, vec!["alpha", "beta", "gamma"]);

        tree.delete_group("beta");

        let after_delete_vec = tree.get_all_groups();
        let after_delete: Vec<_> = after_delete_vec.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(after_delete, vec!["alpha", "gamma"]);

        tree.create_group("zeta");

        let after_create_vec = tree.get_all_groups();
        let after_create: Vec<_> = after_create_vec.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(after_create, vec!["alpha", "gamma", "zeta"]);
    }

    #[test]
    fn test_group_sort_order_in_flatten_tree() {
        // Groups are created in order: zebra, apple, mango (by instance order)
        let mut inst1 = Instance::new("z-session", "/tmp/z");
        inst1.group_path = "zebra".to_string();
        let mut inst2 = Instance::new("a-session", "/tmp/a");
        inst2.group_path = "apple".to_string();
        let mut inst3 = Instance::new("m-session", "/tmp/m");
        inst3.group_path = "mango".to_string();
        let instances = vec![inst1, inst2, inst3];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        // SortOrder::Oldest: groups sorted by oldest session (zebra, apple, mango)
        let items_oldest = flatten_tree(&tree, &instances, SortOrder::Oldest);
        let group_names_none: Vec<_> = items_oldest
            .iter()
            .filter_map(|i| match i {
                Item::Group { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(group_names_none, vec!["zebra", "apple", "mango"]);

        // SortOrder::AZ: groups appear alphabetically
        let items_az = flatten_tree(&tree, &instances, SortOrder::AZ);
        let group_names_az: Vec<_> = items_az
            .iter()
            .filter_map(|i| match i {
                Item::Group { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(group_names_az, vec!["apple", "mango", "zebra"]);

        // SortOrder::ZA: groups appear reverse alphabetically
        let items_za = flatten_tree(&tree, &instances, SortOrder::ZA);
        let group_names_za: Vec<_> = items_za
            .iter()
            .filter_map(|i| match i {
                Item::Group { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(group_names_za, vec!["zebra", "mango", "apple"]);
    }

    #[test]
    fn test_sort_order_cycle() {
        assert_eq!(SortOrder::Newest.cycle(), SortOrder::Oldest);
        assert_eq!(SortOrder::Oldest.cycle(), SortOrder::AZ);
        assert_eq!(SortOrder::AZ.cycle(), SortOrder::ZA);
        assert_eq!(SortOrder::ZA.cycle(), SortOrder::Manual);
        assert_eq!(SortOrder::Manual.cycle(), SortOrder::Newest);
    }

    #[test]
    fn test_sort_order_cycle_reverse() {
        assert_eq!(SortOrder::Newest.cycle_reverse(), SortOrder::Manual);
        assert_eq!(SortOrder::Manual.cycle_reverse(), SortOrder::ZA);
        assert_eq!(SortOrder::ZA.cycle_reverse(), SortOrder::AZ);
        assert_eq!(SortOrder::AZ.cycle_reverse(), SortOrder::Oldest);
        assert_eq!(SortOrder::Oldest.cycle_reverse(), SortOrder::Newest);
    }

    #[test]
    fn test_ungrouped_session_sort_oldest_preserves_insertion_order() {
        let inst1 = Instance::new("Mango", "/tmp/m");
        let inst2 = Instance::new("Apple", "/tmp/a");
        let inst3 = Instance::new("Zebra", "/tmp/z");
        let instances = vec![inst1, inst2, inst3];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        let items = flatten_tree(&tree, &instances, SortOrder::Oldest);
        let session_titles: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Session { id, .. } => instances
                    .iter()
                    .find(|inst| &inst.id == id)
                    .map(|inst| inst.title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(session_titles, vec!["Mango", "Apple", "Zebra"]);
    }

    #[test]
    fn test_ungrouped_session_sort_az() {
        let inst1 = Instance::new("Mango", "/tmp/m");
        let inst2 = Instance::new("Apple", "/tmp/a");
        let inst3 = Instance::new("Zebra", "/tmp/z");
        let instances = vec![inst1, inst2, inst3];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        let items = flatten_tree(&tree, &instances, SortOrder::AZ);
        let session_titles: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Session { id, .. } => instances
                    .iter()
                    .find(|inst| &inst.id == id)
                    .map(|inst| inst.title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(session_titles, vec!["Apple", "Mango", "Zebra"]);
    }

    #[test]
    fn test_ungrouped_session_sort_za() {
        let inst1 = Instance::new("Mango", "/tmp/m");
        let inst2 = Instance::new("Apple", "/tmp/a");
        let inst3 = Instance::new("Zebra", "/tmp/z");
        let instances = vec![inst1, inst2, inst3];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        let items = flatten_tree(&tree, &instances, SortOrder::ZA);
        let session_titles: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Session { id, .. } => instances
                    .iter()
                    .find(|inst| &inst.id == id)
                    .map(|inst| inst.title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(session_titles, vec!["Zebra", "Mango", "Apple"]);
    }

    #[test]
    fn test_session_sort_oldest_within_group_preserves_insertion_order() {
        let mut inst1 = Instance::new("Mango", "/tmp/m");
        inst1.group_path = "work".to_string();
        let mut inst2 = Instance::new("Apple", "/tmp/a");
        inst2.group_path = "work".to_string();
        let mut inst3 = Instance::new("Zebra", "/tmp/z");
        inst3.group_path = "work".to_string();
        let instances = vec![inst1, inst2, inst3];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        let items = flatten_tree(&tree, &instances, SortOrder::Oldest);
        let session_titles: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Session { id, .. } => instances
                    .iter()
                    .find(|inst| &inst.id == id)
                    .map(|inst| inst.title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(session_titles, vec!["Mango", "Apple", "Zebra"]);
    }

    #[test]
    fn test_session_sort_az_within_group() {
        let mut inst1 = Instance::new("Mango", "/tmp/m");
        inst1.group_path = "work".to_string();
        let mut inst2 = Instance::new("Apple", "/tmp/a");
        inst2.group_path = "work".to_string();
        let mut inst3 = Instance::new("Zebra", "/tmp/z");
        inst3.group_path = "work".to_string();
        let instances = vec![inst1, inst2, inst3];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        let items = flatten_tree(&tree, &instances, SortOrder::AZ);
        let session_titles: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Session { id, .. } => instances
                    .iter()
                    .find(|inst| &inst.id == id)
                    .map(|inst| inst.title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(session_titles, vec!["Apple", "Mango", "Zebra"]);
    }

    #[test]
    fn test_session_sort_za_within_group() {
        let mut inst1 = Instance::new("Mango", "/tmp/m");
        inst1.group_path = "work".to_string();
        let mut inst2 = Instance::new("Apple", "/tmp/a");
        inst2.group_path = "work".to_string();
        let mut inst3 = Instance::new("Zebra", "/tmp/z");
        inst3.group_path = "work".to_string();
        let instances = vec![inst1, inst2, inst3];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        let items = flatten_tree(&tree, &instances, SortOrder::ZA);
        let session_titles: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Session { id, .. } => instances
                    .iter()
                    .find(|inst| &inst.id == id)
                    .map(|inst| inst.title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(session_titles, vec!["Zebra", "Mango", "Apple"]);
    }

    #[test]
    fn test_nested_child_groups_sort_order() {
        let mut inst_parent = Instance::new("parent-session", "/tmp/parent");
        inst_parent.group_path = "parent".to_string();
        let mut inst_zeta = Instance::new("zeta-session", "/tmp/zeta");
        inst_zeta.group_path = "parent/zeta".to_string();
        let mut inst_alpha = Instance::new("alpha-session", "/tmp/alpha");
        inst_alpha.group_path = "parent/alpha".to_string();
        let instances = vec![inst_parent, inst_zeta, inst_alpha];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        let items_oldest = flatten_tree(&tree, &instances, SortOrder::Oldest);
        let child_names_oldest: Vec<_> = items_oldest
            .iter()
            .skip(1)
            .filter_map(|i| match i {
                Item::Group { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(child_names_oldest, vec!["zeta", "alpha"]);

        let items_az = flatten_tree(&tree, &instances, SortOrder::AZ);
        let child_names_az: Vec<_> = items_az
            .iter()
            .skip(1)
            .filter_map(|i| match i {
                Item::Group { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(child_names_az, vec!["alpha", "zeta"]);

        let items_za = flatten_tree(&tree, &instances, SortOrder::ZA);
        let child_names_za: Vec<_> = items_za
            .iter()
            .skip(1)
            .filter_map(|i| match i {
                Item::Group { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(child_names_za, vec!["zeta", "alpha"]);
    }

    #[test]
    fn test_sort_az_is_case_insensitive() {
        let mut inst1 = Instance::new("z-session", "/tmp/z");
        inst1.group_path = "Zebra".to_string();
        let mut inst2 = Instance::new("a-session", "/tmp/a");
        inst2.group_path = "apple".to_string();
        let instances = vec![inst1, inst2];
        let tree = GroupTree::new_with_groups(&instances, &[]);

        let items = flatten_tree(&tree, &instances, SortOrder::AZ);
        let group_names: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Group { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(group_names, vec!["apple", "Zebra"]);
    }

    #[test]
    fn test_existing_groups_vec_order_preserved_on_load() {
        let gamma_group = Group::new("gamma", "gamma");
        let alpha_group = Group::new("alpha", "alpha");
        let existing_groups = vec![gamma_group, alpha_group];

        let instances: Vec<Instance> = vec![];
        let tree = GroupTree::new_with_groups(&instances, &existing_groups);

        let roots = tree.get_roots();
        let root_names: Vec<_> = roots.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(root_names, vec!["gamma", "alpha"]);

        let all_groups: Vec<_> = tree
            .get_all_groups()
            .into_iter()
            .map(|g| g.name.as_str().to_string())
            .collect();
        assert_eq!(all_groups, vec!["gamma".to_string(), "alpha".to_string()]);
    }

    #[test]
    fn test_manual_sort_preserves_session_and_group_insertion_order() {
        let mut inst1 = Instance::new("ungrouped-z", "/tmp/u1");
        let mut inst2 = Instance::new("work-second", "/tmp/w2");
        inst2.group_path = "work".to_string();
        let mut inst3 = Instance::new("personal-only", "/tmp/p1");
        inst3.group_path = "personal".to_string();
        let mut inst4 = Instance::new("work-first", "/tmp/w1");
        inst4.group_path = "work".to_string();
        inst1.created_at = inst2.created_at;
        inst3.created_at = inst2.created_at;
        inst4.created_at = inst2.created_at;

        let instances = vec![inst1, inst2, inst3, inst4];
        let tree = GroupTree::new_with_groups(&instances, &[]);
        let items = flatten_tree(&tree, &instances, SortOrder::Manual);

        let labels: Vec<String> = items
            .iter()
            .map(|item| match item {
                Item::Group { path, .. } => format!("group:{path}"),
                Item::Session { id, .. } => {
                    let title = instances
                        .iter()
                        .find(|inst| &inst.id == id)
                        .map(|inst| inst.title.as_str())
                        .unwrap();
                    format!("session:{title}")
                }
                Item::ProfileHeader { name, .. } => format!("profile:{name}"),
            })
            .collect();

        assert_eq!(
            labels,
            vec![
                "session:ungrouped-z",
                "group:work",
                "session:work-second",
                "session:work-first",
                "group:personal",
                "session:personal-only",
            ]
        );
    }

    #[test]
    fn test_move_group_reorders_root_siblings_and_preserves_subtree() {
        let mut inst1 = Instance::new("work-session", "/tmp/work");
        inst1.group_path = "work".to_string();
        let mut inst2 = Instance::new("work-child-session", "/tmp/work/child");
        inst2.group_path = "work/projects".to_string();
        let mut inst3 = Instance::new("personal-session", "/tmp/personal");
        inst3.group_path = "personal".to_string();
        let instances = vec![inst1, inst2, inst3];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        assert!(tree.move_group("personal", -1));

        let items = flatten_tree(&tree, &instances, SortOrder::Manual);
        let groups: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                Item::Group { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(groups, vec!["personal", "work", "work/projects"]);

        let saved_groups: Vec<_> = tree
            .get_all_groups()
            .into_iter()
            .map(|group| group.path)
            .collect();
        assert_eq!(saved_groups, vec!["personal", "work", "work/projects"]);
    }

    #[test]
    fn test_group_serialization_with_default_directory() {
        let mut group = Group::new("work", "work");
        group.default_directory = Some("/home/user/project".to_string());

        let json = serde_json::to_string(&group).unwrap();
        let deserialized: Group = serde_json::from_str(&json).unwrap();

        assert_eq!(
            deserialized.default_directory,
            Some("/home/user/project".to_string())
        );
        assert_eq!(deserialized.name, "work");
        assert_eq!(deserialized.path, "work");
    }

    #[test]
    fn test_set_and_get_default_directory() {
        let mut inst = Instance::new("test", "/tmp/t");
        inst.group_path = "work".to_string();
        let instances = vec![inst];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        assert_eq!(tree.get_default_directory("work"), None);

        tree.set_default_directory("work", "/home/user/project");
        assert_eq!(
            tree.get_default_directory("work"),
            Some("/home/user/project")
        );

        // Setting on nonexistent group is a no-op
        tree.set_default_directory("nonexistent", "/tmp");
        assert_eq!(tree.get_default_directory("nonexistent"), None);
    }

    #[test]
    fn test_get_group_directories() {
        let mut inst1 = Instance::new("test1", "/tmp/1");
        inst1.group_path = "work".to_string();
        let mut inst2 = Instance::new("test2", "/tmp/2");
        inst2.group_path = "personal".to_string();
        let instances = vec![inst1, inst2];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        tree.set_default_directory("work", "/home/user/work");

        let dirs = tree.get_group_directories();
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs.get("work").unwrap(), "/home/user/work");
        assert!(!dirs.contains_key("personal"));
    }

    #[test]
    fn test_backward_compatibility_loading_groups_without_default_directory() {
        let json = r#"{"name":"work","path":"work","collapsed":false}"#;
        let group: Group = serde_json::from_str(json).unwrap();

        assert_eq!(group.name, "work");
        assert_eq!(group.path, "work");
        assert!(!group.collapsed);
        assert_eq!(group.default_directory, None);
    }

    #[test]
    fn test_rename_group_simple() {
        let mut inst = Instance::new("test", "/tmp/t");
        inst.group_path = "work".to_string();
        let instances = vec![inst];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        tree.rename_group("work", "projects").unwrap();

        assert!(!tree.group_exists("work"));
        assert!(tree.group_exists("projects"));
        assert_eq!(
            tree.groups_by_path.get("projects").unwrap().name,
            "projects"
        );
    }

    #[test]
    fn test_rename_group_with_children() {
        let mut inst1 = Instance::new("test1", "/tmp/1");
        inst1.group_path = "work".to_string();
        let mut inst2 = Instance::new("test2", "/tmp/2");
        inst2.group_path = "work/frontend".to_string();
        let instances = vec![inst1, inst2];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        tree.rename_group("work", "projects").unwrap();

        assert!(!tree.group_exists("work"));
        assert!(!tree.group_exists("work/frontend"));
        assert!(tree.group_exists("projects"));
        assert!(tree.group_exists("projects/frontend"));
    }

    #[test]
    fn test_rename_group_merge_into_existing() {
        let mut inst1 = Instance::new("test1", "/tmp/1");
        inst1.group_path = "old".to_string();
        let mut inst2 = Instance::new("test2", "/tmp/2");
        inst2.group_path = "existing".to_string();
        let instances = vec![inst1, inst2];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        tree.rename_group("old", "existing").unwrap();

        assert!(!tree.group_exists("old"));
        assert!(tree.group_exists("existing"));
    }

    #[test]
    fn test_rename_group_noop_same_path() {
        let mut inst = Instance::new("test", "/tmp/t");
        inst.group_path = "work".to_string();
        let instances = vec![inst];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        tree.rename_group("work", "work").unwrap();

        assert!(tree.group_exists("work"));
    }

    #[test]
    fn test_rename_group_noop_empty_target() {
        let mut inst = Instance::new("test", "/tmp/t");
        inst.group_path = "work".to_string();
        let instances = vec![inst];
        let mut tree = GroupTree::new_with_groups(&instances, &[]);

        // Empty target should fail validation
        assert!(tree.rename_group("work", "").is_err());

        assert!(tree.group_exists("work"));
    }
}
