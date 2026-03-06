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
