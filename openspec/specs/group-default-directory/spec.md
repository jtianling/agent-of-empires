# Capability Spec: Group Default Directory

**Capability**: `group-default-directory`
**Created**: 2026-03-17
**Status**: Draft

## Overview

Groups can capture and remember a default project directory based on the first session created
within them. When creating new sessions, the dialog pre-fills the path field from the group's
default directory, reducing repetitive path entry for users who work on the same project across
multiple sessions in a group.

## Requirements

### Requirement: Group captures default directory from first session
When the first session is created in a new group, the system SHALL store that session's project path as the group's `default_directory`. This value SHALL persist in `groups.json` alongside other group fields.

#### Scenario: First session in a new group sets default directory
- **WHEN** a user creates a session with a group name that does not yet exist
- **AND** the session's project path is `/home/user/my-project`
- **THEN** the new group SHALL have `default_directory` set to `/home/user/my-project`
- **AND** the value SHALL be persisted in `groups.json`

#### Scenario: Second session in existing group does not change default directory
- **WHEN** a user creates a session in a group that already has a `default_directory` set
- **AND** the new session uses a different project path
- **THEN** the group's `default_directory` SHALL remain unchanged

#### Scenario: Group created without a session has no default directory
- **WHEN** a group is auto-created as an intermediate parent (e.g., `work` when `work/clients` is created)
- **THEN** the intermediate group SHALL have `default_directory` set to `None`

### Requirement: New session dialog pre-fills path from group default directory
When opening the new session dialog with a group that has a `default_directory`, the path field SHALL be pre-filled with that directory instead of the launch directory.

#### Scenario: Dialog opens with pre-selected group that has default directory
- **WHEN** the user opens the new session dialog
- **AND** the pre-selected group has `default_directory` set to `/home/user/my-project`
- **THEN** the path field SHALL show `/home/user/my-project`

#### Scenario: Dialog opens with pre-selected group that has no default directory
- **WHEN** the user opens the new session dialog
- **AND** the pre-selected group does not have a `default_directory`
- **THEN** the path field SHALL show the launch directory (current behavior)

#### Scenario: User types a group name that matches existing group with default directory
- **WHEN** the user is in the new session dialog
- **AND** the user types or selects a group name that matches an existing group with `default_directory` set
- **AND** the user has not manually edited the path field
- **THEN** the path field SHALL update to the group's `default_directory`

#### Scenario: User-edited path is not overwritten by group default
- **WHEN** the user has manually edited the path field to a custom value
- **AND** the user then types or selects a group with a `default_directory`
- **THEN** the path field SHALL retain the user's manually entered value

### Requirement: Existing groups without default directory are backward compatible
Groups persisted before this feature SHALL continue to work without any default directory behavior. The `default_directory` field SHALL default to `None` when absent from `groups.json`.

#### Scenario: Loading legacy groups.json without default_directory field
- **WHEN** the system loads a `groups.json` that was saved before this feature
- **AND** the group entries do not contain `default_directory`
- **THEN** each group SHALL have `default_directory` set to `None`
- **AND** no errors SHALL occur during deserialization
