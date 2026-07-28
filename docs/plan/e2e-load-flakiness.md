# E2E 高负载不稳定治理

## 背景

2026-07-28 一天之内, 三方 (开发 / reviewer / tester) 各自独立撞到同一现象: **全量 e2e 跑挂一条, 单独跑必过**.  累计六次, 每次是**不同的**测试:

| 撞到的人 | 测试 | 单跑结果 |
|---|---|---|
| tester | `multi_agent_session::tracking_caps_at_four` | 过 |
| tester | `xats_identity::clean_restart_reuses_the_same_identity_key` | 过 |
| reviewer | `identity_key_survives_a_clean_recovery` | 过 |
| reviewer | `at_clean_recovery_of_a_multi_slot_shell_command_instance` | 过 (后查出是真竞态, 已修) |
| 开发 | `clean_recover_preserves_nested_layout_while_launching_fresh` | 5/5 过 |
| 开发 | `agent_session_store::pane_live_upserts_by_tmux_pane` | 4/4 过 |

其中只有一条 (AT-4) 查出是测试自身的真竞态 —— 它只等 slot 0 回写就去读全部 slot, 可能读到"新 slot0 + 旧 slot1", 已用 `wait_for_slots_within()` 修掉.  其余五条至今没有定性.

实测频率: `cold_start_recovery` 模块串行跑 7 轮挂 1 轮; 挂的那轮的测试单独跑 5/5 全过.

## 问题

这不是"偶尔有条测试不稳"的量级问题, 而是**它污染了每一轮验收的信噪比**.  今天走了七轮 code review, 每一轮都要额外花一次往返来回答同一个问题: 这次挂的是真回归, 还是又一次噪声?  三方都必须重复"单独复跑 + 对比 diff 有没有碰它"这套动作.

更糟的是它会**诱导错误的免责**.  一条挂了的测试单跑过了, 很容易被写成"负载抖动, 与本次改动无关"—— 但今天六次里确实有一次是真竞态.  把噪声当默认解释, 早晚会放过一个真 bug.

## 需求

- [ ] 定性: 把剩余五条各自的失败断言抓下来 (现在只有 `test result` 行, 细节没留), 区分是"等待条件写得太紧"还是"被测行为在高负载下真的不同"
- [ ] 收敛: 让 e2e 的等待判据统一走**正面证据**而非固定超时.  仓库里已经有这个模式的正例 —— `wait_for_slots_within` 等的是"所有持久化 slot 都属于 live 集合", 而不是"等 N 秒"
- [ ] 兜底: 失败时保留足够的现场 (屏幕快照 / agent_slot 行 / live pane 列表), 让"单跑过了"不再是唯一能拿到的信息
- [ ] 单测同类问题一并处理: `src/tmux/session.rs` 里创建 tmux session 的单测, 首次 `display-message` 可能落在 socket 可连之前.  已在 `test_capture_pane_screen_excludes_scrollback` 用轮询等待修掉一例, 同一模式在该文件其它单测里仍在

## 待决策

要不要单开一个 openspec change 来做.  2026-07-28 决定**押后**, 先把 auto-confirm / CAT 那批过审提交.

## 相关

- 测试清理纪律: 见 `AGENTS.md` "Tmux Session Safety" 和本目录 `test-cleanup-on-failure.md` 中记录的 Drop guard 做法
