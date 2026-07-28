## Context

`aoe __record-pane` is the single entry point for pane tracking. It reads `$TMUX_PANE` to know which pane it speaks for, takes the agent's native session id and working directory, and writes one `pane_live` row. The reconciler later snapshots those rows into durable `agent_slot` records, and cold-start recovery rebuilds panes from those slots.

Nothing in that chain is Claude-specific except its first link. `__record-pane` is invoked by the agent's own status hook, and only agents with a `hook_config` get one installed: today `claude`, `gemini`, and `cursor`. `codex` has none, so no Codex pane has ever produced a capture.

Two things make Codex different from the agents already covered, and both are why this is a change rather than a one-line registry edit:

1. Its settings file is TOML (`~/.codex/config.toml`), not JSON, and it is a file the user already owns and edits. The existing installer parses `serde_json::Value`, rewrites the whole document, and assumes AoE is the only meaningful author.
2. It gates hooks behind trust. A newly written hook is `Untrusted` until the user reviews it, and does not run before then.

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

So the source becomes a property of the agent: Claude reads stdin `session_id`, Codex reads `$CODEX_THREAD_ID`. This is narrower than reverting to "read an environment variable", which is what the earlier design got wrong: the variable is named per agent, not guessed from a pattern, and an agent with no declared source has no capture rather than a guessed one.

The working directory keeps its existing fallback chain (stdin `cwd`, then `$PWD`), which already works for both.

### Decision 2: Merge into the user's TOML, never rewrite it

`~/.claude/settings.json` is effectively AoE's to manage. `~/.codex/config.toml` is not: on the machine that prompted this change it holds a `notify` entry pointing at another tool and more than twenty `[projects."..."]` sections.

Installation therefore reads the document, adds or replaces only the AoE hook entries, and writes the rest back unchanged. Uninstall removes only entries AoE recognizes as its own, by the same `is_aoe_hook_command` test the JSON path already uses, and leaves the file otherwise untouched -- including leaving the file itself in place when it holds anything else.

The format is declared on the hook configuration rather than sniffed from the file extension, so the installer dispatches on what the agent says it uses.

### Decision 3: The trust gate is reported, not bypassed

Codex will not run a newly installed hook until it is trusted. `--dangerously-bypass-hook-trust` exists and would make the channel work immediately, and using it would mean AoE silently arranging for its own code to run inside the user's agent without the review Codex deliberately requires.

Installation instead tells the user what it wrote and that Codex will ask them to trust it once. This costs one manual step per machine and converges exactly the way the adopted-pane identity key does.

### Decision 4: No retroactive correction of existing slots

A slot recorded as `claude` for a pane now running Codex is wrong, and after this change the same pane will report itself correctly the next time it fires a hook event. Rewriting existing rows at upgrade time would mean guessing, from a pane's current command, what its conversation id was -- and the conversation id is the part that cannot be guessed.

The stale rows therefore correct themselves on first report and are otherwise left alone. This is stated so the first run after upgrading is not mistaken for a defect: a Codex pane that has not yet fired an event still shows its old record.

### Decision 5: What this change deliberately leaves broken

Making Codex panes trackable makes a latent defect reachable. Cross Agent Team decoration (`--dangerously-load-development-channels`) and identity-key ownership are decided by the instance's tool: `is_cross_agent_team()` tests `self.tool`, and `slot_needs_identity_key` skips slot 0 unconditionally because slot 0's key is supposed to live on the instance record. An adopted slot whose agent differs from the instance tool therefore gets the wrong decoration, and an adopted slot 0 gets no key at all.

Today that is unreachable, because an adopted slot can only ever be Claude and the shapes that would collide do not occur. After this change a Claude Cross Agent Team instance can hold an adopted Codex slot, and both defects become live.

It is left out of this change because it is a separate decision about what "the instance's agent" means once a pane's agent and its instance's tool are routinely different, and because folding it in would put an unreviewed second concern inside a change whose first concern is already testable on its own. It must not be left unowned: it is the reason this change's own review flagged it, and it needs to land before or alongside the first Cross Agent Team instance with an adopted Codex pane.

## Risks / Trade-offs

- [Writing a user-owned config file] -> Merge-and-preserve is the requirement, with coverage that unrelated keys and sections survive install and uninstall. The blast radius is a file the user edits by hand, so "we only touch our own entries" has to be verified, not asserted.
- [The trust gate makes the first run look broken] -> Stated at install time and in this design, the same way the xats identity-key convergence step is.
- [Codex changes its hook schema] -> The schema is read from Codex 0.145. AoE writes a `[[SessionStart]]`-style array-of-tables entry and does not attempt to validate Codex's own semantics; a schema change breaks the capture rather than the agent, and shows up as a Codex pane that stops reporting.

## Migration Plan

No data migration. Existing rows keep their values and correct themselves on the pane's next report (Decision 4). Users who never run Codex are unaffected: nothing is written to `~/.codex/config.toml` unless Codex is a detected agent.

## Open Questions

- Whether Codex hooks should be installed globally (`~/.codex/config.toml`) or per project. Global matches what AoE does for Claude and is what this change implements; per-project would scope the blast radius but would have to be written once per managed directory.
