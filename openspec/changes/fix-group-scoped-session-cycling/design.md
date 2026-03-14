## Context

AoE currently installs tmux key bindings for nested attach flows in `src/tmux/utils.rs`. The
attach path stores per-client metadata for three separate concerns:

- the attach-origin profile used by `Ctrl+b j/k`
- the attach-origin AoE session used by `Ctrl+b d`
- the last managed session visited so the TUI can restore selection after re-entry

The current cycle implementation builds one ordered session list for the entire profile by
flattening the group tree. That means a user attached from `skills-manager` can cycle into
`blog-workspace` simply because both sessions live in the same profile order. Once the client is in
an unrelated session, detach still uses the original return session, but the user experience is
already broken because grouping boundaries were ignored.

## Goals / Non-Goals

**Goals:**
- Make nested tmux cycling honor the current session's exact `group_path`
- Keep ungrouped sessions isolated from grouped sessions during `Ctrl+b j/k`
- Preserve the existing attach-origin AoE return target for `Ctrl+b d`
- Preserve TUI detached-session restoration within the now-restricted cycle scope

**Non-Goals:**
- Changing standalone attach behavior when AoE is not running inside tmux
- Redefining group hierarchy semantics outside tmux session cycling
- Adding new persistent config for tmux key behavior

## Decisions

### Resolve cycle scope from the current managed session

**Decision:** Determine the cycle scope by resolving the current tmux session name back to an
`Instance`, then use that instance's exact `group_path` as the filter for candidate sessions.

**Why:** The current session already tells us which group context the user is in. This avoids
threading extra group arguments through tmux bindings or storing another piece of client metadata.

**Alternative: store a per-client group scope on initial attach:** Rejected because it would freeze
the original group and prevent cycling logic from being derived from the actual current managed
session after the user moves within the allowed scope.

**Alternative: use parent-group or subtree scope:** Rejected because the reported bug and current
TUI mental model both treat a session's exact group row as the relevant boundary. Cycling across
sibling groups would still feel like a leak.

### Keep detach return state separate from detached-selection state

**Decision:** Continue using the existing `@aoe_return_session_<client>` value as the immutable
return target for `Ctrl+b d`, while still updating `@aoe_last_detached_session_<client>` on each
successful cycle target change.

**Why:** These two values represent different user expectations. Detach must always go back to the
AoE session that initiated attach, while TUI re-entry should highlight the actual managed session
the user detached from.

**Alternative: update the return session on each cycle:** Rejected because it would recreate the
original bug in a different form, allowing `Ctrl+b d` to bounce between managed sessions instead of
returning to AoE.

### Filter the ordered session list after applying TUI ordering rules

**Decision:** Reuse the existing flattened ordering logic, then restrict the resulting session list
to the current scope before resolving next/previous targets.

**Why:** This preserves sort order, manual ordering, and collapsed-group visibility rules that the
current spec already ties to cycling behavior.

**Alternative: build a separate per-group ordering path:** Rejected because it duplicates group
sorting logic and risks drifting from the TUI's visible ordering semantics.

## Risks / Trade-offs

- **Risk: current tmux session cannot be resolved back to a stored instance** → Mitigation: treat
  that as "no in-scope target" and leave the client where it is rather than guessing.
- **Risk: collapsed groups produce an empty candidate set** → Mitigation: preserve existing
  flatten-tree semantics so behavior stays aligned with the current visible TUI order.
- **Risk: hidden `aoe tmux switch-session` behavior becomes harder to reason about** → Mitigation:
  add focused unit tests for grouped and ungrouped scope resolution plus an e2e regression for
  nested detach after cycling.

## Migration Plan

No data migration is required. The change is runtime-only and affects tmux key binding behavior for
future attaches.

## Open Questions

None.
