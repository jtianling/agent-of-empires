# Capability Spec: Shell Category

**Capability**: `terminal-category`
**Created**: 2026-03-11
**Status**: Draft

## Purpose

The shell category allows users to launch plain shell sessions (non-agent) through the same
session management interface used for AI coding agents. Shell sessions use the user's default
shell and skip agent-specific features like YOLO mode and worktree management.
## Requirements
### Requirement: Shell tool is available in tool picker
The system SHALL include "shell" as a selectable tool in the new session dialog's tool picker. Shell SHALL appear after "gemini" and before "cursor" in the tool list. The alias "terminal" SHALL resolve to "shell" for backwards compatibility.

#### Scenario: Shell shown in tool picker
- **WHEN** the user opens the new session dialog
- **THEN** "shell" appears in the tool list between "gemini" and "cursor"

#### Scenario: Shell is always available
- **WHEN** the system detects available tools at startup
- **THEN** "shell" is always present regardless of installed binaries

### Requirement: Shell session launches user shell
The system SHALL launch the user's default shell (`$SHELL`, falling back to `/bin/sh`) when creating a shell session, in the specified working directory.

#### Scenario: Create shell session with default shell
- **WHEN** the user creates a new session with tool set to "shell"
- **THEN** a tmux session is created running the user's `$SHELL` in the specified path

#### Scenario: Shell fallback when SHELL is unset
- **WHEN** `$SHELL` is not set and the user creates a shell session
- **THEN** the session falls back to `/bin/sh`

### Requirement: Agent-specific fields hidden for shell

系统 SHALL 隐藏 shell pane 不适用的 agent 专属字段: YOLO Mode 和 Cross Agent Team。  Path 和 Worktree SHALL 保持可用, 因为它们配置 pane working directory, 而不是 agent 权限。

#### Scenario: YOLO mode hidden for shell
- **WHEN** 用户在 New Session 中为一个 pane 选择 `shell`
- **THEN** 该 pane 的 YOLO Mode SHALL 不显示

#### Scenario: Cross Agent Team hidden for shell
- **WHEN** 用户在 New Session 中为一个 pane 选择 `shell`
- **THEN** 该 pane 的 Cross Agent Team SHALL 不显示

#### Scenario: Path and Worktree remain available for shell
- **WHEN** 用户为 primary 或 secondary pane 选择 `shell`
- **THEN** 该 pane 的 Path SHALL 保持显示
- **AND** 该 pane 的 Worktree SHALL 保持显示

### Requirement: Shell has no YOLO mode
The shell tool SHALL NOT have a YOLO/auto-approve mode configured (`yolo: None`).

#### Scenario: Shell YOLO is None
- **WHEN** the shell agent definition is queried for YOLO mode
- **THEN** it returns `None`

### Requirement: Shell status detection returns Idle
The shell tool's status detection function SHALL always return `Status::Idle`.

#### Scenario: Shell status is always Idle
- **WHEN** status detection runs on a shell session's pane content
- **THEN** the result is `Status::Idle`
