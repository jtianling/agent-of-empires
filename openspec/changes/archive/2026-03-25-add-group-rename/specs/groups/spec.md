## ADDED Requirements

### Requirement: Groups can be renamed with cascading updates
AoE SHALL support renaming a group by changing its full path. Renaming a group SHALL cascade path updates to all descendant groups and all sessions whose `group_path` matches or descends from the old path. When the target path already exists, AoE SHALL offer to merge the groups.

#### Scenario: Rename a leaf group
- **WHEN** a group at path `work/frontend` is renamed to `work/ui`
- **THEN** the group path SHALL be updated to `work/ui`
- **AND** all sessions with `group_path = "work/frontend"` SHALL be updated to `group_path = "work/ui"`

#### Scenario: Rename a parent group cascades
- **WHEN** a group at path `work` is renamed to `projects`
- **AND** descendant groups `work/frontend` and `work/backend` exist
- **THEN** descendant paths SHALL update to `projects/frontend` and `projects/backend`
- **AND** all sessions under the old paths SHALL have their `group_path` updated accordingly
