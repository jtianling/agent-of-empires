# Pre-existing e2e 失败跟踪

发现于 `background-reconcile` change 收尾时 (2026-06-22) 的全量 `cargo test`.  经 aoe-tester 在隔离 HOME 下确定性复现 + 因果排除, 确认这些失败是 **pre-existing** (当前 `main` 本身就有), 与 `background-reconcile` 无关 (该 change 只动 `src/tmux/notification_monitor.rs` 的 `run_notification_monitor`, 不碰下列任何代码路径).

全量 `cargo test` 现状: lib unit 1163 全绿; e2e 12 红 (均为下列 pre-existing).  这 12 个红会让全量 `cargo test` 一直 FAILED, 掩盖未来真实回归, 建议优先处理.

## 一, profile launch 模式漂移 (11 个)

- `profile_picker::*` (7): opens_and_closes / shows_multiple_profiles / switch_profile / create_new_profile / create_esc_returns_to_list / delete_cancel / delete_flow
- `unified_view::*` (4): all_profiles_flat_view / default_view_shows_all_profiles / profile_filter_via_picker / return_to_all_view_via_picker

现象: 全部卡在 `spawn_tui` 后第一个 `wait_for("[all]")` 超时 (`tests/e2e/harness.rs:463` wait_for_timeout, ~10s).  实抓首屏是健康的 home view, 但表头是 `[default]` 单-profile 模式, 不是测试假设的 `[all]` 全-profile 模式 (测试注释写 "Default launch is now all-profiles mode").

待查: 是**实现回归** (默认本应进 `[all]` 全-profile 视图, 现在退回 `[default]`) 还是**测试过时** (默认已故意改成 `[default]`, 测试没跟上).  需先定位 home view 启动时的 launch 模式决策代码, 再决定改实现还是改测试.

## 二, codex waiting 标题图标 (1 个)

- `cli::test_codex_session_waiting_title_uses_hand_icon`

现象: `tests/e2e/cli.rs:477` assert_eq 失败, pane_title 停在 `Codex Wait Title`, 没变成 `✋ Codex Wait Title` (waiting 的手图标没触发).  确定性失败 (连跑 3 次稳定红, 各 5.1-5.3s).

待查: codex 的 ✋ 由 `set_pane_title` (`src/tmux/status_bar.rs:135` <- `:308`, codex title monitor) 或 TUI status_poller 设置.  确认 waiting 状态到 ✋ 标题这条链路是否回归.

## 三, harness 清理缺口 (非测试失败, 但相关)

`spawn_tui` 起的 notification monitor + 私有 socket tmux server 不被 harness 的 `Drop` 清理, 会在私有 socket 泄漏 (默认 `tmux ls` 看不到, 所以默认 socket baseline diff 显示零泄漏).  那批**不** `spawn_tui` 的 agent-session e2e 反而没这问题.  建议给 `TuiTestHarness` 的 Drop 补上私有 socket 的 `kill-server` + monitor 进程回收.

## 证据来源

- aoe-tester 隔离 HOME 复跑这 12 个全红, 确定性 (非随机超时, 非 HOME 内容污染, 非负载 flaky).
- `git diff` 确认 `background-reconcile` 改动 +25 行全在 `run_notification_monitor`; `grep` 确认它不碰 profile launch 模式表头 / pane title.
- 未做 pristine `main` (不含本次改动) 的黄金对照 (需 stash 未提交改动, 按工作树纪律未擅自 stash); 架构解耦 + 确定性失败已使 "本 change 致因" 不成立.

## 建议

单独立 openspec change 修复 (优先 "一", 因为它占 11/12).  修复前先研究根因决定改实现还是改测试.
