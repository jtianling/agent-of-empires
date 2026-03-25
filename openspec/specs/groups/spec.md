# Capability Spec: Session Groups

**Capability**: `groups`
**Created**: 2026-03-06
**Status**: Stable

## Overview

Groups organize sessions into a hierarchical tree in the TUI. They use slash-delimited paths
(e.g., `work/clients/acme`) and support collapsing/expanding to keep the session list manageable.
Groups are persisted per profile in `groups.json`.

## Key Entities

### Group

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Display name (last path segment) |
| `path` | `String` | Full slash-delimited path (e.g. `work/clients`) |
| `collapsed` | `bool` | Whether the group is collapsed in the TUI |
| `default_directory` | `Option<String>` | Default project directory for new sessions in this group |

The `default_directory` field SHALL be serialized in `groups.json` with `#[serde(default)]` for backward compatibility.

### GroupTree

Manages the full group hierarchy. Built from:
1. Existing groups loaded from `groups.json` (preserves save order)
2. Groups implied by session `group_path` fields (auto-created if missing)

## Path Conventions

- Paths use `/` as separator
- Intermediate groups are auto-created (e.g., adding a session to `work/clients/acme`
  automatically creates `work`, `work/clients`, and `work/clients/acme` groups)
- Groups can exist without sessions (empty containers)

## Session Assignment

Sessions are assigned to groups via the `group_path` field on `Instance`. Setting
`group_path = "work/clients"` places the session under the `work/clients` group.

## TUI Behavior

- Groups appear as collapsible rows in the session list
- Collapsed groups hide their child sessions and sub-groups
- The `GroupTree::flatten_tree()` function produces a flat list of `Item` (Group or Session)
  for rendering, respecting collapse state
- Sort order applies within groups (sessions sorted by created_at, a-z, etc.)

## `flatten_tree` Output

```
Item::Group("work")              ← can be collapsed
  Item::Group("work/clients")
    Item::Session(...)           ← session under work/clients
  Item::Session(...)             ← session directly under work
Item::Session(...)               ← ungrouped session (top level)
```

## Requirements

### Requirement: Group entity has default_directory field
The Group entity includes an optional `default_directory` field that stores the default project directory for new sessions in this group.

#### Scenario: Group with default_directory serializes and deserializes
- **WHEN** a group has `default_directory` set to `/home/user/project`
- **AND** the group is saved to `groups.json`
- **THEN** the JSON SHALL include the `default_directory` field
- **AND** loading the same JSON SHALL restore the `default_directory` value

### Requirement: Manual ordering within the session tree
AoE SHALL provide a `Manual` sort mode for the home session list. In `Manual` mode, `Shift+K`
and `Shift+J` SHALL reorder the currently selected item while preserving the existing session-to-group
assignment.

#### Scenario: Reorder a session within its current group
- **WHEN** the home screen sort mode is `Manual` and the selected row is a session
- **AND** the user presses `Shift+K` or `Shift+J`
- **THEN** AoE SHALL move that session up or down among the sessions that share the same `group_path`
- **AND** AoE SHALL NOT move that session into a different group

#### Scenario: Reorder a group among sibling groups
- **WHEN** the home screen sort mode is `Manual` and the selected row is a group
- **AND** the user presses `Shift+K` or `Shift+J`
- **THEN** AoE SHALL move that group up or down among groups with the same parent path
- **AND** AoE SHALL preserve the group's child sessions and descendant groups

#### Scenario: Persist manual ordering
- **WHEN** a session or group is reordered in `Manual` mode
- **THEN** AoE SHALL persist the updated order to profile storage
- **AND** the same order SHALL be restored after reloading the TUI

### Requirement: Status bar shows accurate keybinding hints
The tmux status bar SHALL display `Ctrl+b 1-9 space jump` instead of `Ctrl+b 1-9 jump` to
accurately reflect the Space confirmation step. The `Ctrl+b n/p switch` hint SHALL be removed
from the status bar.

#### Scenario: Status bar after attach
- **WHEN** a session is attached and the status bar is configured
- **THEN** the status-left SHALL contain `Ctrl+b 1-9 space jump`
- **AND** the status-left SHALL NOT contain `n/p`

### Requirement: Groups can be renamed with cascading updates
AoE SHALL support renaming a group by changing its full path. Renaming a group SHALL cascade path updates to all descendant groups and all sessions whose `group_path` matches or descends from the old path. When the target path already exists, AoE SHALL offer to merge the groups.

#### Scenario: Rename a leaf group
- **WHEN** a group at path `work/frontend` is renamed to `work/ui`
- **THEN** the group path SHALL be updated to `work/ui`
- **AND** all sessions with `group_path = "work/frontend"` SHALL be updated to `group_path = "work/ui"`

#### Scenario: Rename a parent group cascades
- **WHEN** a group at path `work` is renamed to `projects`
- **AND** descendant groups `work/frontend` and `work/backend` exist
- **THEN** descendant paths SHALL update to `projects/frontend` and `projects/backend`
- **AND** all sessions under the old paths SHALL have their `group_path` updated accordingly

## Functional Requirements

- **FR-001**: Intermediate groups MUST be auto-created when a session is assigned to a deep path.
- **FR-002**: Groups MUST persist their collapse state across TUI restarts.
- **FR-003**: Groups are per-profile: switching profiles shows a different group tree.
- **FR-004**: Deleting a group MUST reassign its sessions to the parent group or top level.
- **FR-005**: Session sort order MUST apply within groups (not just at the top level).
- **FR-006**: Empty groups (no sessions, no children) SHOULD be displayed but can be deleted.
- **FR-007**: `GroupTree` construction MUST preserve the prior save order for groups to avoid unexpected reordering.

## Success Criteria

- **SC-001**: Sessions in nested groups are visually indented in the TUI.
- **SC-002**: Collapsing a group hides all descendant sessions.
- **SC-003**: Adding a session to `a/b/c` automatically creates groups `a`, `a/b`, `a/b/c`.
- **SC-004**: Group collapse state survives application restart.
