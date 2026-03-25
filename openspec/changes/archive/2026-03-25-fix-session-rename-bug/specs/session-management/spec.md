## MODIFIED Requirements

### Requirement: Session Rename
When a session is renamed (title change), the system SHALL rename the underlying tmux session to match the new title. The rename operation MUST NOT interrupt running processes. The tmux session name MUST be generated from the NEW title and session ID using `Session::generate_name()`.

The system SHALL construct the tmux Session reference using the OLD title before mutating the instance, ensuring the `tmux rename-session` command targets the correct (existing) session.

This applies to both same-profile renames and cross-profile renames.

#### Scenario: Same-profile rename updates tmux session name
- **WHEN** a user renames a session from "OldTitle" to "NewTitle" within the same profile
- **THEN** the tmux session SHALL be renamed from `aoe_OldTitle_<id>` to `aoe_NewTitle_<id>`
- **AND** all processes in the session SHALL continue running uninterrupted
- **AND** the status poller SHALL find the session under the new name

#### Scenario: Cross-profile rename updates tmux session name
- **WHEN** a user renames a session and moves it to a different profile
- **THEN** the tmux session SHALL be renamed to reflect the new title
- **AND** all processes in the session SHALL continue running uninterrupted

#### Scenario: Rename when tmux session does not exist
- **WHEN** a user renames a session whose tmux session has already exited
- **THEN** the rename SHALL update only the stored instance title
- **AND** no tmux rename command SHALL be attempted
