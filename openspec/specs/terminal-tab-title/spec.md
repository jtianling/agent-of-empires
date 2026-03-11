# Capability Spec: Terminal Tab Title

**Capability**: `terminal-tab-title`
**Created**: 2026-03-12
**Status**: Draft

## Overview

The TUI dynamically updates the terminal tab/window title using OSC 0 escape sequences to reflect
the current application state. This gives users at-a-glance awareness of what AoE is doing when
they have multiple terminal tabs open. The feature is enabled by default and can be disabled via
configuration.

## Requirements

### Requirement: Terminal tab title reflects TUI state
The TUI SHALL set the terminal tab/window title using the OSC 0 escape sequence (`\x1b]0;{title}\x07`) to reflect the current application state. The title MUST be updated whenever the TUI state changes.

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

### Requirement: Title updates are deduplicated
The system SHALL only write the title escape sequence when the computed title differs from the last written title, to avoid unnecessary stdout writes.

#### Scenario: State unchanged across event loop iterations
- **WHEN** the TUI event loop runs and the computed title is the same as the previously written title
- **THEN** no title escape sequence SHALL be written to stdout

#### Scenario: State changes between loop iterations
- **WHEN** the TUI event loop runs and the computed title differs from the previously written title
- **THEN** the new title escape sequence SHALL be written to stdout

### Requirement: Terminal title is saved on TUI startup
The TUI SHALL save the current terminal title before setting its own title, by writing a CSI 22;2 t (push title) escape sequence to stdout.

#### Scenario: TUI starts with dynamic tab title enabled
- **WHEN** the TUI launches with `dynamic_tab_title` set to `true`
- **THEN** the system SHALL push the current terminal title onto the title stack before writing any AoE title

#### Scenario: TUI starts with dynamic tab title disabled
- **WHEN** the TUI launches with `dynamic_tab_title` set to `false`
- **THEN** the system SHALL NOT push any title onto the stack

### Requirement: Title is cleared on TUI exit
The TUI SHALL restore the terminal tab title to its pre-launch state during terminal teardown, using the xterm title stack mechanism (CSI 22;2 t to push, CSI 23;2 t to pop).

#### Scenario: Normal TUI exit
- **WHEN** the user quits the TUI (via `q` or `Ctrl+c`)
- **THEN** the terminal tab title SHALL be restored to the title that was active before AoE launched

#### Scenario: Panic or abnormal exit
- **WHEN** the TUI exits due to a panic (handled by the existing panic hook)
- **THEN** the terminal title restore SHALL be included in the panic cleanup sequence

### Requirement: Tab title can be disabled via configuration
The tab title feature SHALL respect the `dynamic_tab_title` configuration field. When disabled, no title escape sequences SHALL be written.

#### Scenario: Feature disabled in config
- **WHEN** `dynamic_tab_title` is set to `false` in the global config
- **THEN** the TUI SHALL NOT write any title escape sequences during its lifecycle

#### Scenario: Feature enabled (default)
- **WHEN** `dynamic_tab_title` is `true` (the default)
- **THEN** the TUI SHALL update the terminal tab title based on state changes
