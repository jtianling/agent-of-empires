## Context

AoE's Cross Agent Team support currently covers launch mechanics only. For `claude` it appends `--dangerously-load-development-channels <channel>` and auto-confirms the startup screens. For `codex` it runs a pane-local bootstrap that generates a UUID, pre-registers `$TMUX_PANE` with it, and passes it as `xats.agent_id`. Neither carries a team or a name: the codex UUID is a short-lived nonce whose only job is to prove to the daemon which process the launcher started, and it is discarded after use.

The xats side was investigated with its maintainer before this change was written, and the design was reviewed by them afterwards. The findings that shape it:

- `reconnect` is not a separate recovery mechanism. It is a lookup followed by the ordinary registration call. Its three existing lookup keys (`runtime_ui_pid`, `thread_id`, and the opencode/kimi session pair) happen to be process-scoped, which is why none of them survives a restart. A lookup key that is stable across restarts makes the existing behavior work unchanged.
- Re-registering the same team and name is a takeover, not a conflict. The agent id is preserved, so ids other agents already hold stay valid, and the unread cursor is preserved. The previous connection is closed.
- Registering a *different* name is not an update. Identity is uniquely keyed by device, team, and name, so a rename inserts a new record with a new agent id and leaves the old record in place.
- For `claude`, passing the UI process id during that call hard-overwrites the pane, tty, and pid binding, so a restored identity points at the new pane rather than the dead one. For `codex` the UI process id is deliberately not passed, because doing so disables the launcher's pane pre-registration path.
- The UI process id can only be read by the agent itself from inside its own session. AoE cannot supply it. The recovery call therefore has to be made by the agent, and AoE's job ends at delivering the key.
- Looking an identity up by pane requires knowing the team first, and the stored pane value is only the last successful binding rather than a guarantee that the pane is alive. Discovering an identity after the fact is therefore not a reliable substitute for carrying one.

## Goals / Non-Goals

**Goals:**

- Let a pane that was relaunched without its conversation recover the xats identity it had before.
- Keep team and name ownership entirely on the xats side. AoE carries an opaque key and nothing else.
- Make the key durable across AoE process restart and machine reboot.
- Degrade to today's manual registration when the key cannot be resolved.

**Non-Goals:**

- Let AoE read, display, configure, or validate a team or a name.
- Add xats identity fields to the New Session dialog, the settings TUI, or profile overrides.
- Register on the agent's behalf.
- Change behavior for sessions that do not have Cross Agent Team enabled.
- Preserve identity across devices.

## Decisions

### Decision 1: AoE mints the identity key; the daemon does not

AoE generates an opaque value at first launch and treats it as write-once state for that slot. The agent presents it when it registers, and presents it again when it needs to recover.

The alternative was to let the identity keep the id the daemon already assigns and have AoE capture that id afterwards. Rejected on two counts. Capturing it requires a pane lookup that is scoped by team, which AoE deliberately does not know, so the lookup cannot even be issued. And capture would need a moment to happen: the user registers at an arbitrary point in the conversation, so AoE would have to poll, and at cold-start recovery time the stored pane on that row is almost always a dead one.

Minting removes both problems. There is nothing to capture and no timing to get right, because the key exists before the agent does.

### Decision 2: The key is per slot, not per instance

A single session routinely runs different agents in different panes, and those agents hold different xats identities. Storing one key per instance would let two panes recover into the same identity, which the daemon resolves by letting the later caller take over and silently disconnecting the earlier one.

Each pane therefore gets its own key. Where that key is stored differs by the pane's role, which Decision 3 covers.

A consequence worth stating, because it will be asked about: the identity follows the slot, not the tool. If a slot that ran `claude` is later changed to run `codex`, the new tool presents the same key and recovers the same team and name. The daemon overwrites the agent type and delivery details while keeping the identity and its agent id. This is the intended behavior. To everyone else on the team that pane is the same collaborator, and swapping the underlying tool should not rename it.

### Decision 3: The key is stored where the rest of that pane's identity already lives

Storage follows the split the codebase already has. The instance record holds the state describing the primary agent: its pre-allocated session id, its resume token, its pending fork. The primary pane's identity key is the same kind of fact and lives there. Adopted panes are described by durable slot records, so their keys live on those.

The alternative was a uniform launch-time association keyed by the tmux pane, which the reconciler would copy into the slot record. It was written into an earlier draft of this design and then dropped, because the problem it solved does not exist:

- The only pane AoE launches before a slot record exists is the primary one, and the instance record is already the right home for it.
- AoE launches an adopted pane only during restart or recovery, when that pane's slot record is already present and can be read and written directly.

Keeping the association would have added a table, a reconcile hop, and an orphan-row sweep to serve a case with no instances.

What reconcile must still do is narrower: a pane capture carries no identity key, so rewriting a slot from a capture must preserve the key already on the slot instead of blanking it.

### Decision 4: Panes AoE never launched get their key one relaunch later

Agent panes are adopted observe-first. A user can split a pane, start an agent in it by hand, and the reconciler will adopt it into a slot; AoE never builds that pane's command. There is no way to give a running process an environment variable it did not start with, and tmux's environment is session-scoped rather than pane-scoped, so a session-level variable would hand every pane the same key and produce exactly the collision Decision 8 exists to prevent.

So AoE mints and injects only where it builds the command. For a hand-started pane that is the first restart or recovery of its slot. Identity continuity for such a pane therefore begins one relaunch later than for the primary pane, which has a key from its very first launch.

This is a real limitation rather than a temporary one, and it is worth stating plainly because the symptom is confusing otherwise: the first clean restart of a hand-started agent still asks the user to register, and every one after it does not.

### Decision 5: Injection is unconditional, not restart-specific

Whenever Cross Agent Team is enabled for a pane, the key is injected. It is injected on the very first launch, on resume, on clean restart, and on both recovery modes.

Making injection conditional on a fresh mode would be wrong in an obvious way and a subtle way. Obvious: the agent must present the key at its very first registration, or the daemon has nothing to associate it with, and the first launch is never a fresh restart. Subtle: a conditional injection puts identity logic on the restart paths, when the restart paths should stay unaware that identity exists.

The consequence is that this change adds no code to any restart or recovery path. It adds a stored value, an environment variable on the launch command, and one rule about cloning.

### Decision 6: The environment variable is `XATS_IDENTITY_KEY`

The name is part of the cross-project contract and is fixed before implementation, because changing it after minting begins invalidates every key already issued.

It deliberately avoids the word "token". The xats project already uses `XATS_TOKEN` for the daemon's bearer credential, and that variable is present in the same launcher shell. Two variables differing by one word while meaning unrelated things invites a specific and ugly failure: exporting the bearer value as the identity value would give every pane on the device the same value, so every pane would recover into a single identity and fight over it, with every individual step looking correctly configured.

Within AoE the same value is stored as `xats_identity_key`, so the concept reads the same in the database, the code, and the environment.

### Decision 7: Environment variable, not a command-line argument

The identity key is a weak credential. Anything that can read it can ask the daemon for that identity.

Its practical exposure is bounded: the daemon listens on loopback and is already protected by a shared bearer credential that any same-user process could read, so a local attacker able to see the identity key could already impersonate any agent directly. The key adds no new attack surface, and scoping identity lookup to the local device means a key carried to another machine is inert.

Within that bound the cheap precaution is still worth taking. Command-line arguments are visible to every process on the machine through the process table; environment variables are visible only to the same user. The key goes in the environment, stays out of logs, and stays out of anything committed. No encryption and no rotation.

### Decision 8: Cloning mints a fresh key; it is never copied

New-from-selection and fork produce a new pane that must not inherit the source pane's identity. Copying the key would produce two panes claiming one identity, and the daemon resolves that by letting the newer caller take over and closing the older connection. The symptom is one agent silently no longer receiving messages, with evidence only in the daemon log.

This guard carries more weight than its size suggests, and the reason is worth stating plainly: **it is the only place the failure can be prevented at all.** On the daemon side, "pane A restarting and recovering its own identity" and "pane B presenting a copied key to take over A's identity" are the same call with the same arguments. The registration-time conflict rule described under External Dependency is a second net that only catches the case where both panes are alive at registration time; nothing downstream can catch a key that was copied and then bound cleanly.

AoE is well placed to build this guard precisely because the key lives in AoE's own storage rather than being derived from anything the agent controls.

### Decision 9: A key that does not resolve is a normal state

A daemon database can be reset, an agent can unregister itself, and a row can be deleted. In all of these the key still exists on AoE's side but resolves to nothing, and the agent is told to register normally.

AoE must not treat this as an error, must not clear the stored key, and must not surface a failure. The user simply registers the way they do today. Clearing the key on a failed resolution would be actively harmful: the identity may be restored later, and the key is the only thing that could reconnect to it.

### Decision 10: The codex nonce and the identity key stay separate

The existing codex pre-registration UUID and the identity key look alike and mean opposite things. The nonce is evidence, scoped to one launch, consumed on use, and expired by a TTL. The key is an alias, and it lives as long as the identity does.

A codex pane therefore carries two distinct values. Merging them would put the identity alias under the nonce's expiry sweep, which would delete identities on a timer.

This separation also falls out of how the daemon binds panes: when no UI process id is supplied, pane binding falls back to the existing nonce path, so the two values coexist without either side doing extra work.

## External Dependency

The xats side must accept the identity key at registration, resolve it during reconnect, and teach its startup hint to present it instead of asking the user. The following points were reviewed with its maintainer and are recorded as the agreed contract, because each is easy to implement vaguely.

**Reconnect shape differs per tool.** For `claude` it is `reconnect({identity_key, ui_pid})`, where the process id refreshes the pane, tty, and pid binding in the same call rather than leaving a window in which messages are delivered to the dead pane. For `codex` it is `reconnect({identity_key, thread_id})`, carrying the new post-restart thread so delivery is rewritten to it; the UI process id is deliberately omitted because supplying it would disable the launcher's pane pre-registration path.

**Registration-time binding is a three-way rule, not a two-way one.** A rename is a legitimate action that produces a new identity record, and at the daemon it is indistinguishable in shape from the bug this rule is meant to catch. Treating "key already bound elsewhere" as a flat error would make renaming fail:

| Situation | Handling |
|---|---|
| Key unknown | Bind it to the record being registered |
| Key already bound to this same record | Idempotent, no action |
| Key bound to another record whose process binding is absent, equal to this one, or no longer alive | Rename migration: move the key to the new record and clear it from the old one |
| Key bound to another record with a different live process | Reject, naming the previous team and name in the error so the collision is diagnosable |

**The key is stored on the identity record**, not in the codex pre-registration table, so expiry sweeps cannot reach it. Its uniqueness is scoped to the device, which is what makes a key carried to another machine inert.

The startup hint must try the identity key *before* its existing branches. After a fresh restart both "does not remember its identity" and "has an identity key" are true at once, and the existing order would take the process-id branch and fail.

Until that half ships, AoE-side behavior is verifiable on its own (the variable is present, stable across restarts, fresh on clone, absent when the feature is off) but end-to-end identity continuity is not.

## Risks / Trade-offs

- [Key copied into a cloned or forked session] -> Fresh allocation is an explicit requirement with dedicated coverage. Per Decision 8 this is the only point at which the failure is preventable, so the guard is not optional.
- [Agent omits the key on its first registration] -> The binding never happens, and AoE cannot observe it: injection succeeded, and a later launch simply gets told to register normally, which Decision 9 defines as a normal state. The symptom is "this feature just does not work for some panes" with no diagnosable trace. This is the most fragile link in the chain, and it is the one step driven by a natural-language hint. Mitigation is on the xats side: inline the value into the registration call shown in the hint rather than asking the agent to read the environment itself, and add the variable to the tool-description probe list the way the existing launcher variables are.
- [Key leaks through argv or logs] -> Environment-variable injection only, with coverage asserting it does not appear in the launch command's arguments.
- [Legacy databases lack the column] -> The existing idempotent schema-healing path adds it, matching how the durable slot pane column was healed.
- [Reconcile fails to carry the key into the slot] -> The identity silently stops surviving restarts. Covered by reconcile-level tests asserting the key reaches the durable record.
- [AoE ships before the xats side] -> Injecting an unused environment variable is inert. No user-visible behavior changes until the resolver exists.
- [User expects identity to be restored on a session created before this change] -> Pre-existing panes have no key and keep today's manual registration until they are recreated.

## Migration Plan

1. Add the identity key column to the durable slot record and the launch-time pane association through the migration system, with the idempotent healing path covering legacy databases.
2. Begin minting and injecting for Cross Agent Team panes. Existing sessions have no key and behave exactly as they do today.
3. Ship the xats-side resolver, binding rule, and startup-hint change separately. Identity continuity begins working for panes launched after step 2 once that lands.

Rollback is safe at the data level: an older binary ignores the column, and an unread environment variable has no effect.

## Open Questions

- Whether the identity key should be regenerated when a user deliberately re-registers a pane under a different name. The answer is no, but not for the reason initially assumed: the daemon does not update the record in place, it inserts a new one, so the key must be *migrated* to the new record by the registration-time rule above. Regenerating on the AoE side would be both unnecessary and impossible, since AoE never learns that a rename happened.
