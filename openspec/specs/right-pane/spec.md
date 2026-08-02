# right-pane Specification

## Purpose
Let a session be created with a second tool beside its own, launched into a pane of its own with a working directory of its own.

## Requirements
### Requirement: New session dialog includes right pane tool selector
The new session dialog SHALL include a "Right Pane" field directly below the "Tool" field. This field SHALL always be visible regardless of the main tool selection (including when "shell" is selected). This field SHALL offer the same list of available tools as the main Tool field, prefixed with a "none" option. The default selection SHALL be "none".

#### Scenario: Right pane field displays with none selected
- **WHEN** the user opens the new session dialog
- **THEN** a "Right Pane" field SHALL appear below the "Tool" field
- **AND** the field SHALL show "none" as the selected value

#### Scenario: User cycles through right pane tool options
- **WHEN** the user focuses the "Right Pane" field
- **AND** presses Left or Right arrow keys
- **THEN** the selection SHALL cycle through "none" followed by all available tools (same list as the Tool field)

#### Scenario: Right pane none selection creates session without split
- **WHEN** the user submits the new session dialog with Right Pane set to "none"
- **THEN** the session SHALL be created identically to the current behavior (single pane, no split)

### Requirement: New session dialog includes a right pane path field

The new session dialog SHALL include a "Right Pane Path" field directly below the "Right Pane" field. The field SHALL be displayed only when a right pane tool other than "none" is selected and the session is not sandboxed. An empty value SHALL mean the session's own working directory.

The field SHALL offer the same editing behavior as the session path field: ghost completion of directory names, acceptance of the ghost with Right or End at the end of the input, cursor movement by path segment, `Ctrl+P` to open the directory picker, and the invalid-path indication.

The session path field SHALL NOT be relabelled as a left pane field. It remains the session's own path, which the left pane starts in and which the session's identity, worktree base, group default directory, and sandbox mounts are derived from.

#### Scenario: The field appears with a right pane tool selected
- **WHEN** the user changes Right Pane from "none" to a tool
- **THEN** a "Right Pane Path" field SHALL appear below the Right Pane field
- **AND** changing Right Pane back to "none" SHALL remove it

#### Scenario: An empty value means the session directory
- **WHEN** the user submits the dialog with a right pane tool selected and the right pane path left empty
- **THEN** the right pane SHALL start in the session's working directory

#### Scenario: The directory picker targets the field it was opened from
- **WHEN** the user presses `Ctrl+P` with the right pane path field focused
- **AND** selects a directory
- **THEN** the selected directory SHALL be written into the right pane path field
- **AND** the session path field SHALL be unchanged

#### Scenario: Ghost completion works in the right pane path field
- **WHEN** the user types a partial directory name into the right pane path field that has a unique completion
- **THEN** the completion SHALL be shown as ghost text
- **AND** pressing Right at the end of the input SHALL accept it

### Requirement: Session creation splits tmux window when right pane tool is selected
When the user selects a tool for the right pane, the session creation flow SHALL automatically split the tmux session window horizontally after the main session is created, and launch the selected tool in the new right pane.

The right pane SHALL start in the working directory chosen for it. When no directory was chosen, it SHALL start in the main session's `project_path`, regardless of AoE's own launch directory. That fallback SHALL be resolved at the moment of the split rather than captured when the dialog was submitted, so a session whose directory is decided during creation -- a worktree resolution, a group default directory -- carries the right pane with it.

A chosen directory SHALL be used as given. It SHALL NOT be worktree-resolved, because it is not the session's repository.

The directory the right pane starts in SHALL be recorded on that pane's durable slot, so restart and cold-start recovery return the pane to it rather than to the session's directory.

Every right pane that AoE launches through the New Session flow SHALL receive a durable slot at launch, including a `shell` pane that inherits the session's directory. A managed shell pane SHALL therefore participate in restart and cold-start recovery and SHALL count toward the session's four-slot limit. A raw tmux split that the user creates while attached remains outside this managed lifecycle. See the `agent-session-store` capability for the durable record rules.

A `shell` right pane SHALL use the user's configured POSIX shell as its login wrapper rather than a fixed, unrelated login shell. The final host-side interactive shell SHALL preserve the user's configured shell even when the POSIX wrapper must fall back for fish, nu, or PowerShell. Both shell executable paths SHALL be safely quoted. When the user's shell is zsh, launching the right pane SHALL NOT source Bash login configuration before starting zsh.

#### Scenario: Right pane tool creates horizontal split
- **WHEN** the user submits the new session dialog with Right Pane set to a tool (e.g., "claude")
- **THEN** after the main tmux session is created, the system SHALL execute `tmux split-window -h` targeting the session
- **AND** the right pane SHALL run the selected tool's binary command

#### Scenario: Shell right pane with no chosen directory uses session working directory
- **WHEN** the user creates a new session with path set to `/some/project`
- **AND** the right pane tool is set to "shell"
- **AND** no right pane path is given
- **THEN** the shell in the right pane SHALL start with its working directory set to `/some/project`
- **AND** running `pwd` in the right pane SHALL output `/some/project`

#### Scenario: Shell right pane uses the user's shell directly
- **WHEN** the user's configured shell is `/bin/zsh`
- **AND** the user creates a session whose right pane tool is "shell"
- **THEN** the right pane SHALL reach an interactive zsh through the user-shell launch path
- **AND** Bash login configuration SHALL NOT be sourced before zsh starts

#### Scenario: A non-POSIX user shell survives the wrapper fallback
- **WHEN** the user's configured shell is fish, nu, or PowerShell
- **AND** the user creates a session whose right pane tool is "shell"
- **THEN** the POSIX login wrapper MAY fall back to Bash
- **AND** the final interactive shell SHALL still be the user's configured shell

#### Scenario: Managed shell right pane is durable in the session directory
- **WHEN** AoE launches a shell right pane with no separate right pane path
- **THEN** the pane SHALL receive a durable slot carrying the session directory
- **AND** restart and cold-start recovery SHALL include that shell pane

#### Scenario: Right pane starts in its own chosen directory
- **WHEN** the user creates a new session with path set to `/some/project`
- **AND** the right pane tool is set to "shell"
- **AND** the right pane path is set to `/some/other`
- **THEN** the shell in the right pane SHALL start in `/some/other`
- **AND** the left pane SHALL still start in `/some/project`

#### Scenario: Right pane working directory matches left pane after worktree resolution
- **WHEN** the user creates a new session with a worktree branch specified
- **AND** the right pane tool is set to "shell"
- **AND** no right pane path is given
- **THEN** the right pane's working directory SHALL be the resolved worktree path (same as the left pane), not the original repository path

#### Scenario: A chosen directory is not worktree-resolved
- **WHEN** the user creates a new session with a worktree branch specified
- **AND** the right pane path is set to a directory outside that repository
- **THEN** the right pane SHALL start in that directory as given
- **AND** the left pane SHALL start in the resolved worktree path

#### Scenario: The right pane's slot records its own directory
- **WHEN** a right pane is launched into a directory other than the session's
- **THEN** that pane's durable slot record SHALL carry the directory the pane was launched into
- **AND** the primary pane's slot record SHALL carry the session's directory

#### Scenario: A restart returns the right pane to its own directory
- **WHEN** a session whose right pane was launched into a directory other than the session's is restarted
- **THEN** the relaunched right pane SHALL start in that same directory
- **AND** the relaunched left pane SHALL start in the session's directory

#### Scenario: Right pane command is wrapped to disable Ctrl-Z
- **WHEN** a right pane tool is launched
- **THEN** the tool command SHALL be wrapped with the same `stty susp undef` wrapper used for the main tool

#### Scenario: Right pane has remain-on-exit enabled
- **WHEN** a right pane is created
- **THEN** `remain-on-exit` SHALL be set to `on` at the pane level for the right pane
- **AND** this SHALL NOT affect the main (left) pane's remain-on-exit setting

### Requirement: Agent pane tracking remains correct after right pane split
The `@aoe_agent_pane` session option SHALL continue to point to the main (left) pane after the right pane split. Status detection, health checks, and detach behavior SHALL all target the left pane.

#### Scenario: Status detection targets left pane after split
- **WHEN** a session is created with a right pane tool
- **AND** the agent in the left pane is running
- **AND** `detect_status()` is called
- **THEN** the status SHALL be determined from the left pane content, not the right pane

#### Scenario: Detach from right pane returns to AoE correctly
- **WHEN** a user is viewing the right pane of a split session
- **AND** the user presses `Ctrl+b d` to detach (nested mode)
- **THEN** the user SHALL return to the AoE TUI
- **AND** the session SHALL NOT be killed or recreated on next attach

#### Scenario: Detach from left pane returns to AoE correctly
- **WHEN** a user is viewing the left pane of a split session
- **AND** the user presses `Ctrl+b d` to detach (nested mode)
- **THEN** the user SHALL return to the AoE TUI
- **AND** the session SHALL NOT be killed or recreated on next attach

### Requirement: Right pane works with sandboxed sessions
For sandboxed sessions, the right pane tool command SHALL be executed inside the container, using the same container exec wrapping as the main tool. The right pane SHALL use the same container and container working directory as the main pane.

A sandboxed session SHALL NOT offer a right pane working directory. The agent's directory is decided by the container exec, so a host-side directory would be accepted and then have no effect.

#### Scenario: Sandboxed session right pane runs inside container
- **WHEN** the user creates a sandboxed session with a right pane tool selected
- **THEN** the right pane command SHALL be wrapped with the container's `docker exec` invocation
- **AND** the right pane SHALL use the same container and working directory as the main pane

#### Scenario: Sandboxing hides the right pane path field
- **WHEN** the user opens the new session dialog with a right pane tool selected
- **AND** enables sandboxing
- **THEN** the right pane path field SHALL NOT be displayed
- **AND** disabling sandboxing again SHALL restore it

### Requirement: YOLO field visibility considers both pane tools
The new session dialog SHALL show the "Skip permission prompts" (YOLO mode) checkbox when either the left pane tool or the right pane tool is a code agent that supports opt-in YOLO mode. The checkbox SHALL be hidden only when neither pane has a tool that needs the YOLO option.

#### Scenario: Shell left pane with code agent right pane shows YOLO checkbox
- **WHEN** the user opens the new session dialog
- **AND** selects "shell" as the left pane tool
- **AND** selects a code agent (e.g., "claude") as the right pane tool
- **THEN** the "Skip permission prompts" checkbox SHALL be visible

#### Scenario: Code agent left pane with none right pane shows YOLO checkbox
- **WHEN** the user opens the new session dialog
- **AND** selects a code agent as the left pane tool
- **AND** the right pane is set to "none"
- **THEN** the "Skip permission prompts" checkbox SHALL be visible

#### Scenario: Shell left pane with none right pane hides YOLO checkbox
- **WHEN** the user opens the new session dialog
- **AND** selects "shell" as the left pane tool
- **AND** the right pane is set to "none"
- **THEN** the "Skip permission prompts" checkbox SHALL NOT be visible

#### Scenario: Shell left pane with shell right pane hides YOLO checkbox
- **WHEN** the user opens the new session dialog
- **AND** selects "shell" as the left pane tool
- **AND** selects "shell" as the right pane tool
- **THEN** the "Skip permission prompts" checkbox SHALL NOT be visible

#### Scenario: Changing right pane tool dynamically updates YOLO checkbox visibility
- **WHEN** the user has "shell" as the left pane tool
- **AND** changes the right pane from "none" to a code agent
- **THEN** the "Skip permission prompts" checkbox SHALL appear
- **AND** when changing the right pane back to "none"
- **THEN** the checkbox SHALL disappear

### Requirement: A submit confirms every directory it would have to create

When a submit finds that one or more of the directories it needs does not exist, the dialog SHALL present a single confirmation naming all of them. Confirming SHALL create all of them and proceed with the submit; declining SHALL return to editing without creating any of them.

A single confirmation is required rather than one per field: sequential prompts can leave the first directory created after the user declines the second, which is a state the user did not ask for and cannot see.

#### Scenario: Both directories missing are confirmed together
- **WHEN** the user submits with a session path and a right pane path that both name directories that do not exist
- **THEN** one confirmation SHALL be shown naming both directories
- **AND** confirming SHALL create both and create the session

#### Scenario: Declining creates nothing
- **WHEN** the confirmation for missing directories is declined
- **THEN** no directory SHALL be created
- **AND** the dialog SHALL return to editing

#### Scenario: Only the missing directories are named
- **WHEN** the user submits with an existing session path and a right pane path that does not exist
- **THEN** the confirmation SHALL name only the right pane path

### Requirement: Fork dialog includes a right pane path field

The fork dialog SHALL include a right pane path field alongside its right pane tool selector, with the same meaning and editing behavior as the new session dialog's. An empty value SHALL mean the forked session's working directory, which is the parent's.

#### Scenario: Forked right pane starts in a chosen directory
- **WHEN** the user forks a session with a right pane tool and a right pane path set
- **THEN** the forked session's right pane SHALL start in that directory
- **AND** the forked session's own pane SHALL start in the parent's working directory

#### Scenario: Forked right pane defaults to the parent's directory
- **WHEN** the user forks a session with a right pane tool and no right pane path
- **THEN** the forked session's right pane SHALL start in the parent's working directory
