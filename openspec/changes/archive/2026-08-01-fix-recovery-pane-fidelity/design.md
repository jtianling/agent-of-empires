## Context

`build_base_pane_command` branches on whether a pane is the primary one:

```rust
if !is_primary {
    // the slot's own agent binary, plus a resume flag when one applies
}
// primary: self.get_tool_command(), plus extra_args, --session-id, fork template
```

The primary branch reaches for instance state because the primary pane is normally the instance's own agent: the command override, the pre-allocated `--session-id`, the pending fork token and `extra_args` all describe that one agent.

Adoption breaks that assumption. Agent tracking is observe-first, so a user can split a pane, start an agent by hand, and have it adopted into a slot without AoE ever launching it. The instance's tool then describes nothing about slot 0. In the reported case the instance tool was a shell and both slots recorded `claude`; recovery relaunched slot 0 as a shell and slot 1 correctly as `claude`.

This was known. `add_and_start` in the e2e suite carries the comment "The instance tool must match slot 0's recorded agent: recovery rebuilds the primary pane's resume command from `self.tool` (`get_tool_command()`)", and every recovery test satisfies that constraint. The behavior was documented and designed around rather than fixed, which is why the whole recovery test surface is blind to it.

Separately, recovery decides success from what it saw while launching. `resume_launch_pane` reports a pane it could not build a command for or could not respawn, and `rebuild_recovery_panes` reports a pane it could not create. A pane that is created, respawned successfully, and then disappears falls through all of it. In the reproduction the primary pane was alive 150 ms after recovery and gone by 200 ms, well after recovery had returned success.

## Goals / Non-Goals

**Goals:**

- Relaunch each tracked pane as the agent its slot recorded.
- Keep instance-level launch concepts attached to the agent they describe.
- Make recovery say when it did not give a slot back.
- Make a shell-tool instance with adopted agent slots reachable by tests.

**Non-Goals:**

- Fix the `pane-died` hook, which runs `set-option -p remain-on-exit off ; respawn-pane` with no `-t` target and therefore acts on tmux's current pane rather than the pane that died. It is a session-scoped hook with a blast radius well beyond recovery and is being handled separately.
- Fix the loss of adopted panes to kill-then-respawn, which have no `remain-on-exit` because AoE never created them. This change surfaces that loss; it does not prevent it.
- Repair a slot whose pane did not survive. Reporting is the goal; a retry would mask whatever removed it.
- Change what a non-primary pane runs today, which is already correct.

## Decisions

### Decision 1: Instance-primary treatment requires both the position and the agent

A pane gets the instance-primary command construction only when it is slot 0 **and** the agent its slot recorded is the instance's own tool. Otherwise it is built from that agent's binary, exactly as a secondary pane already is.

Position alone is not enough, because it asks "is this slot 0" when the thing that also matters is "is this the agent the instance describes". A command override, a pre-allocated conversation id and a pending fork all belong to the instance's tool; handing them to a different agent that happens to occupy slot 0 is what produced a shell where an agent belonged.

The agent match alone is not enough either, and this is the less obvious half. Two slots can record the same agent -- a user running `claude` in both halves of a split is the ordinary case. Treating every matching slot as instance-primary would hand the instance's single pre-allocated conversation id, pending fork token and xats identity key to more than one pane at once, so two panes would claim one conversation and one identity. Those values are singular because the instance has one primary agent, and slot 0 is what names it.

Alternative considered: keep the position test and special-case a shell tool. Rejected because the mismatch is not specific to shells. An instance whose tool is `claude` with a slot 0 that records `codex` has the same problem, and the position test would still get it wrong.

### Decision 2: Verify slots against live panes after the rebuild settles

Once every slot has been launched and the layout applied, recovery lists the session's live panes and reports any slot whose pane is not among them.

The check has to come after a brief settle, not immediately: in the reproduction the pane survived its own relaunch and disappeared roughly 200 ms later, so an immediate check would confirm exactly the state that is about to stop being true. Recovery is a one-shot user-initiated action that already spends far longer rebuilding the session, so a short wait before the final verdict is affordable in a way it would not be on a polling path.

This deliberately does not retry or repair. Recovery's job here is to be honest about what came back.

### Decision 3: The reported failure names the slot, not just the pane

A missing pane is reported against its slot with the agent and directory the slot recorded, because that is what the user recognizes. A bare pane id names something that no longer exists and that the user never saw.

The same report is also appended to the instance's event log as a `lost` row, next to the `adopt`/`capture` entries that recorded the slot in the first place. This goes beyond the required per-pane failure: the outcome vector is in-memory and the TUI's error field is cleared by the next status poll, so a durable row is what lets the user (and a test) see afterwards that recovery said something.

### Decision 4: Cover the shape the helper avoids

The suite gains a way to build an instance whose tool is a shell with slots recording a different agent. Every existing recovery test is built the other way by construction, so without this the fix has no test that could fail.

## Risks / Trade-offs

- [Instance-primary concepts stop applying to a mismatched slot 0] -> That is the fix. A `--session-id` or fork token belonging to the instance's tool was never meaningful for a different agent, and passing it produced the reported failure.
- [The settle before verification slows recovery] -> Bounded and one-shot, on an action that already rebuilds a tmux session and relaunches every pane.
- [A pane that dies later than the settle still goes unreported] -> Accepted. The check narrows the window rather than closing it; the underlying causes are tracked separately.
- [`expects_shell` remains instance-scoped] -> It still describes the instance's own tool, which is correct for the wrap it guards. The branch it gates sits behind the command override, and a mismatched pane no longer takes that branch at all, so the instance-scoped answer cannot reach a pane it does not describe.

## Migration Plan

No data or schema change. Behavior applies at the next restart or recovery.

## Open Questions

- None blocking. Whether `expects_shell` should follow the pane's agent rather than the instance's tool is worth revisiting once the `pane-died` hook work lands, since both touch how a non-agent pane is treated.
