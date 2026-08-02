## MODIFIED Requirements

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
