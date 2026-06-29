# Pre-existing e2e 失败跟踪

## 已解决 (2026-06-29)

全部解决.  收尾时实际红的是 21 个 (本文最初记录的 12 个 + 之后 `d2159e90` 引入的 9 个).  最终干净 `cargo test --test e2e`: 72 passed / 0 failed / 1 ignored (Docker sandbox).  分四组:

- **组 A (11), `[all]` 表头**: fork 的 home view 一直是单-profile, `[all]` 全-profile 视图是 upstream PR #427/#454 的功能, 合并时只吃进了测试没采纳实现.  修: `profile_picker` 7 个 `wait_for("[all]")` -> `("[default]")`; `unified_view` 4 个测的是不存在的多-profile 聚合, 删除文件.
- **组 B (6) + 组 C (3), R restart / V cold-start 后 pane 命令不是 agent 命令**: 真凶是 `d2159e90` (fix-resume-preserves-launch-context, 在本文最初记录之后) 把 slot resume 统一走 `build_pane_command`, primary(slot 0) 现在会应用 `has_command_override` -> fixture 的 `--cmd-override sh` 把 primary 命令变成 "sh" 抑制 resume.  `d2159e90` 是正确生产修复, 是 fixture 自相矛盾.  修: `add_and_start` 去掉 `--cmd-override sh`, 改 `-c <tool>` 让实例 tool 对齐 slot-0 agent, 加 `install_tool_stub` 让 primary pane 存活.
- **组 C 深层真 bug (生产)**: cold-start 恢复 sibling 时, sibling pane 用空命令建出来是死 pane, `get_pane_pid` 对死 pane 返回 0, `kill_pane_process_tree_target` -> `kill_process_tree(0)` -> 从 launchd 递归杀**整个系统进程树** (含 aoe 自己), 恢复中断.  这是被 slot-0 失败长期掩盖的潜伏灾难性安全 bug.  修: `src/process/mod.rs` 两层防御 (`get_pane_pid` 过滤 pid<=1; `kill_process_tree` 入口 `is_unsafe_kill_root` guard).
- **组 C 3-pane 测试侧顺序**: seed 的 live reconcile 按 pane-index 给 slot 编号, 与测试 `slots[]` 数组序不同; 断言改成按 DB 每个 slot 自己的 native 对齐核对 (顺序无关) + 全集等值.
- **组 D (1), codex ✋**: 测试拿 codex 空输入提示符当 Waiting, 但语义已故意反转为 Idle (单元测试 `test_detect_codex_status_idle_at_prompt` 为证).  修: 注入内容换成真 Waiting 触发 `Press enter to confirm`.

以下为原始记录 (历史, 仅 12 个), 保留备查.

---

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
