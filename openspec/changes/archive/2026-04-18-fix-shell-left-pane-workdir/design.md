## Context

When AoE creates a tmux session, the left pane command is built by `Instance::build_agent_command()` and wrapped via `wrap_command_ignore_suspend_with_env()`. The resulting command relies on tmux's `-c` flag to set the working directory:

```
tmux new-session -d -s {name} -c {project_path} "{shell} -lc 'stty susp undef; exec env {tool}'"
```

For code agents (Claude, Codex, etc.), the tool binary inherits the cwd from the login shell wrapper. For Shell sessions, the inner `exec env /bin/zsh` starts an interactive shell that sources `.zshrc` but inherits cwd from the outer login shell. If the outer login shell's profile (`.zprofile`, `.zlogin`) changes the cwd, all tools are affected, but code agents don't visibly show the discrepancy (they don't display `$PWD` in a prompt), while a raw shell makes it immediately obvious.

The right pane shell in `build_right_pane_command()` already handles this by prepending `cd {dir} &&` before `exec`:

```rust
format!("cd {} && stty susp undef; exec {}", escaped_dir, cmd)
```

## Goals / Non-Goals

**Goals:**
- Shell left-pane sessions start in the user-specified `project_path`
- Match the reliability pattern already proven in right pane shell code

**Non-Goals:**
- Fixing cwd for non-shell agents (they don't visibly exhibit the issue, and adding `cd` there would be a larger change with no user-visible benefit)
- Changing the right pane command (it already works)

## Decisions

### Add explicit `cd` in `build_agent_command` for shell sessions

Prepend `cd {escaped_project_path} &&` to the inner command when the tool is a shell, before passing to `wrap_command_ignore_suspend_with_env`.

**Why not modify `wrap_command_ignore_suspend_with_env` itself?** That function is shared by all tools (Claude, Codex, etc.). Adding a `cd` there would require passing `project_path` into the function signature, changing every call site. Since only shell sessions exhibit the issue visibly, scoping the fix to the shell path in `build_agent_command` is minimal and targeted.

**Why not add `cd` for all tools?** The issue is only user-visible for shell sessions. Adding it for all tools would be a larger change with no observable benefit and potential risk of breaking agent-specific startup behavior.

## Risks / Trade-offs

- [Risk] The explicit `cd` could fail if `project_path` doesn't exist at session start time (e.g., worktree deleted). → Mitigation: `build_instance` already validates path existence before creating the instance. The `cd` failure would surface as a shell error, same as the current `-c` flag behavior.
- [Trade-off] This fixes the symptom (shell cwd) rather than the root cause (login profiles changing cwd). → Acceptable: the right pane already uses this exact pattern successfully, and fixing user login profiles is out of scope.
