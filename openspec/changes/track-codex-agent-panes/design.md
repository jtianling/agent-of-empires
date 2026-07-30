## Context

`aoe __record-pane` is the single entry point for pane tracking. It reads `$TMUX_PANE` to know which pane it speaks for, takes the agent's native session id and working directory, and writes one `pane_live` row. The reconciler later snapshots those rows into durable `agent_slot` records, and cold-start recovery rebuilds panes from those slots.

Nothing in that chain is Claude-specific except its first link. `__record-pane` is invoked by the agent's own status hook, and only agents with a `hook_config` get one installed: today `claude`, `gemini`, and `cursor`. `codex` has none, so no Codex pane has ever produced a capture.

Two things make Codex different from the agents already covered:

1. Its session id comes from its environment (`$CODEX_THREAD_ID`), not from hook stdin.
2. It gates hooks behind trust. A newly written hook is `Untrusted` until the user reviews it, and does not run before then.

An earlier draft of this design carried a third: that Codex keeps its hooks in `~/.codex/config.toml`, a TOML file the user owns and edits, so installation would have to merge into it. That turned out to be avoidable -- see Decision 2.

## Goals / Non-Goals

**Goals:**

- A Codex pane produces a `pane_live` capture with the same fields, keyed the same way, as a Claude pane.
- Installing Codex hooks preserves every byte of unrelated user configuration in `~/.codex/config.toml`.
- The trust step is stated to the user rather than bypassed.

**Non-Goals:**

- Correcting a slot whose recorded agent is stale. This change stops new staleness at its source for Codex; it does not re-derive history. A slot recorded before this ships stays as recorded until its pane reports again.
- Cross Agent Team decoration and identity-key ownership for adopted panes (Decision 5).
- Hook support for any other untracked agent (`opencode`, `vibe`, `copilot`, `pi`).

## Decisions

### Decision 1: The session id comes from a source the agent names, not from one shape

The current requirement says the native session id is read from hook stdin JSON, and says so as a correction of an earlier design that read an environment variable. That correction was right for Claude, whose hook receives `{"session_id": ...}` on stdin and whose environment carries no reliable equivalent.

Codex looked like it inverted this. It exports `$CODEX_THREAD_ID` (0.124 and later), and that is the same value identifying the conversation for resume and for xats reconnect, so this design first had Codex read the variable.

That was wrong, and a live session is what showed it: Codex exports the variable into the environment of the commands its **tools** run, not into its **hooks'**. The measurement is by elimination, and it is the reason the conclusion is trustworthy rather than another guess. A Codex pane's hook wrote its status file, which is gated on `$AOE_INSTANCE_ID` -- a variable AoE injects into the Codex process and nothing else sets -- so the hook demonstrably inherits the agent's full environment. The same hook's capture, running first in the same command, wrote no row. Full environment plus a live `$TMUX_PANE` leaves the thread variable as the only gate it could have failed.

One earlier observation had pointed the other way and was over-read: a `codex exec` invocation *did* produce a row carrying a thread id. That process was the child of a tool command, so it inherited `$CODEX_THREAD_ID` from the tool environment its parent had set. It showed that the variable reaches nested processes, not that it reaches hooks.

So the source stays a property of the agent, and every agent's is currently `HookStdin`. Keeping it named rather than assumed is what let this be corrected in one registry line instead of across the capture path.

An earlier draft of this decision went one step further and said an agent with no declared source is not captured at all. That was wrong, and the suite is what showed it: a pane recorded as `shell` -- an adopted pane next to an agent -- stopped being capturable, and two recovery tests that depend on that shape went red. The property worth having is narrower than the rule that broke them. What must never happen is a Codex pane recorded under the id on its stdin, because that id is real and identifies something else. An agent AoE installs no hook for cannot reach this path from a hook at all, and a caller that names one is stating the id outright rather than having it guessed. So a declared source is exclusive, and no declaration keeps the stdin id.

The working directory keeps its existing fallback chain (stdin `cwd`, then `$PWD`), which already works for both.

### Decision 2: Write the hooks file Codex already offers, and leave `config.toml` alone

Codex discovers hooks from two places per configuration layer: a `[hooks]` table inside `config.toml`, and a dedicated `hooks.json` in the same folder. Both are loaded; a layer that populates both only earns a warning ("loading hooks from both ..."), not a conflict.

That second source removes the reason this change looked dangerous. `~/.codex/config.toml` is the user's: on the machine that prompted this change it holds a `notify` entry pointing at another tool and more than twenty `[projects."..."]` sections, and AoE merging into it would put a hand-edited file at risk on every install. `~/.codex/hooks.json` is a file AoE creates and owns, exactly as `~/.claude/settings.json` is today. So `config.toml` is not written at all.

The shape is the one the installer already produces. Codex's `HooksFile` is `{"hooks": {"<Event>": [{"matcher": ..., "hooks": [{"type": "command", "command": "..."}]}]}}` -- the same structure `build_aoe_hooks` writes for Claude. No TOML support, no per-agent settings format, no new installer path: the registry entry names a different path and a different event set, and the existing JSON installer does the rest, including preserving hooks the user put there themselves.

Verified against the installed binary rather than the source checkout, which was six weeks stale: `codex-cli 0.145.0` contains the `hooks.json` discovery path, the both-sources warning string, and the full event list `PreToolUse, PermissionRequest, PostToolUse, PreCompact, PostCompact, SessionStart, SessionEnd, UserPromptSubmit, SubagentStart, SubagentStop, Stop`. Note `SessionEnd`, which the checkout did not have -- reading the source alone would have described a different version than the one on the machine.

The one asymmetry with Claude's event set is that Codex has no `Notification` event. `PermissionRequest` is its analogue for the waiting status, and `ElicitationResult` has no counterpart.

### Decision 3: The trust gate is reported, not bypassed

Codex will not run a newly installed hook until it is trusted. `--dangerously-bypass-hook-trust` exists and would make the channel work immediately, and using it would mean AoE silently arranging for its own code to run inside the user's agent without the review Codex deliberately requires.

Installation instead tells the user what it wrote and that Codex will ask them to trust it once. This costs one manual step per machine and converges exactly the way the adopted-pane identity key does.

### Decision 4: No retroactive correction of existing slots

A slot recorded as `claude` for a pane now running Codex is wrong, and after this change the same pane will report itself correctly the next time it fires a hook event. Rewriting existing rows at upgrade time would mean guessing, from a pane's current command, what its conversation id was -- and the conversation id is the part that cannot be guessed.

The stale rows therefore correct themselves on first report and are otherwise left alone. This is stated so the first run after upgrading is not mistaken for a defect: a Codex pane that has not yet fired an event still shows its old record.

### Decision 5: The defect this change would have made reachable, and where it now stands

Making Codex panes trackable makes a latent defect reachable. Cross Agent Team decoration (`--dangerously-load-development-channels`) and identity-key ownership were decided by the instance's tool rather than the pane's agent, so an adopted slot whose agent differed from the instance tool got the wrong decoration, and an adopted slot 0 got no identity key at all. That was unreachable only because an adopted slot could only ever be Claude.

It has since been fixed on its own, ahead of this change (`c29ed8be`): decoration follows the slot's recorded agent in both directions, and an adopted slot 0 is recognized as needing its own key. This section is kept rather than deleted because it names the dependency: if that fix is ever reverted, this change is what makes the defect live.

What remains genuinely out of scope is the restart path for a pane no slot describes, which reads the pane's own process to avoid relaunching it as the instance's tool. That is a guess where this change supplies a fact, and it stops being load-bearing for Codex once Codex panes hold slots.

### Decision 6: Codex is told its own pane, because its hooks do not run in it

The rest of this design assumed a hook inherits the environment of the pane its agent runs in. For Claude that holds. For Codex it does not, and the failure is worse than an absent variable.

Codex clients launched as `codex --remote ws://...` are thin front ends for one long-lived app-server process, and that daemon is where tools and hooks actually execute. It inherits its environment once, when it starts. Measured on the machine this change was written for: the client in pane `%47` had `TMUX_PANE=%47` and `AOE_INSTANCE_ID` set correctly, while a command it ran saw `TMUX_PANE=%39` -- the pane the daemon had been started from, hours earlier, belonging to an unrelated shell session -- and no `AOE_INSTANCE_ID` at all. Every Codex client on that machine shared the one daemon, so every one of them would have reported the same wrong pane.

So the capture would not merely have gone missing. It would have claimed a pane belonging to something else, and recovery reads those rows. One such row was in fact written during this investigation, recording a shell pane as a Codex agent.

What Codex does forward per session is configuration. `-c shell_environment_policy.set.<NAME>=<value>` reaches the executing environment through the daemon, and it merges into the user's own `[shell_environment_policy.set]` table rather than replacing it (verified with a live session: an injected probe variable and the user's two pre-existing entries were all present). A Codex launch therefore carries its pane id and instance id as two such overrides, the pane expanded by the pane's own shell at launch where `$TMUX_PANE` is still correct.

The capture reads `$AOE_TMUX_PANE` first and `$TMUX_PANE` only as a fallback. The precedence is that way round because an agent sets the first only when the second names a pane that is not its own, so preferring the fallback would let a stale value claim someone else's pane.

This is separate from Decision 1, which the same investigation went on to correct in the other direction: the pane is unreliable only for app-server-backed clients, while the thread variable is absent from every Codex hook.

**The remedy above was then falsified on a live session.** A client launched with both overrides on its command line fired a hook that recorded the daemon's pane anyway: `shell_environment_policy.set` is applied to the environment of the *shell tool's* commands -- the name says so -- and never to hooks. The probe that had validated the mechanism was itself a shell tool command, so it validated the wrong executor. The diagnosis in this section stands; the remedy is superseded by Decision 7.

### Decision 7: No Codex hooks at all -- the rollout file is the source of truth

Decision 6 left Codex hooks with no channel that can carry the pane: the environment lies about it, per-session config never reaches them, and their stdin has no pane field. Baking the pane into the hook command itself per session would change the command's trust hash on every launch and reprompt the user each time. Every hook-shaped option is exhausted, so Codex has no hooks: the registry entry drops its hook configuration, nothing is written under `~/.codex/`, and the trust step (Decision 3) disappears with the file. Codex status stays on pane-content detection, which is what it was before this change.

What replaces the hook is Codex's own bookkeeping. Every conversation, in every launch mode, writes one rollout file -- `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<thread-id>.jsonl`, first line carrying the conversation's `cwd` -- and AoE knows everything else at launch: the pane, the instance, the project path, and when the pane's process started. The reconciler binds a Codex instance's primary pane to the earliest unclaimed rollout created after the pane started in the instance's project directory, and the existing snapshot path turns that into a durable slot. A resumed conversation's rollout predates the respawn and never re-matches, which is correct: its slot already carries the conversation. A command override is the user's own program and is never claimed for.

The capture subcommand also stops trusting `$TMUX_PANE` on faith, hook or no hook: it verifies the named pane's root process is among its own ancestors before writing, and skips the write on a positive mismatch. A hook running in a Codex daemon fails that check (the daemon is disowned; the pane it inherited belongs to a shell), which is what turns "stale hooks still installed on some machine" from a data-corruption path into a no-op. Verification is positive-only -- an unanswerable check (no server, pane gone) is accepted -- so hand-launched agents outside AoE's knowledge keep being captured.

What this gives up: a hand-launched Codex in an adopted pane is no longer captured (its hook is gone, and no launch record exists to anchor a rollout match). The app-server case was never capturable by hooks, and the AoE-launched case -- the one recovery is responsible for -- is fully covered.

One trap worth recording for the tests: `tmux` silently falls back to the real default socket when `$TMUX_TMPDIR` names a directory that does not exist. A test that wants "no server reachable" must point at an existing empty directory, or the ownership check above answers against the developer's live server.

## Risks / Trade-offs

- [A user's own `hooks.json`] -> AoE creates the file, but it may not be the only author. The existing installer already merges rather than overwrites and keeps non-AoE entries; that behavior is what makes this safe, so it is covered rather than assumed.
- [`config.toml` is still where Codex records hook trust] -> AoE does not write that file; Codex does, under `[hooks.state]`, when the user trusts a hook. Nothing here should be read as AoE never causing a write to it -- only as AoE never performing one.
- [The trust gate makes the first run look broken] -> Stated at install time and in this design, the same way the xats identity-key convergence step is.
- [Codex changes its hook schema] -> The shape is read from the installed `codex-cli 0.145.0` binary. A schema change breaks the capture rather than the agent, and shows up as a Codex pane that stops reporting. The version-drift trap is real and was hit while designing this: the local source checkout was six weeks behind the installed binary and described a smaller event set.

## Migration Plan

No data migration. Existing rows keep their values and correct themselves on the pane's next report (Decision 4). Users who never run Codex are unaffected: nothing is written to `~/.codex/` unless Codex is a detected agent.

## Open Questions

- Whether Codex hooks should be installed globally (`~/.codex/hooks.json`) or per project. Global matches what AoE does for Claude and is what this change implements; per-project would scope the blast radius but would have to be written once per managed directory.
- ~~Whether a hook command Codex spawns inherits `$CODEX_THREAD_ID`.~~ Answered on a live session with the hook trusted: it does, and the id is the running conversation's. What the same run disproved is the wider assumption behind the question -- that the hook inherits the *pane's* environment at all. It inherits the app-server's. See Decision 6.
