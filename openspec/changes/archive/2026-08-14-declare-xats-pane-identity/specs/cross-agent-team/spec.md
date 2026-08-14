## MODIFIED Requirements

### Requirement: Cross Agent Team panes carry a durable identity key

When Cross Agent Team is enabled for a pane, AoE SHALL associate that pane with an opaque identity key and SHALL inject it into the launched pane as the `XATS_IDENTITY_KEY` environment variable. The key SHALL be minted by AoE, SHALL be treated as an opaque value that AoE never interprets, and SHALL be injected on every launch of that pane regardless of restart mode, including the pane's first launch.

AoE MAY store and carry a xats team and agent name declared by the user for a pane, and SHALL treat both as opaque values it never interprets, exactly as it treats the identity key. AoE SHALL NOT derive, guess, or default either value from any other data it holds, including the session title, the pane's tool, or the working directory.

#### Scenario: Key injected on first launch

- **WHEN** a Cross Agent Team pane is launched for the first time
- **THEN** AoE SHALL mint an identity key for it
- **AND** the launched pane's environment SHALL contain that key as `XATS_IDENTITY_KEY`

#### Scenario: Key injected for both supported tools

- **WHEN** a Cross Agent Team pane is launched for `claude` or for `codex`
- **THEN** the launched pane's environment SHALL contain the pane's identity key

#### Scenario: Key is distinct from the codex pane pre-registration nonce

- **WHEN** a Cross Agent Team `codex` pane is launched
- **THEN** the pane SHALL carry both its durable identity key and a freshly generated single-use pane pre-registration nonce
- **AND** the two values SHALL be different

#### Scenario: No key when the feature is disabled

- **WHEN** a session is launched with Cross Agent Team disabled
- **THEN** AoE SHALL NOT mint or inject an identity key

#### Scenario: Key is not exposed through command arguments

- **WHEN** a Cross Agent Team pane is launched
- **THEN** the identity key SHALL NOT appear in the launch command's arguments
- **AND** it SHALL NOT be written to logs

#### Scenario: A declared name is never invented by AoE

- **WHEN** a Cross Agent Team pane has no declared xats team or agent name
- **THEN** AoE SHALL treat the identity as undeclared
- **AND** SHALL NOT substitute the session title or any other value in its place

### Requirement: Codex xats pane bootstrap

When Cross Agent Team is enabled for a non-sandboxed `codex` session, AoE SHALL
launch Codex through a pane-local xats bootstrap. The bootstrap MUST pre-register
the current `TMUX_PANE` with a fresh UUID before executing Codex, then connect the
Codex TUI to the local app-server with that UUID supplied as `xats.agent_id` and
the session project path supplied as the Codex working directory.

When the pane's environment carries a non-empty `XATS_IDENTITY_KEY`, the
bootstrap SHALL tell the pre-registration call to read it, by naming the
variable via `--identity-key-env`; the CLI reads the value from its own
environment. The key's value SHALL NOT appear on the argv of any process the
bootstrap script starts -- not the executed Codex command line and not the
pre-registration call's own -- because argv is readable by every process on
the machine. (The value does reach the pane through AoE's pre-existing
env-injection prefix, which transits the tmux launch argv; that mechanism
predates this change, is shared with Claude panes, and is out of scope here.
What this change adds on top is masking the value in AoE's own debug logs of
launch commands.) The pre-registration call SHALL also carry a lengthened row
TTL (`--ttl`, the flag the CLI parses) so the daemon's poke-back window covers
a Codex cold start.

If a pre-registration call carrying the declared-identity flags fails, the
bootstrap SHALL retry it once with those flags removed and every other flag
kept, so a CLI that does not parse them cannot fail a Codex launch. The retry
SHALL keep naming the pane's identity key and SHALL keep the TTL; retrying
without the key is prohibited by "Codex xats bootstrap failure is explicit"
and that prohibition wins. A pane that declares no identity SHALL make exactly
one pre-registration attempt, because it adds no flag to fall back from and any
fallback would therefore have to drop the key.

The retry decision SHALL rest on the exit code alone, not on the CLI's error
text, and SHALL survive shell options inherited from the environment
(`SHELLOPTS` carrying `errexit` reaches the bootstrap's `sh`).

The bootstrap SHALL NOT read, inject, print, or persist the xats authentication
token value. It SHALL rely on the already-configured local xats environment.

#### Scenario: Fresh Codex xats launch

- **WHEN** a user creates a non-sandboxed Codex session with Cross Agent Team enabled
- **THEN** the target pane is pre-registered with a fresh UUID
- **AND** Codex starts in remote mode against the local app-server
- **AND** Codex receives the project path and the same UUID as `xats.agent_id`

#### Scenario: Identity key rides the pre-registration environment, not any argv

- **WHEN** a Codex Cross Agent Team pane launches with `XATS_IDENTITY_KEY` in its environment
- **THEN** the pre-registration call carries `--identity-key-env` naming the variable
- **AND** neither the pre-registration argv nor the executed Codex command line contains the key's value

#### Scenario: Debug logs of launch commands mask the key's value

- **WHEN** AoE logs a pane launch command or its tmux argv at debug level
- **THEN** the logged text carries the identity-key env prefix with its value struck out

#### Scenario: A pane without an identity key pre-registers without the flag

- **WHEN** a Codex Cross Agent Team pane launches with no `XATS_IDENTITY_KEY` in its environment
- **THEN** the pre-registration call carries no identity-key flag
- **AND** the launch proceeds as before

#### Scenario: A CLI that rejects the declared-identity flags does not fail the launch

- **WHEN** a Codex Cross Agent Team pane with a declared xats identity launches
- **AND** the pre-registration call carrying the declared-identity flags exits non-zero
- **THEN** the bootstrap retries once with the declared-identity flags removed
- **AND** the retry still names the pane's identity key and carries the TTL
- **AND** a successful retry launches Codex normally
- **AND** the retry fires even under shell options inherited from the environment

#### Scenario: An undeclared pane makes exactly one attempt

- **WHEN** a Codex Cross Agent Team pane with no declared identity fails its pre-registration
- **THEN** the bootstrap SHALL NOT make a second pre-registration attempt
- **AND** the pane prints the pre-registration diagnostic and terminates with a non-zero status

#### Scenario: YOLO disabled remains non-YOLO

- **WHEN** Cross Agent Team is enabled for Codex and YOLO Mode is disabled
- **THEN** the Codex command uses the xats bootstrap
- **AND** the command does not include `--dangerously-bypass-approvals-and-sandbox`

#### Scenario: YOLO enabled coexists with xats bootstrap

- **WHEN** Cross Agent Team and YOLO Mode are both enabled for Codex
- **THEN** the Codex command uses the xats bootstrap
- **AND** the command includes `--dangerously-bypass-approvals-and-sandbox`

#### Scenario: Codex fork uses xats bootstrap

- **WHEN** a Cross Agent Team Codex session is forked from a captured native session id
- **THEN** the fork pane is pre-registered with a fresh xats claim
- **AND** the Codex fork command connects to the local app-server
- **AND** the parent native session id is preserved as the fork source

## ADDED Requirements

### Requirement: Cross Agent Team panes carry a declared xats identity

A pane with Cross Agent Team enabled SHALL be able to carry a user-declared xats
identity consisting of a team and an agent name, and a declaration SHALL belong
to exactly one live pane. Both parts SHALL be independently optional: an empty
value means undeclared, and a pane with both parts empty SHALL behave exactly as
panes behaved before this capability existed.

The declared identity SHALL be stored on the pane's own durable slot, alongside
that pane's identity key, so sibling panes in the same session declare
independently. Storage SHALL tolerate records written before this capability
existed by reading them as undeclared.

#### Scenario: Declared identity persists on the pane's slot

- **WHEN** a user declares a xats team and agent name for a Cross Agent Team pane
- **THEN** AoE SHALL store both values on that pane's durable slot
- **AND** reopening AoE SHALL read back the same values

#### Scenario: Sibling panes declare independently

- **WHEN** two panes of the same session each declare a xats identity
- **THEN** each pane's declared values SHALL be stored on its own slot
- **AND** neither pane's values SHALL overwrite the other's

#### Scenario: Records predating the capability read as undeclared

- **WHEN** AoE reads a slot record written before this capability existed
- **THEN** the pane's declared team and agent name SHALL read as empty
- **AND** the pane SHALL launch exactly as it did before

#### Scenario: Only one part declared

- **WHEN** a pane declares a team but no agent name
- **THEN** AoE SHALL store and carry the declared part
- **AND** SHALL carry the undeclared part as empty rather than substituting a value

### Requirement: Declared identity is injected into the launched pane

When a Cross Agent Team pane with a declared xats identity is launched, AoE SHALL
inject the declared parts into that pane's environment, so an agent able to read
its own environment can register under that identity without asking the user.
Undeclared parts SHALL NOT be injected as empty variables.

Unlike the identity key, the declared identity is not a credential, so it MAY
appear in launch command arguments and MAY be logged.

#### Scenario: Declared identity reaches the pane environment

- **WHEN** a Cross Agent Team pane with a declared team and agent name is launched
- **THEN** the launched pane's environment SHALL carry both declared values
- **AND** they SHALL be carried alongside the pane's identity key

#### Scenario: Undeclared identity injects nothing

- **WHEN** a Cross Agent Team pane with no declared identity is launched
- **THEN** the launched pane's environment SHALL carry no declared-identity variables
- **AND** the launch command SHALL be unchanged from before this capability existed

#### Scenario: Injection is independent of the pane's tool

- **WHEN** a Cross Agent Team pane declaring an identity is launched for any supported tool
- **THEN** the declared values SHALL be injected the same way for every such tool

### Requirement: Declared identity is stable across restart, resume, and recovery

A pane's declared xats identity SHALL survive every relaunch path that preserves
the pane's slot, including restart, resume, fresh restart, and cold-start
recovery, and SHALL be injected on each of those launches. AoE SHALL NOT mint,
clear, or rotate a declared identity on relaunch.

#### Scenario: Declared identity survives restart

- **WHEN** a pane with a declared xats identity is restarted
- **THEN** the relaunched pane SHALL carry the same declared values

#### Scenario: Declared identity survives cold-start recovery

- **WHEN** AoE recovers a pane's slot after its tmux session was lost
- **THEN** the recovered pane SHALL carry the same declared values

#### Scenario: Relaunch never clears a declaration

- **WHEN** a pane with a declared xats identity is relaunched by any path
- **THEN** AoE SHALL NOT write an empty declared identity over the stored one

#### Scenario: A fork does not inherit the parent's declaration

- **WHEN** a session whose pane declares a xats identity is forked
- **THEN** the forked session's pane SHALL start undeclared
- **AND** the parent SHALL keep its own declaration

### Requirement: Declared identity reaches the daemon through Codex pre-registration

A non-sandboxed Codex pane's bootstrap SHALL pass that pane's declared xats identity to the pre-registration call as arguments, so the daemon can address the pane by identity even when the pane's identity key resolves to no holder.

This channel exists because Codex tool processes run inside a shared app-server
and therefore read that server's environment rather than their own pane's: the
declaration must reach the daemon before Codex starts, since Codex itself can
never see it.

Undeclared parts SHALL NOT be passed. A pane with no declared identity SHALL
produce the same pre-registration call it produced before this capability
existed.

#### Scenario: Declared identity is passed at pre-registration

- **WHEN** a Codex Cross Agent Team pane with a declared team and agent name launches
- **THEN** the pre-registration call SHALL carry both declared values as arguments

#### Scenario: Undeclared Codex pane is unchanged

- **WHEN** a Codex Cross Agent Team pane with no declared identity launches
- **THEN** the pre-registration call SHALL carry no declared-identity arguments

### Requirement: Declared identity is entered per pane where the feature is switched on

Wherever a user can turn Cross Agent Team on for a pane, the user SHALL also be
able to declare that pane's xats team and agent name. The fields SHALL be
presented per pane, SHALL accept an empty value to mean undeclared, SHALL refuse
values storage would refuse, and SHALL be inert when Cross Agent Team is off for
that pane.

A declaration is entered when the pane is configured. AoE has no flow for
reconfiguring an already-created pane, so this capability adds none: a stored
declaration SHALL be replaced only by a later non-empty declaration for that
slot, and SHALL NOT be cleared by any launch, restart, or recovery that carries
no declaration.

#### Scenario: Declaring an identity when creating a session

- **WHEN** a user enables Cross Agent Team for a pane while creating a session
- **THEN** the user SHALL be able to enter that pane's xats team and agent name
- **AND** each pane of that session SHALL take its own values

#### Scenario: Clearing a field before submitting means undeclared

- **WHEN** a user types a declaration and then clears the field before submitting
- **THEN** the submitted pane SHALL carry that part as undeclared

#### Scenario: A rejected value never reaches storage

- **WHEN** a user enters a value the storage boundary refuses
- **THEN** the field SHALL refuse it at entry

#### Scenario: Characters the daemon reads as addressing syntax are refused at entry

- **WHEN** a user types a declared agent name containing `:`, `(` or `)`
- **THEN** the field SHALL refuse those characters
- **AND** a declared team SHALL likewise refuse `(` and `)`
- **AND** the refusal SHALL happen at entry rather than at launch, because the
  Codex bootstrap cannot distinguish the daemon rejecting a bad value from an
  older CLI rejecting an unknown flag, and would drop the declaration silently

#### Scenario: Fields are inert when the feature is off

- **WHEN** Cross Agent Team is off for a pane
- **THEN** the declared-identity fields SHALL NOT be editable for that pane
- **AND** a declaration typed before the switch was turned off SHALL NOT be submitted
