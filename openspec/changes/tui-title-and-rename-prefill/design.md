## Context

The TUI home screen title currently reads `Agent of Empires [profile]`. This is verbose for a UI element that's always visible. The rename dialog initializes "New title" as empty, forcing users to retype the full title when they only want a small edit.

## Goals / Non-Goals

**Goals:**
- Shorten the TUI title to `AoE [profile]`
- Pre-fill the rename dialog's "New title" field with the current session title

**Non-Goals:**
- Changing the title anywhere else (CLI output, welcome dialog, about text)
- Changing the rename dialog layout or adding new fields

## Decisions

1. **Title change is limited to the home screen border title** (`src/tui/home/render.rs:127`). Other references to "Agent of Empires" (CLI help text, welcome dialog, error messages) remain unchanged since they serve different purposes.

2. **Pre-fill uses `Input::new(current_title)` pattern** already used by `new_group` in the same constructor. This is a one-line change in `RenameDialog::new()` at `src/tui/dialogs/rename.rs:59`.

## Risks / Trade-offs

- Existing rename dialog tests assume `new_title` starts empty. Tests that type a new title and submit will now have both the pre-filled text and typed text concatenated. These tests need to be updated to account for the pre-filled value.
