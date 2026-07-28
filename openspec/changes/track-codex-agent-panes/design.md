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

Codex inverts it. It exports `$CODEX_THREAD_ID` into the pane's environment (0.124 and later), and this is the same value that identifies the conversation for resume and for xats reconnect. Its hook stdin shape is not something AoE needs to depend on to get the id it already has a stable name for.

So the source becomes a property of the agent: Claude reads stdin `session_id`, Codex reads `$CODEX_THREAD_ID`. This is narrower than reverting to "read an environment variable", which is what the earlier design got wrong: the variable is named per agent, not guessed from a pattern.

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

## Risks / Trade-offs

- [A user's own `hooks.json`] -> AoE creates the file, but it may not be the only author. The existing installer already merges rather than overwrites and keeps non-AoE entries; that behavior is what makes this safe, so it is covered rather than assumed.
- [`config.toml` is still where Codex records hook trust] -> AoE does not write that file; Codex does, under `[hooks.state]`, when the user trusts a hook. Nothing here should be read as AoE never causing a write to it -- only as AoE never performing one.
- [The trust gate makes the first run look broken] -> Stated at install time and in this design, the same way the xats identity-key convergence step is.
- [Codex changes its hook schema] -> The shape is read from the installed `codex-cli 0.145.0` binary. A schema change breaks the capture rather than the agent, and shows up as a Codex pane that stops reporting. The version-drift trap is real and was hit while designing this: the local source checkout was six weeks behind the installed binary and described a smaller event set.

## Migration Plan

No data migration. Existing rows keep their values and correct themselves on the pane's next report (Decision 4). Users who never run Codex are unaffected: nothing is written to `~/.codex/` unless Codex is a detected agent.

## Open Questions

- Whether Codex hooks should be installed globally (`~/.codex/hooks.json`) or per project. Global matches what AoE does for Claude and is what this change implements; per-project would scope the blast radius but would have to be written once per managed directory.
- Whether a hook command Codex spawns inherits `$CODEX_THREAD_ID`. Decision 1 depends on it, and it is the one link that cannot be settled by reading the binary's strings -- it needs a real Codex session with the hook trusted. Task 6.3 is where that gets answered; if it turns out the variable is not in the hook's environment, the source for Codex has to be re-derived from its hook stdin instead, and Decision 1 is what changes.
