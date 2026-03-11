## MODIFIED Requirements

### Requirement: Terminal tab title reflects TUI state
The TUI SHALL set the terminal tab/window title using the OSC 0 escape sequence (`\x1b]0;{title}\x07`) to reflect the current application state. The title MUST be updated whenever the TUI state changes. When any active session is running an agent with `sets_own_title: true`, the TUI SHALL NOT write any title escape sequence, allowing the agent to manage the terminal title directly.

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

#### Scenario: Agent with sets_own_title is running
- **WHEN** any active session is running an agent with `sets_own_title: true` (e.g., Claude Code, Gemini CLI)
- **THEN** the TUI SHALL NOT write any OSC 0 title escape sequence, allowing the agent to manage the terminal title

#### Scenario: Agent with sets_own_title exits or no such agent active
- **WHEN** no active session has an agent with `sets_own_title: true`
- **THEN** the TUI SHALL resume normal title management based on state
