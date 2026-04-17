## ADDED Requirements

### Requirement: Shell left pane starts in project_path

When a session is created with the Shell tool as the left (main) pane, the shell process SHALL start with its working directory set to the session's `project_path`. The command SHALL include an explicit `cd` to `project_path` before launching the interactive shell, ensuring the directory is correct even if login shell profiles change the cwd.

#### Scenario: Shell session starts in specified directory
- **WHEN** a user creates a new session with tool set to "shell"
- **AND** the Path field is set to `/some/project/path`
- **THEN** the shell SHALL start with its working directory as `/some/project/path`
- **AND** the tmux pane command SHALL include `cd '/some/project/path' &&` before the `exec` of the shell binary

#### Scenario: Shell session with special characters in path
- **WHEN** a user creates a shell session with a path containing spaces or quotes
- **THEN** the path SHALL be properly shell-escaped in the `cd` command
- **AND** the shell SHALL start in the correct directory

#### Scenario: Consistency with right pane shell behavior
- **WHEN** a session is created with Shell on both the left pane and right pane
- **THEN** both panes SHALL use the same `cd {project_path} && ... exec {shell}` pattern
- **AND** both panes SHALL start in the same working directory
