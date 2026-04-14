## 1. Add right pane YOLO helper

- [x] 1.1 Add `right_pane_needs_yolo()` method to `NewSessionDialog` in `src/tui/dialogs/new_session/mod.rs`. Returns `true` when `right_pane_tool_index > 0`, the tool is not "shell", and the tool's YOLO mode is not `AlwaysYolo`.

## 2. Fix has_yolo condition

- [x] 2.1 Update `has_yolo` in `src/tui/dialogs/new_session/mod.rs` (handle_key field index calculation, ~line 773) to OR in `self.right_pane_needs_yolo()`.
- [x] 2.2 Update `has_yolo` in `src/tui/dialogs/new_session/render.rs` (~line 40) to OR in `self.right_pane_needs_yolo()`.

## 3. Tests

- [x] 3.1 Add unit test: shell left pane + code agent right pane produces `has_yolo = true` (YOLO checkbox visible).
- [x] 3.2 Add unit test: shell left pane + "none" right pane produces `has_yolo = false`.
- [x] 3.3 Add unit test: shell left pane + shell right pane produces `has_yolo = false`.
- [x] 3.4 Add unit test: code agent left pane + "none" right pane produces `has_yolo = true` (existing behavior preserved).
