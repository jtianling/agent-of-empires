## Context

`recover_from_slots` rebuilds a dead session from its durable slots:

1. `start_with_size` recreates the tmux session, which also starts its first pane with `build_agent_command(None)`.
2. `rebuild_recovery_panes` pairs slot 0 with that pane and splits a placeholder shell for each remaining slot.
3. The saved layout is remapped onto the new pane ids.
4. Every slot goes through `resume_launch_pane`, which kills the pane's process tree and respawns it with that slot's command.

Step 4 is the authoritative launch. Step 1's agent launch exists only because session creation happens to take a command, and the code comment already describes it as launched fresh and then uniformly relaunched.

That throwaway launch is where recovery breaks. For the primary pane, `build_agent_command` injects the instance's pre-allocated `--session-id`, and during recovery that id names the conversation being recovered, which already exists. A real agent refuses to start and exits immediately. With a single-pane session, the pane's death takes the session with it, so step 4 respawns into nothing: `can't find pane: %N`.

The manual acceptance run reproduced this with Claude on both `R` and `C`, and a local run reproduced it with a stub that exits immediately, on `R`. The shared rebuild is the cause; the restart mode is irrelevant.

The reason this survived so long is the test stub. `install_tool_stub` writes `exec sleep 2147483647`, so every recovery test in the suite runs an agent that cannot die. The failure needs a process that exits, which no test could produce.

## Goals / Non-Goals

**Goals:**

- Launch each recovered agent exactly once, from the loop that already decides what each slot should run.
- Keep everything else session rebuild does: worktree and sandbox setup, on-launch hooks, tmux options, pane pinning.
- Make the failure class reachable by tests.

**Non-Goals:**

- Change what the per-slot launch runs, in either restart mode.
- Change the normal (non-recovery) start path, which launches the agent once and correctly.
- Make an agent that legitimately fails to start look like a success. A pane whose agent exits on its own is still a dead pane; this change only stops recovery from destroying the session before the real launch happens.

## Decisions

### Decision 1: Rebuild with a placeholder shell

Session creation already accepts an optional command, and passing none starts the default shell. Recovery uses that, so the rebuilt primary pane is a placeholder exactly like the panes `rebuild_recovery_panes` splits for the other slots, which the existing comment already describes as running the shell until the resume flow respawns them.

This makes the rebuild uniform: every recovered pane starts as a placeholder and every agent is launched once, by the per-slot loop.

The alternative was to keep the agent launch and give it a usable identity, for example by allocating the fresh session id before the rebuild in fresh mode. Rejected: it only helps the fresh path (resume mode has no new id to allocate and would still relaunch a conversation that is already open), and it keeps two launches racing over one pane. Launching once is simpler than making a redundant launch survivable.

### Decision 2: A placeholder rebuild leaves the pending fork token alone

The normal start path clears `fork_pending` after creating the session, because the agent it just launched consumes the fork. A placeholder rebuild launches no agent, so clearing the token would drop a fork that never happened. The token is left for the per-slot launch to use.

### Decision 3: A placeholder rebuild does not run agent auto-confirmation

`run_auto_confirm` polls the pane for an agent's startup prompts. During a placeholder rebuild the pane holds a shell, so the poll can only waste its timeout. Recovery already calls it once at the end, after the real launches.

### Decision 4: Cover the failure with an agent that exits immediately

The suite gains a stub that exits instead of sleeping forever, and cold recovery is covered with it in both restart modes. Without it the regression is invisible to tests, which is precisely how this defect reached users.

The assertion is that the session survives and each slot's pane is respawned, not that the agent stays running: a stub that exits will exit again after the authoritative relaunch too. What must hold is that recovery reaches that relaunch at all.

## Risks / Trade-offs

- [The rebuilt pane briefly holds a shell instead of the agent] -> It is replaced within the same recovery call, before the user is attached, and this is already how every non-primary recovered pane behaves.
- [A placeholder pane outlives a failed per-slot launch] -> The slot's error is already surfaced per pane, and a shell is a better end state than a pane that vanished with its session.
- [Skipping fork-token clearing leaks a stale token if recovery aborts] -> The token is only consumed by an actual launch; leaving it matches the pre-rebuild state rather than inventing a new one.
- [The new stub makes other tests flaky if reused carelessly] -> It is opt-in per test, and the immortal stub stays the default.

## Migration Plan

No data or schema change. The fix takes effect for the next recovery.

## Open Questions

- None blocking.
