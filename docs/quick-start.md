# Quick Start

## Launch the TUI

```bash
aoe
```

This opens the dashboard. You'll see an empty session list on first run.

## Create Your First Session

**From the TUI:** Press `n` to open the new session dialog. Fill in the path to your project (or leave it as `.` for the current directory) and press `Enter`.

**From the CLI:**

```bash
aoe add /path/to/project
```

The session appears in the dashboard with status **Idle**.

## Attach to a Session

Select a session and press `Enter` to attach. You're now inside a tmux session running your AI agent (Claude Code by default).

To return to the TUI, press **`Ctrl+b d`** (the standard tmux detach shortcut).

## Use the Terminal View

Press `t` to toggle between Agent View and Terminal View. Each agent session has a paired shell terminal where you can run builds, tests, and git commands without interrupting the agent.

## Review Changes with Diff View

Press `D` to open the diff view. This shows changes between your working directory and the base branch. Navigate files with `j`/`k`, press `e` to edit, and `Esc` to close.

## Create a Worktree Session

To work on a new branch with its own directory:

```bash
# CLI
aoe add . -w feat/my-feature -b

# TUI: press n, fill in the worktree branch field
```

This creates a new git branch, a worktree directory, and a session pointing at it. When you delete the session, AoE offers to clean up the worktree too.

## Create a Sandboxed Session

To run an agent inside a Docker container:

```bash
aoe add --sandbox .
```

In the TUI, toggle the sandbox checkbox when creating a session. The agent runs in an isolated container with your project mounted at `/workspace` and authentication credentials shared via persistent Docker volumes.

Requires Docker to be installed.

## Choose a Different Agent

By default, AoE uses Claude Code. To use a different tool:

```bash
aoe add -c opencode .   # or any other supported agent
```

In the TUI, select the tool from the dropdown in the new session dialog.

## Run Two Agents Side by Side

The new session dialog's **Right Pane** field launches a second agent beside the
first one.  Once a right pane tool is selected, a **Right Pane Path** field
appears below it: the directory that agent starts in.

- Leave it empty and the right pane follows the session.  This is resolved when
  the pane is created, not when you submit the dialog, so a worktree-backed
  session carries the right pane into the resolved worktree along with the left
  pane.
- Fill it in and the right pane starts there instead.  The directory is used as
  given -- it is not worktree-resolved, because it is not the session's
  repository.  A reviewer in a sibling checkout or a tester in a scratch tree
  are what this is for.
- If either directory does not exist yet, the dialog asks once, naming every
  directory it would create.  Declining creates none of them.
- Sandboxed sessions do not offer the field.  Inside a container the agent's
  directory is decided by the container exec, so a host-side directory would be
  accepted and then have no effect.

The session's own **Path** is unchanged by any of this.  It stays the session's
anchor: the worktree base, the group default directory, the sandbox mount root.
Only the pane's working directory is per-pane.

The fork dialog offers the same field for the forked session's right pane, where
an empty value means the parent's directory.

## Add an Agent Pane to a Running Session

Press `%` on a selected running session to add another managed agent pane.  A
small dialog offers the agent (defaulting to the session's own tool) and the
working directory (defaulting to the session's own); AoE attaches once the pane
is up.

`%` refuses rather than works around its limits: a session that is not running
is reported instead of started, and a session that already has four panes is
reported instead of grown.

> `%` on the home screen is not the same as `Ctrl+b %` inside an attached
> session.  See the note under the keyboard reference below.

The CLI equivalent, which does not attach:
```bash
aoe session add-agent-pane <session> --tool codex --path /path/to/other
```

## TUI Keyboard Reference

| Key | Action |
|-----|--------|
| `n` | New session |
| `%` | Add an agent pane to the selected session, then attach |
| `Enter` | Attach to session |
| `d` | Delete session |
| `t` | Toggle Agent/Terminal view |
| `D` | Open diff view |
| `/` | Search sessions |
| `?` | Show help |
| `q` | Quit |
| `Ctrl+b d` | Detach from tmux session |

### `%` on the home screen vs `Ctrl+b %` while attached

The two look alike and their directory rules deliberately point in opposite
directions:

| Trigger | Where | Result | Directory |
|---------|-------|--------|-----------|
| `%` | home screen | a managed pane: AoE launches what you picked and tracks it | chosen in the dialog, defaulting to the session's |
| `Ctrl+b %` | attached to a session | a plain tmux pane: nothing launched, nothing tracked | always the session's project path |

What "tracked" means depends on what you picked:

- An agent gets a durable slot, so a restart brings it back as that agent in its
  own directory.  On a Cross Agent Team session it is also given its own identity
  key; otherwise there is no key to mint.
- A `shell` gets a slot only when you gave it a directory of its own, and never
  gets an identity key.  A shell that simply followed the session needs no
  record, because a restart would put it back there anyway.

A hand-made split has no interface through which to name a directory, so
inheriting the session's is the only useful behavior it can have.  A managed
pane is created through a dialog, which is such an interface.  The distinction
is whether the pane was given a chance to be configured, not whether you were
attached.

## Next Steps

- [Workflow Guide](guides/workflow.md) -- recommended setup with bare repos and parallel agents
- [Docker Sandbox](guides/sandbox.md) -- container configuration and custom images
- [Repo Config & Hooks](guides/repo-config.md) -- per-project settings
- [CLI Reference](cli/reference.md) -- every command and flag
