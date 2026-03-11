## 1. Main Loop Refactor

- [x] 1.1 Move the cache refresh logic (`refresh_*_preview_cache_if_needed`) from `src/tui/home/render.rs` into `src/tui/home/mod.rs` as a dedicated `update_caches` method.
- [x] 1.2 Refactor `App::run` in `src/tui/app.rs` to call `home.update_caches` *before* the draw condition check.
- [x] 1.3 Ensure `needs_redraw` is accurately set by `update_caches` when any cache actually changes.

## 2. Animation & Redraw Throttling

- [x] 2.1 Implement a `last_tick_redraw` timestamp in `App` (`src/tui/app.rs`) to track the last time a redraw was triggered by a tick event.
- [x] 2.2 Limit redraws from `tick_dialog` to a maximum frequency of 10Hz (100ms interval).
- [x] 2.3 Ensure user input events (keys/mouse) still trigger immediate redraws (at the end of the loop iteration) without throttling.

## 3. Rendering Stability

- [x] 3.1 Review dialog `render` methods to ensure `Clear` is used on the smallest necessary area and that no state changes occur during rendering.
- [x] 3.2 Add tracing/logs to track `terminal.draw` frequency to identify any remaining redraw-feedback-loops.

## 4. Verification

- [x] 4.1 Verify that the Gemini "thinking" spinner no longer causes the input box and lower screen area to flicker in `tmux`.
- [x] 4.2 Verify that hook execution spinners (⠼) are stable and do not cause global screen artifacts.
- [x] 4.3 Confirm that TUI navigation responsiveness (key handling) is not negatively impacted by the animation throttling.
