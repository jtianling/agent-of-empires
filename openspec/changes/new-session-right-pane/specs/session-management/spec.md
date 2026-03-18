## MODIFIED Requirements

### Requirement: Session creation sets group default directory for new groups
When creating a session that causes a new group to be created, the system SHALL set the group's `default_directory` to the session's `project_path`. This applies only when the group did not exist before the session was created.

The session creation flow SHALL accept an optional right pane tool parameter. When provided, the system SHALL split the tmux session window horizontally after creation and launch the specified tool in the right pane, while maintaining correct `@aoe_agent_pane` tracking.

#### Scenario: Creating session with new group sets default directory
- **WHEN** `create_session()` is called with a `group_path` that does not exist in the group tree
- **AND** the session's `project_path` is `/home/user/project`
- **THEN** after the group is created, its `default_directory` SHALL be `/home/user/project`

#### Scenario: Creating session in existing group does not change default directory
- **WHEN** `create_session()` is called with a `group_path` that already exists in the group tree
- **THEN** the group's `default_directory` SHALL NOT be modified

#### Scenario: Creating session with right pane tool splits window
- **WHEN** `create_session()` is called with a `right_pane_tool` value that is not "none"
- **THEN** after the tmux session is created, the system SHALL split the window horizontally
- **AND** the right pane SHALL launch the specified tool
- **AND** `@aoe_agent_pane` SHALL still reference the original left pane
