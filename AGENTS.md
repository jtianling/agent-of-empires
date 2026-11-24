# Repository Guidelines

> `CLAUDE.md` is a symlink to this file. Do not edit `CLAUDE.md` directly -- edit `AGENTS.md` instead.

## Project Structure & Module Organization

- `src/main.rs`: binary entrypoint (`aoe`).
- `src/lib.rs`: shared library code used by the CLI/TUI.
- `src/cli/`: clap command handlers (e.g., `src/cli/add.rs`, `src/cli/session.rs`).
- `src/tui/`: ratatui UI and input handling.
- `src/session/`: session storage, configuration, and group management.
- `src/tmux/`: tmux integration and status detection.
- `src/process/`: OS-specific process handling (`macos.rs`, `linux.rs`).
- `src/docker/`: Docker sandboxing and container management.
- `src/git/`: git worktree operations and template resolution.
- `src/update/`: version checking against GitHub releases.
- `src/migrations/`: versioned data migrations for breaking changes (see below).
- `tests/`: integration tests (`tests/*.rs`).
- `tests/e2e/`: end-to-end tests exercising the full `aoe` binary (see E2E Tests below).
- `docs/`: user-facing documentation and guides.
- `scripts/`: installation and utility scripts.
- `xtask/`: build automation workspace.

- `contrib/`: community-maintained integration files (e.g., OpenClaw skill). Checked by `cargo xtask check-skill` in CI.

## Build, Test, and Development Commands

- `cargo build` / `cargo build --release`: compile (release binary at `target/release/aoe`).
- `cargo build --profile dev-release`: faster optimized builds for local development. Skips LTO for quicker compile times while still producing an optimized binary. Use `--release` for final/CI builds.
- `cargo run --release`: run from source; requires `tmux` installed.
- `cargo check`: fast type-checking during development.
- `cargo test`: run unit + integration tests (some tests skip if `tmux` is unavailable).
- `cargo fmt`: format with rustfmt (run before pushing).
- `cargo clippy`: lint (fix warnings unless there’s a strong reason not to).
- Debug logging: `AGENT_OF_EMPIRES_DEBUG=1 cargo run` (writes to `debug.log` in app data dir).

## Settings & Configuration

- **Every configurable field must be editable in the settings TUI.** When adding a new config field to `SandboxConfig`, `WorktreeConfig`, etc., you must also:
  1. Add a `FieldKey` variant in `src/tui/settings/fields.rs`
  2. Add a `SettingField` entry in the corresponding `build_*_fields()` function
  3. Wire up `apply_field_to_global()` and `apply_field_to_profile()`
  4. Add a `clear_profile_override()` case in `src/tui/settings/input.rs`
- Profile overrides (`*ConfigOverride` structs in `profile_config.rs`) must also include the new field with merge logic in `merge_configs()`.

## Coding Style & Naming Conventions

- Prefer "let the tools decide": keep code `cargo fmt`-clean and `cargo clippy`-clean.
- **Never use emdashes (—)** in documentation or comments.
- Rust naming: `snake_case` for modules/functions, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Keep OS-specific logic in `src/process/{macos,linux}.rs` rather than sprinkling `cfg` checks.
- Do not be concerned about maintaining backwards compatibility. You should not assume that it needs to be backwards compatible, but you should mention when you make a change that breaks backwards compatibility.
- Add comments where they aid understanding, but remove obvious ones before finishing:
  - **Keep**: comments explaining non-obvious formulas, layout structure documentation, or "why" something is done
  - **Remove**: section headers that just name what the next line does (e.g., `// Render buttons` before `render_buttons()`), or comments restating the code

## Testing Guidelines

- Use unit tests in-module (`#[cfg(test)]`) for pure logic; use `tests/*.rs` for integration tests.
- Tests must be deterministic and clean up after themselves (tmux tests should use unique names like `aoe_test_*` or `aoe_e2e_*`).
- Avoid reading/writing real user state; prefer temp dirs (see `tempfile` usage in `src/session/storage.rs`).
- New features touching TUI rendering, CLI subcommands, or session lifecycle should consider adding an e2e test.

### E2E Tests

Full-binary end-to-end tests live in `tests/e2e/`. They exercise `aoe` through tmux (for TUI tests) and as a subprocess (for CLI tests). Run them with:

```sh
cargo test --test e2e              # all e2e tests
cargo test --test e2e -- --nocapture  # with screen dumps on failure
```

The test harness (`tests/e2e/harness.rs`) provides `TuiTestHarness` with:
- `spawn_tui()` / `spawn(args)` -- launch `aoe` in a detached tmux session with isolated `$HOME`
- `send_keys(keys)` / `type_text(text)` -- send keystrokes or literal text
- `wait_for(text)` -- poll the screen until text appears (10s timeout, panics with screen dump)
- `capture_screen()` / `assert_screen_contains(text)` -- one-shot screen assertions
- `run_cli(args)` -- run `aoe` as a subprocess with the same env isolation

TUI tests auto-skip if tmux is not installed. Docker-dependent tests use `#[ignore]` and require a running daemon. All tests use `#[serial]` for tmux isolation.

#### Recording E2E Tests

E2E tests can produce asciinema recordings (`.cast`) and GIF files automatically. This is useful for PR reviews and documenting TUI behavior.

- **Local**: `RECORD_E2E=1 cargo test --test e2e -- --nocapture` (requires `asciinema` and `agg` on `$PATH`). Outputs go to `target/e2e-recordings/`.
- **CI**: Add the `needs-recording` label to a PR. The `E2E Recordings` workflow will run the tests with recording enabled and upload GIF artifacts.

## Commit & Pull Request Guidelines

- Branch names: `feature/...`, `fix/...`, `docs/...`, `refactor/...`.
- Commit messages: use conventional commit prefixes (`feat:`, `fix:`, `docs:`, `refactor:`).
- PRs: follow the template in `.github/pull_request_template.md`. When creating PRs via `gh pr create`, read the template first and use its structure for the `--body` argument. Include a clear “what/why”, how you tested (`cargo test`, plus any manual tmux/TUI checks), and screenshots/recordings for UI changes.

## Git Configuration

- Do not modify git configuration (e.g., `.gitconfig`, `.git/config`, `git config` commands) without explicit user approval.
- The one exception: adding a new remote to fetch a contributor's fork during PR code review is allowed without asking.

## Local Data & Configuration Tips

- Runtime config/data location:
  - **Linux**: `$XDG_CONFIG_HOME/agent-of-empires/` (defaults to `~/.config/agent-of-empires/`)
  - **macOS/Windows**: `~/.agent-of-empires/`
- Keep user data out of commits. For repo-local experiments, use ignored paths like `./.agent-of-empires/`, `.env`, and `.mcp.json`.

## Worktree 创建耗时排查经验

- 创建慢时分开测量 git worktree 建造, `on_create` 和 agent 启动, 不只记录总时长.  CLI add 与 TUI 的 hook 来源不同, CLI 基线不能当作全局 hook 已执行的证据.
- 本机全局 `clone-node-modules.sh` 曾递归查找所有 `node_modules`, 把 `.e2e-test` 的历史构建副本和 `.next` 缓存也复制到新 worktree.  2026-11-06 的 share worktree 额外复制了 65 份测试副本依赖, 现场目录时间线跨度约两分钟.
- 经 jt 确认, 2026-11-06 已在依赖扫描中按目录名排除 `.e2e-test` 和 `.next`, 包括嵌套缓存.  正常 root/workspace 依赖, skills 与 `.sharp-runtime` 保留, 不通过跳过整个 agent 配置目录提速.
- 测试依赖克隆 hook 时, 使用临时仓库对照正常根目录, workspace 与测试副本, 明确断言哪些依赖被复制.  不重跑真实项目 hook 来探测耗时, 不自动清理用户 worktree 或修改全局 hook.

## Tmux Session Safety (never disrupt live AoE sessions)

AoE runs as a developer tool, so the machine compiling and testing this code is
usually the same machine running real AoE-managed tmux sessions on the default
tmux socket. Destructive tmux operations can therefore kill the user's live work.

Hard rules for both production code AND tests:

- **Never run `tmux kill-server`** -- not in code, not in tests, not in an ad-hoc shell command. It tears down every session on the socket. There is no scenario in this codebase where killing the whole server is correct. (This rule has been violated and killed live sessions; treat it as absolute.)
- **Never kill or iterate sessions by prefix on a shared socket.** Do not do "list `aoe_*` sessions, then kill them" against the default socket. Production teardown/reconcile MUST target a specific managed session by its exact name (`kill-session -t <exact-name>`), never a pattern sweep.
- **Tests that touch tmux MUST use an isolated socket**, never the default socket. A full `cargo test` run MUST NOT kill, reap, or disturb any live AoE session. If you add a tmux-touching test, route it through `TuiTestHarness`, or use the `crate::tmux::isolate_tmux_socket()` helper for unit tests.
- **`TMUX_TMPDIR` ALONE IS NOT ISOLATION.** When the test runner (or your shell) is itself inside a tmux session, `$TMUX` is set, and every `tmux` client connects to that CURRENT server and *ignores `TMUX_TMPDIR` entirely*. A `kill-session`/`kill-server` then hits the developer's live server. To isolate you MUST also remove `$TMUX` and `$TMUX_PANE` (e.g. `Command::env_remove("TMUX")` / `std::env::remove_var("TMUX")`), the way the e2e harness and `isolate_tmux_socket()` do. Before relying on any isolation, verify with `env | grep TMUX` that `$TMUX` is unset for the tmux client.
- Any new `kill-server`/`kill-session`/pattern-based cleanup in a sweep helper (even guarded by `#[cfg(test)]`) must be scoped to the e2e-private socket, the way `reap_stale_aoe_test_sessions` already is.

If a change needs broad tmux cleanup, scope it to a private socket or to exact session names. When in doubt, do not destroy sessions. **Do not run ad-hoc `tmux` probe/cleanup commands on a machine with live AoE sessions while `$TMUX` is set.**

## xats agent 切换经验

- 旧版 AoE 直接启动的 agy 首次注册得到 `custom/Antigravity` 和 `delivery=none`, 需恢复重启一次切换启动器.  新版新建时即使用 `xats-agy --defer-registration`, 由官方 stdio 在首次注册时绑定 `antigravity-pty`.  2026-11-23 独立验收确认首次注册前后 agy PID 和 conversation UUID 一致, 连续两封唤醒回信, 草稿保留, 全程只有一次启动.

- 2026-11-23 用两个不同团队的同名成员验证 Antigravity 回信.  当前 xats 的 `send_message_by_id` 按发送方团队核对收件人, 即使使用正确的 `from_agent_id`, 跨团队也会得到 `unknown_recipient`.  跨团队按原邮件的 `from_name` 和 `from_team` 调用 `send_message`, 已验证只投递给目标成员.

- Antigravity 原生 TUI 由 `agy` 启动.  同目录可同时存在多个 conversation, 恢复须使用目标窗格的 `SessionStart.conversationId` 和 `--conversation <UUID>`.  2026-11-23 在 1.5.14 同目录先创建 A、再创建 B, 已实测按 A 的 UUID 恢复只显示 A 的合成标记.  `agy acp` 提供的定址接口属于 ACP 会话, 不能据此推断原生 TUI 也能被空闲唤醒.

- xats pane lookup/release 返回 `tmux_response_invalid` 时, 同时检查整个 socket 的 pane 元数据.  2026-11-13 的 10, 11 号目标均正常, 但其他 pane 的 `pane_pid=0` 和空 TTY 被 xats 当作非法记录, 导致所有目标查询失败.  未运行 pane 不应污染活目标的校验, 目标自身未运行仍须明确拒绝释放, 不清理无关 pane 来绕过问题.
- 增加可切换 agent 时, 必须核对 xats 的跨类型交接合同.  OpenCode 同类型 runtime 的 reserve/commit 会拒绝仍注册为 Codex 的 identity key, 仅扩展 AoE 选项与保存 tool 不足以完成切换.
- 不得通过跳过类型检查, 删除旧身份或直接修改 xats 数据库完成切换.  保留同一 agent 身份及 name/team/key, 新 runtime 验证并提交交接后才完成切换.
- AoE slot 的 `xats_runtime_generation` 与 xats 的 `register_generation` 是不同的 fence, 不得混用.  停止旧 pane 前持久保存源状态和交接请求, 支持失败后恢复.
- 诊断 TUI session 编号时, 读取 tmux 的 `@aoe_index` 映射, 不按 `sessions.json` 数组位置猜编号.  2026-11-04 的 13 号实际是 `monkeys-blue-7`, 并非数组第 13 条.
- 新控制面接口未在共享 daemon 激活时, 不得让已支持的 Claude/Codex 非托管切换无条件依赖它.  2026-11-05 的 18 号切换因 handoff prepare 路由未部署而回归.  兼容处理仅限明确路由不存在且无已接受交接证据, 不绕过服务端校验, OpenCode 和已受控身份不得降级.
- Codex 配置快照可能位于几十 MB 历史文件的较早位置.  2026-11-05 的 9, 11 号右 pane 恢复因只扫描最后 4 MB 误判缺少快照而失败.  持久化恢复需流式扫描并选取精确 thread 的最后完整快照, 不因固定 tail 窗口丢失已知状态.
- 普通 xats Codex 的原生 thread UUID 必须在界面启动前持久化, 不依赖模型完成 xats 注册.  2026-11-13 的正常创建会话因注册挑战过期而没有保存 UUID, 冷恢复又先创建 pane 后验证, 最终只剩 shell.  冷恢复须先只读验证, provider 和权限准备仍在旧订阅退出后执行.
- 2026-11-22 重启前核对发现旧式直接启动的 Codex 可以正常聊天但缺少 slot UUID.  经用户授权, 对精确且空闲的窗格发送原生 `/status` 取得当前 UUID, 再用同一 app-server 的 `thread/read` 核对 UUID 和 cwd.  正常退出后仍须等线程状态变为 `notLoaded` 再显式补绑; 同号窗格的旧注册可能属于另一身份.
- Codex 索引和服务端可以返回逻辑 `.jsonl` 路径, 而磁盘仅有 `.jsonl.zst`.  冷恢复先核对全部所需历史的线程、目录和设置, 再创建任何窗格.  13 份现场压缩历史均有可读快照; 只核对索引记录或线程存在性不足以确认设置可恢复.
- tmux 重建后会复用 pane ID, 不能仅凭同名 pane 判断 xats 身份归属.  2026-11-14 的 11, 12 号删除被前一 server 代次的身份残留阻止, 需同时核对 socket, server PID, server start time, 绑定时间和旧载体进程.  目标 key 无 holder 不代表 pane 没有其他绑定, 不得绕过 `pane_unverifiable` 强删.
- 已死 pane 不能作为 live-only xats release 目标, 也不能只凭 key lookup 的 `not_found` 静默跳过.  该接口不排除尚无 agent row 的 prepared managed claim.  2026-11-15 的 26 号删除因此被 `pane_not_running` 阻止; 自动处理需把 dead 目标留在原批次, 由服务端核验物理缺席和 managed claim, 删除前再次核对代次, 保护已经迁移的旧身份.

- 2026-11-16 jt 明确调整删除规则: xats 身份释放属于尽力清理, 远端拒绝, 不可用或协议错误不能阻止 AoE session 删除.  保留未释放身份并在 debug.log 记录具体原因; 本地归属变化, 进程及资源清理错误仍严格处理.  此规则取代此前要求身份释放失败阻止 session 删除的产品语义.

## macOS 上的 AoE 短命进程排查

- 2026-11-22 只读核对到 20 个 AoE 进程, 其中 18 个是各自跟踪单个 session 的 Codex 标题监视器, 另有 1 个通知监视器和 1 个无子命令的 AoE.  按进程名计数会把不同职责混在一起.  当时 20 个进程映射了 5 个不同的可执行文件 inode, 仅 3 个与磁盘安装文件一致.
- `run_codex_title_monitor` 稳态每轮执行 1 次 `has-session`, 2 次 `show-options` 读取 owner PID 和标题, 1 次 `capture-pane`, 完成后休眠 500ms.  18 个监视器的这四项合计理论上限约 144 次/秒.  独立 10 秒采样捕获到 626 次读取上述两项的 `show-options`.  整机约 2500 个新进程/秒和 launchd 拒绝日志是不同口径, AoE 占比及系统拒绝原因尚未确定.
- 验证降频时按父进程角色, tmux 子命令和目标统计相同时间窗的唯一子进程, 同时记录存量监视器的可执行文件代次.  保留标题等待标记, 切换活动 pane 和脱离首页后的行为.
- 修复后的 Codex 标题由每个 profile 的通知监视器维护.  新监视器接管时清除对应 session 的旧 owner 选项, 让旧标题进程自行退出.  仅替换磁盘上的二进制不会激活这次交接.
- 2026-11-22 新版接管后, 主界面和通知监视器的运行映像均与安装文件 inode 一致, 活动标题监视器从 18 个降为 0 个.  同一采样器的两个 10 秒窗口中, AoE 直接 tmux 子进程从修前 1411 / 1497 个降至 225 / 265 个, 两轮旧 owner/title 查询均为 0.  另一采样器独立观察到 195 个及 0 次旧字段查询.  这些是采样下界, 全机进程创建率的变化不能全部归因于此次修复.
- 后续用 kqueue 的 NOTE_FORK 按父 PID 计数, 同期 10.002 秒观察到共享通知监视器 5332 次 fork 通知.  进程快照会漏掉短命和未完成 exec 的子进程.  本机同一条只读 ps 查询的两组对照中, 使用名称各观察到 21 次 fork 通知, 使用绝对路径各 1 次.  21 仅是 ps 在该 PATH 的实测, 不能直接套用到 tmux.  性能验收同时核对父进程 fork 事件与成功命令分类, 前者也可能因内核合并事件而少计.
- Linux ARM64 / tmux 3.4 独立 E2E 中, 将通知清单、抓屏和恢复对账分别批量执行后, 通知监视器在约 10 秒内启动的 tmux 客户端从 27 / 90 / 300 个降为 6 / 6 / 18 个, 分别对应 1 个空闲会话、8 个空闲会话和 8 个报告 Running 的会话.  这是容器内的直接命令计数, macOS 实际 fork 降幅须在新监视器接管后另测.
- 2026-11-22 02:47, 确认 macOS 主界面与通知监视器都已加载 R5 安装映像.  三个连续 10 秒窗口观测到通知监视器 65 / 72 / 86 个 NOTE_FORK 事件, 同时主界面均为 0.  独立复测两个对齐窗口为 86 / 65 个, 对比上一版约 533 次/秒, 监视器的观测速率下降约 98.6%.  同期 xats binding/lookup 共 74 / 67 次, 覆盖所有调用方, 无法据此判断 AoE 的缓存命中率.
- tmux 3.4 的 display-message 会把 0x1e 输出成字面量反斜线和 036.  抓屏分帧使用每批随机生成的可打印整行标记.  E2E 手动启动 monitor 时必须传入 harness 的 TMUX_TMPDIR, 因为 profile 的 -L 优先于继承的 TMUX.  验证附着期间的恢复时要确认客户端实际连到了目标 session.
- 隔离环境中的 tmux 3.4 长连接对照: 同一个 control-mode 客户端连续执行 120 轮查询、抓屏和标题写入, PID 与启动时间保持一致.  直接附到业务 session 时, 普通用户离开后附着数仍为 1; 改附独立观察 session 后, 可用 pane ID 跨 session 操作, 业务附着数正常归零, server 上额外保留 1 个 session.  未附着的单次查询在 stdin 保持打开时仍返回 %exit.  这是机制验证, R5 仍使用批量短命令.
- 长连接版本使用每个 profile 的专用 `aoe-observer-*` session.  正常业务客户端的计数和导航应排除带 `@aoe_control_observer` 标记的观察客户端.  Linux ARM64 / tmux 3.4 的独立 ptrace 验收覆盖空闲、运行、多 pane 通知和附着期间的恢复写入, 五个稳态窗口均为零子进程创建和零 exec.  启动、断线重连及退出单独计数.
- 2026-11-22 macOS 现场独立验收: 主界面和通知监视器的运行映像均匹配长连接版安装文件.  启动后三个 10 秒窗口中, 两个 AoE 进程及其各自的常驻 tmux 子进程均观测到 0 次 NOTE_FORK, 窗口结束后 tmux PID 保持一致.  同期整机进程唯一编号仍有增长, HTTP 请求来源未在此次测量中核验.
- 控制连接中写通知 option 使用精确目标 `=session:`.  直接把 tmux 返回的展示字节与内部原始长度比较, 会在值包含美元符号等字符时误判.  等长通知更新用服务端字面值比较验证, 同时确认无变化时不重复写入.

## Tmux Keybinding Lifecycle

AoE attaches to managed tmux sessions via `attach-session`. Keybindings are set up on attach and cleaned up on TUI exit.

When adding or modifying tmux key bindings, session options, or attach behavior:

1. **Key binding setup**: All keybindings are configured in `setup_session_cycle_bindings()` which runs before every `attach-session` call.
2. **Key binding cleanup**: `cleanup_session_cycle_bindings()` runs on TUI exit to unbind all AoE-specific keys.
3. **Checklist for every keybinding change**: (a) binding added in `setup_session_cycle_bindings()`? (b) cleanup in `cleanup_session_cycle_bindings()`? (c) status bar hint matches actual availability? All three must be verified.

## Terminal Title Handling

- Treat outer terminal title handling and tmux pane title handling as separate concerns.
- For the AoE TUI itself, use the stable title `Agent of Empires`. Do not reintroduce per-view or per-dialog dynamic TUI titles unless the spec is updated first.
- When AoE starts, preserve the pre-launch terminal title and restore it on normal exit and panic cleanup. For Alacritty, prefer the title stack sequences already used in `src/tui/tab_title.rs`.
- For AoE-managed tmux sessions, make the outer terminal title follow the active pane title through session-scoped tmux options such as `set-titles` and `set-titles-string=#T`.
- Preserve the existing `sets_own_title` split in `src/agents.rs`: agents that manage their own titles should keep doing so, and AoE should only manage pane titles for agents that do not.
- Do not mutate unrelated global tmux defaults just to make title propagation work. Scope title changes to AoE-managed sessions.
- If you change terminal-title behavior, add or update runtime coverage for tmux title propagation and title restoration, and manually sanity-check the behavior in Alacritty when feasible.

### Code-Level Implementation Notes

- Keep TUI title-writing helpers in `src/tui/tab_title.rs`. Add low-level escape-sequence writes there instead of scattering raw OSC/CSI strings through the TUI.
- `src/tui/mod.rs` owns the TUI title lifecycle. Startup should save the incoming title and set the stable AoE title; normal teardown and panic cleanup should restore the saved title.
- `src/tui/app.rs` should only reapply the stable AoE title when returning from attach flows. Use `terminal.backend_mut()` for title writes in attach/detach paths, not direct `std::io::stdout()` calls.
- `src/tmux/status_bar.rs` owns tmux session-scoped title passthrough. If attached sessions need to affect the outer terminal title, wire that through tmux options there rather than adding new TUI polling logic.
- `src/tui/status_poller.rs` owns AoE-managed pane-title refresh for agents with `sets_own_title = false`. If waiting/running states should affect pane titles, change that file instead of teaching the TUI to infer agent titles.
- `src/agents.rs` is the source of truth for whether an agent sets its own title. Update `sets_own_title` there before adding any special-case title logic elsewhere.
- If existing sessions need new tmux title behavior without being recreated, refresh session options through `src/session/instance.rs` attach helpers and the CLI attach path in `src/cli/session.rs`.
- Prefer session-scoped tmux option changes such as `set-option -t <session> ...` or `set-window-option -t <session> ...`. Avoid `-g` unless the behavior is intentionally global.
- Add focused unit tests for title helpers near the implementation, and add integration or e2e coverage in `tests/e2e/` for tmux title passthrough or restoration behavior.

### Reference Docs

- Alacritty escape sequence support: `https://raw.githubusercontent.com/alacritty/alacritty/master/docs/escape_support.md`
- tmux title and pane-title behavior: `https://github.com/tmux/tmux/wiki/Advanced-Use#pane-titles-and-the-terminal-title`
- tmux manual reference for `set-titles`, `set-titles-string`, and `allow-set-title`: `https://man7.org/linux/man-pages/man1/tmux.1.html`
- xterm control sequences for OSC title setting and CSI `22`/`23` title stack save/restore: `https://invisible-island.net/xterm/ctlseqs/ctlseqs.pdf`

## Data Migrations

When making breaking changes to stored data (file locations, config schema, etc.), use the migration system in `src/migrations/` instead of adding fallback/compatibility logic to the main code.

**Why**: Keeps the main codebase clean. Legacy transition logic is isolated and clearly marked as such.

**How it works**:
1. A `.schema_version` file in the app directory tracks the current version
2. On startup, `migrations::run_migrations()` runs any pending migrations in order
3. Each migration bumps the version after completion

**Adding a new migration**:

1. Create `src/migrations/vNNN_description.rs`:
   ```rust
   use anyhow::Result;

   pub fn run() -> Result<()> {
       // Migration logic here
       Ok(())
   }
   ```

2. Update `src/migrations/mod.rs`:
   ```rust
   mod vNNN_description;

   const CURRENT_VERSION: u32 = NNN;  // bump this

   const MIGRATIONS: &[Migration] = &[
       // ... existing migrations ...
       Migration { version: NNN, name: "description", run: vNNN_description::run },
   ];
   ```

**Guidelines**:
- Migrations must be idempotent (safe to run multiple times)
- Use `tracing::info!` to log what's happening
- Platform-specific migrations should use `#[cfg(target_os = "...")]`
- Test migrations by creating the old state manually and verifying the transition
- Before finishing any feature request, make sure that you have run cargo fmt, clippy, and tests.
- `docs/cli/reference.md` is auto-generated by `cargo xtask gen-docs`. Do not edit it by hand -- update the clap help text in `src/cli/` instead and re-run the generator. CI checks that this file is in sync.

## Design System
Always read `DESIGN.md` before making any visual or UI decisions.
All font choices, colors, spacing, and aesthetic direction are defined there.
Do not deviate without explicit user approval.
In QA mode, flag any code that doesn't match DESIGN.md.
