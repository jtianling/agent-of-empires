## MODIFIED Requirements

### Requirement: Terminal tab title reflects TUI state
The TUI SHALL set the terminal tab/window title using the OSC 0 escape sequence (`\x1b]0;{title}\x07`) to reflect the current application state while the AoE pane is active. Inside agent sessions, tmux SHALL propagate the active pane title to the outer terminal. Agents with `sets_own_title: true` (Claude Code, Gemini CLI) SHALL keep their own pane titles, and agents without `sets_own_title` (including Codex CLI) SHALL continue using AoE-managed pane titles.

#### Scenario: TUI starts and sets initial title
- **WHEN** the TUI launches
- **THEN** the terminal tab title SHALL be set to `◇ AoE` (the idle/home state)

#### Scenario: Dialog opens requiring user input
- **WHEN** any dialog that requires user input is open (confirmation, creation, rename, delete options, hook trust, profile picker)
- **THEN** the terminal tab title SHALL change to `✋ Input Required - AoE`

#### Scenario: Session is being created
- **WHEN** a session creation is in progress (creation poller active)
- **THEN** the terminal tab title SHALL change to `⏳ Creating... - AoE`

#### Scenario: Settings view is open
- **WHEN** the user opens the settings screen
- **THEN** the terminal tab title SHALL change to `⚙ Settings - AoE`

#### Scenario: Diff view is open
- **WHEN** the user opens the diff view
- **THEN** the terminal tab title SHALL change to `📊 Diff - AoE`

#### Scenario: Return to home screen
- **WHEN** the user closes a dialog, settings, or diff view and returns to the home screen
- **THEN** the terminal tab title SHALL revert to `◇ AoE`

#### Scenario: Switch to Claude Code or Gemini CLI session
- **WHEN** the user switches from the AoE TUI to an agent session whose tool has `sets_own_title: true`
- **THEN** the outer terminal title SHALL update to that session's current pane title
- **AND** the displayed title SHALL match what the same tool would show when run directly outside AoE

#### Scenario: Switch to Codex CLI session
- **WHEN** the user switches from the AoE TUI to a Codex CLI session
- **THEN** the outer terminal title SHALL update to the pane title AoE currently manages for that session
- **AND** Codex CLI title behavior SHALL remain unchanged from the pre-fix behavior
