## 1. tmux 提交能力

- [x] 1.1 在 `src/tmux/session.rs` 增加按 pane id 提交字面文本的函数, 复用 `Session::send_keys` 已有的 `tmux send-keys -l` 写法, 末尾提交一次 Enter
- [x] 1.2 为该函数补单元测试, 覆盖参数拼装; 不得在测试中触碰真实 tmux server

## 2. 打通 "key 是否为本次新建" 的信息流

- [x] 2.1 让 `Instance::ensure_xats_identity_key` 报告本次是否铸造了新 key
- [x] 2.2 让 `Instance::ensure_slot_identity_keys` 报告哪些 slot 的 key 是本次铸造的
- [x] 2.3 更新两者的既有调用点, 使其编译通过且行为不变
- [x] 2.4 补单元测试: 首次调用报告为新建, 再次调用报告为复用; 持久化失败时不得报告为复用

## 3. auto-confirm 承载就绪信号与条件提交

- [x] 3.1 把 `auto_confirm_panes` 的入参由裸 pane id 扩展为携带 "是否需要恢复身份" 标记的条目
- [x] 3.2 仅在 `shows_claude_input_prompt` 为真的 settle 分支上提交 `reconnect`; 需要恢复身份的 pane 在已知确认屏全部答完后不得提前 settle
- [x] 3.3 每个 pane 至多提交一次, 且只提交给调用方本次启动的 pane
- [x] 3.4 提交失败只记 warn, 不使 session 失败, 不影响同批其他 pane
- [x] 3.5 pane 直到超时都未就绪时不提交, 保持既有的 "不报错, 留可交互" 行为
- [x] 3.6 补单元测试证明需要恢复身份的 pane 答完全部已知确认屏后仍等待 ready, 不会漏发 `reconnect`

## 4. 五个调用点各自提供判据

- [x] 4.1 单 pane 首次启动路径 (`instance.rs` 约 1449 行) 传入 `ensure_xats_identity_key` 的结果
- [x] 4.2 单 pane respawn 路径 (`instance.rs` 约 2269 行) 传入 `ensure_xats_identity_key` 的结果
- [x] 4.3 多 pane restart 路径 (`instance.rs` 约 2354 行) 按 slot 传入 `ensure_slot_identity_keys` 的结果
- [x] 4.4 多 pane recovery 路径 (`instance.rs` 约 2536 行) 按 slot 传入 `ensure_slot_identity_keys` 的结果
- [x] 4.5 `auto_confirm_launched_pane` (`src/tui/app.rs`, `src/cli/session.rs` 调用) 服务新增 pane, 恒定传入 "新建" 从而不提交

## 5. 排除项验证

- [x] 5.1 单元测试: Codex pane 不提交
- [x] 5.2 单元测试: 未开启 Cross Agent Team 的 Claude pane 不提交
- [x] 5.3 单元测试: sandboxed session 不提交
- [x] 5.4 单元测试: fork 与 new-from-selection 产生的 pane 因 key 为新建而不提交
- [x] 5.5 单元测试: 同一次多 pane 重启中, 复用 key 的 slot 提交而其新建 key 的 sibling 不提交; 调用方未报告的 slot 按无 key 处理

## 6. e2e 覆盖

- [x] 6.1 评估在 `tests/e2e/` 增加一条覆盖 "重启后自动提交 reconnect" 的用例; 必须走 `TuiTestHarness` 私有 socket, 并同时清除 `TMUX` 与 `TMUX_PANE`
- [x] 6.2 若判定无法在隔离环境下稳定复现, 在 tasks 中记录该结论与理由, 不得留下会连到真实 tmux server 的测试

### 6 的结论: 本次不落 e2e 用例, 留作后续

**结论**: 用例在技术上可构造, 但本次不提交, 因为它无法在本次实施环境中被执行一次.

**可构造性**: 走 `TuiTestHarness` 私有 socket, 用 tool 仍为 `claude` 的 command override 挂一个 stub 脚本, 由它渲染 `shows_claude_input_prompt` 认得的输入框 (可复用 `src/session/testdata_real_claude_ready.txt`) 并把收到的 stdin 追加到文件; 先 start 让 slot 铸出 key, 再 restart 使其走复用分支, 最后断言该文件出现 `reconnect`.

**不提交的理由**: 本次实施环境的 tmux server 上挂着用户几十个真实工作中的 AoE session, 硬约束禁止执行任何触碰 tmux 的测试 (验证手段只有 `cargo build` / `check` / `clippy` / `fmt`).  一条从未运行过的 e2e 用例进入仓库, 其失败会在 CI 上被归因到本变更, 而它的红是"没写对"还是"功能不对"无从分辨 —— 这比暂时没有覆盖更糟.

**当前替代覆盖**: 触发判据与全部排除项 (Codex / 未开启 Cross Agent Team / sandboxed / fork 与 new-from-selection) 由 `src/session/instance.rs` 的单元测试覆盖; 提交时机所依赖的 `shows_claude_input_prompt` 判定沿用既有的真实截屏 fixture 测试, 未新增识别逻辑.  未覆盖的部分是"就绪后确实把文本送进了 pane"这一段真实链路.

**未留下任何会连到真实 tmux server 的测试**: 新增测试为纯逻辑与临时目录 sqlite, 不启动 tmux.

## 7. 收尾

- [x] 7.1 `cargo fmt`
- [x] 7.2 `cargo clippy` 无新增警告
- [x] 7.3 `cargo build` 通过
- [x] 7.4 确认未引入 `tmux kill-server`, 未引入按前缀批量操作 session 的代码或测试
