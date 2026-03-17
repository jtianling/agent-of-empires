## MODIFIED Requirements

### Requirement: Session creation sets group default directory for new groups
When creating a session that causes a new group to be created, the system SHALL set the group's `default_directory` to the session's `project_path`. This applies only when the group did not exist before the session was created.

#### Scenario: Creating session with new group sets default directory
- **WHEN** `create_session()` is called with a `group_path` that does not exist in the group tree
- **AND** the session's `project_path` is `/home/user/project`
- **THEN** after the group is created, its `default_directory` SHALL be `/home/user/project`

#### Scenario: Creating session in existing group does not change default directory
- **WHEN** `create_session()` is called with a `group_path` that already exists in the group tree
- **THEN** the group's `default_directory` SHALL NOT be modified
