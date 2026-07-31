## ADDED Requirements

### Requirement: Codex conversation binding covers every pane of a managed session

Codex rollout binding SHALL be attempted for each pane of a managed session, not for its primary pane alone. A Codex pane that is never bound produces no `pane_live` capture, therefore no durable slot, and is therefore skipped by restart and by cold-start recovery -- silently, because an untracked pane looks the same as a session that has none.

The preconditions that describe the instance rather than the pane SHALL be applied to the primary pane only. That the instance's tool is `codex`, and that its command has not been overridden, are statements about the instance's own agent pane; a non-primary pane may run a different agent than the instance's tool, and an override describes the program AoE launches for the instance, not what a user or a right-pane selection put in another pane. Every pane, primary or not, SHALL still be judged by the positive evidence already required: a process in its tree invoking Codex.

Panes SHALL be considered in ascending pane index, which is their creation order. Together with the existing rule that a thread id already held by another pane or slot is never claimed again, this SHALL keep a later-created pane from taking an earlier pane's conversation.

#### Scenario: A second Codex pane is bound to its own conversation
- **WHEN** a managed session has two panes each running Codex
- **AND** each has a rollout created after its own pane process started in the session's project path
- **THEN** the store SHALL hold a `pane_live` row for each pane
- **AND** the two rows SHALL carry different thread ids

#### Scenario: A bound extra pane reaches a durable slot
- **WHEN** a non-primary pane of a managed session has been bound to a conversation
- **AND** the reconciler runs
- **THEN** an `agent_slot` row SHALL exist for that pane

#### Scenario: A Codex pane in a session whose tool is not Codex is bound
- **WHEN** a managed session's instance tool is not `codex`
- **AND** one of its non-primary panes is running Codex
- **THEN** that pane SHALL be eligible for rollout binding

#### Scenario: A command override does not block a non-primary pane
- **WHEN** a managed session's instance carries a command override
- **AND** one of its non-primary panes is running Codex
- **THEN** that pane SHALL be eligible for rollout binding
- **AND** the instance's own primary pane SHALL still not be claimed for

#### Scenario: A non-primary pane not running Codex is not claimed for
- **WHEN** a non-primary pane of a managed session holds no process invoking Codex
- **THEN** no rollout SHALL be claimed for that pane

#### Scenario: The primary pane's conversation is not taken by a later pane
- **WHEN** a session's primary pane and a later-created pane are both eligible
- **AND** the primary pane's rollout is the earlier of the two
- **THEN** the primary pane SHALL hold that rollout's thread id
- **AND** the later pane SHALL hold a different one
