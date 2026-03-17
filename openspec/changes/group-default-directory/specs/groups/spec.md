## MODIFIED Requirements

### Requirement: Group entity has default_directory field

The Group entity gains a new optional field:

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Display name (last path segment) |
| `path` | `String` | Full slash-delimited path (e.g. `work/clients`) |
| `collapsed` | `bool` | Whether the group is collapsed in the TUI |
| `default_directory` | `Option<String>` | Default project directory for new sessions in this group |

The `default_directory` field SHALL be serialized in `groups.json` with `#[serde(default)]` for backward compatibility.

#### Scenario: Group with default_directory serializes and deserializes
- **WHEN** a group has `default_directory` set to `/home/user/project`
- **AND** the group is saved to `groups.json`
- **THEN** the JSON SHALL include the `default_directory` field
- **AND** loading the same JSON SHALL restore the `default_directory` value
