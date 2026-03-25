## Context

`rename_selected()` in `src/tui/home/operations.rs` has two code paths that rename tmux sessions:

1. **Same-profile rename** (lines 322-344): reads `old_title`, mutates instance, then tries to build a `Session` from the mutated instance to call `tmux rename-session`. The Session is constructed with the NEW title, so `session.exists()` returns false (tmux still has the old name), and the rename is skipped.

2. **Cross-profile rename** (lines 280-294): mutates `instance.title` first, then checks `orig_inst.title != effective_title` using `self.get_instance()` which returns the already-mutated instance. The condition is always false, so rename is skipped.

In both cases, the tmux session keeps its old name while AoE stores the new title. The status poller generates `Session::generate_name(id, new_title)`, can't find the tmux session, and marks it as Error.

## Goals / Non-Goals

**Goals:**
- tmux session name stays in sync with AoE instance title after rename
- No process interruption during rename (tmux `rename-session` is a cosmetic operation)
- Both same-profile and cross-profile rename paths work correctly

**Non-Goals:**
- Changing the tmux session naming scheme
- Adding rollback if tmux rename fails (current warn-and-continue is fine)

## Decisions

### Capture old tmux Session before mutation

Build the `tmux::Session` object (or just the old tmux name string) from the old title BEFORE `mutate_instance` changes the title. Then use the old session to rename to the new name.

**Alternative considered**: rename tmux first, then mutate instance. Rejected because if tmux rename succeeds but instance mutation fails, we'd have a name mismatch in the other direction. Capturing the old session name is simpler and keeps the existing code structure.

### Unify the rename logic for both paths

Both the same-profile and cross-profile paths have the same bug pattern. Extract the tmux rename into a sequence: (1) build old session, (2) compute new name, (3) rename.

## Risks / Trade-offs

- [Race with status poller] Between `mutate_instance` and `tmux rename-session`, the poller could see the new title but the old tmux name. This window is very small (sub-millisecond) and the consequence is a single Error flash that self-corrects on the next poll. Acceptable.
