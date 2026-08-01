## 1. Relaunch as the Slot's Agent

- [x] 1.1 Decide instance-primary command construction by whether the slot's recorded agent is the instance's tool, instead of by slot position.
- [x] 1.2 Confirm a matching slot keeps today's behavior byte for byte: command override, pre-allocated conversation id, pending fork token, extra arguments.
- [x] 1.3 Confirm a mismatched slot builds from its own agent binary and carries none of the instance's conversation identity.

## 2. Report Slots That Did Not Come Back

- [x] 2.1 After launching every slot and applying the layout, verify each durable slot has a live pane in the rebuilt session.
- [x] 2.2 Let the rebuild settle before verifying, since a pane can survive its relaunch and disappear shortly after.
- [x] 2.3 Report a missing slot by the agent and working directory it recorded, and do not retry or recreate it.

## 3. Close the Coverage Gap

- [x] 3.1 Add a way to build an instance whose tool is a shell with slots recording a different agent, the shape `add_and_start` deliberately avoids, and say so in its doc comment.
- [x] 3.2 Cover recovery of that shape: every slot comes back as its recorded agent.
- [x] 3.3 Cover that a slot whose pane disappears is reported rather than silently dropped, verified red against the unfixed code.

## 4. Keep the Relaunched Pane Alive

- [x] 4.1 Hold a pane open across the process-tree kill that precedes its respawn, in the relaunch itself rather than at each site that creates a pane.
- [x] 4.2 Always write `remain-on-exit` on respawn instead of only when it must be on, so a pane relaunched as an agent after being created as a shell (or the reverse) ends up describing what now runs in it.
- [x] 4.3 Cover the production shape the existing shell test misses -- an instance whose command is a real shell, so `expects_shell()` is true -- with a single slot, where losing the pane loses the whole session. Verified red against the unfixed code.
- [x] 4.4 Take in independent acceptance coverage for the shapes the author's own test left out: three slots with layout, `remain-on-exit` describing each pane's own agent after the relaunch, and the clean (`C`) path at one and two slots. The defect reaches `C` as well, where a single-slot instance loses the whole session.

## 5. Verification

- [x] 5.1 Run `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo test --test e2e` with all tmux-touching tests confined to the project harness's private socket.
- [x] 5.2 Run `openspec validate fix-recovery-pane-fidelity --strict`.
