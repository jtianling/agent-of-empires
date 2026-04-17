## 1. Fix shell left pane working directory

- [x] 1.1 In `Instance::build_agent_command()` (`src/session/instance.rs`), for the non-sandboxed shell case (where `self.expects_shell()` is true), prepend `cd {shell_escape(&self.project_path)} &&` to the command string before passing it to `wrap_command_ignore_suspend_with_env`. This mirrors the pattern used by `build_right_pane_command` in `src/tui/app.rs:91-93`.

## 2. Testing

- [x] 2.1 Add a unit test in `src/session/instance.rs` (or the existing test module) that verifies `build_agent_command()` for a shell instance includes `cd '/expected/path' &&` in the generated command string.
