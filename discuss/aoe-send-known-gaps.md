# `aoe send` 的两个已知缺口

记录于 2026-08-01.  两条都是 jt 在真机上用 `aoe send` 给 Cross Agent Team 的 codex 会话发消息时遇到的.  **目前不修**, 先记录, 免得下次重新发现一遍.

2026-10-28 状态更新: 缺口 2 的发送修复已通过独立隔离验收,
下文历史观察与候选原因保留.  缺口 1 的普通 CLI 附加 pane 选取不在本次修复范围内,
受管附加 pane 可使用已有的 `managed pane send` 精确入口.

## 缺口 1: 只能打到主 pane, 够不着右 pane

`src/cli/send.rs:35` 调 `Session::send_keys`, 而它 (`src/tmux/session.rs:277`) 把目标写死成:

```rust
let target = format!("{}:^.0", self.name);
```

即"第一个窗口的 0 号 pane".  所以 `aoe send <会话> <消息>` 永远打在主 pane 上, 右 pane 收不到.

**为什么现在才暴露**: 右 pane 长期是个"启动了就不管"的附属物 —— 它不预注册、不进 slot、不被重启覆盖.  `extra-agent-pane-parity` (2026-07-31) 把它变成一等公民之后, "给某个 pane 发消息"才成为一个有意义的操作.  也就是说这不是回归, 是**新暴露的缺口**.

**修法方向** (未实施): 让 `aoe send` 能指定 pane 或 slot.  会话已经有 `agent_slot` 表 (slot 0 = 主 pane, slot 1.. = 额外 pane), 所以 `--slot N` 是现成的坐标系, 不需要让用户去认 tmux 的 `%NN`.

## 缺口 2: 发出去的消息没有被提交 (jt 实测)

jt 实测: 用 `aoe send` 发给 codex pane 的文本**出现在输入框里但没有被提交**, 要手工回车.

**注意这与代码读起来的样子矛盾** —— `src/tmux/session.rs:288` 明确有:

```rust
// Enter to submit
Self::tmux_send(&target, &["Enter"])?;
```

所以**不是"忘了发回车"**.  待查的是 codex TUI 这条路径为什么没把这个 Enter 当成提交, 候选方向:

- codex TUI 在收到 `send-keys -l <文本>` 之后可能处于某种输入/粘贴模式, 需要先退出才接受提交键;
- tmux 的 `Enter` 发的是 `C-m`, codex TUI 可能期待别的键 (例如某些 TUI 用 `C-j` 或需要 bracketed paste 结束序列);
- 多行文本路径里那个 `ESC + CR` (模拟 Shift+Enter 插入换行) 可能让 TUI 停在换行状态.

**不要仅凭代码里有 `Enter` 就判定这条不成立** —— 观察是真机上的, 代码只说明"发了一个 Enter", 不说明"codex 把它当成了提交".  这正是本轮反复踩到的那类错误 (断言产物而不是断言实际效果).

**验证方向** (未实施): 用一个能打印自己收到什么按键的替身 TUI 接在同一条路径上, 看它到底收到了什么序列; 而不是从 codex 的行为反推.

## 相关

- 两条都不影响 `Shift+C` 重启链路, 所以未纳入 2026-07-31 那三条 change.
- 缺口 1 的坐标系依赖 `agent_slot`, 见 `openspec/specs/pane-session-capture/spec.md`.

---

2026-10-28 补充: managed send 也出现输入后未提交的现象.

steward 在 macOS 的 Codex 0.164.0 remote TUI 中报告, 向 owned session
`9afb9f0e2e554117` 执行 `managed send '/status'` 返回 `sent=true`, 20 秒后
capture 仍显示 composer 中的 `/status`, 没有状态面板.  当时主 pane 已恢复,
底栏显示 `GPT-6.1-Sol high`, 尚未分派业务任务.  此项为 steward 的运行时观察,
本轮未独立复现, 未使用额外按键或真实 tmux 探针验证.

当时 managed session 和 pane 的 send 均调用
`src/tmux/session.rs` 的 `submit_text_to_pane_target`, 先发送 literal 文本,
再单独发送 `Enter`, 两次调用之间没有显式等待.  `sent=true` 只说明发送命令
成功, 不证明 Codex 已提交或执行输入.

本地 Codex 参考源码中的 paste burst 会短暂抑制 Enter 提交, 但也有 slash
command 绕过抑制的测试.  参考源码与安装二进制尚未确认匹配, 因此不能直接
把本次 `/status` 现象归因于 paste burst, 也没有证据证明固定等待可修复.
后续需在隔离环境记录输入事件与实际提交结果, 分别验证普通文本和 slash command.

首次调查仅记录, 未修改发送逻辑.  模型验收仍需区分 TUI 回显与实际请求使用的模型,
不能用 `sent=true` 或 xats `runtime_ready=true` 代替业务验收.

---

同日授权修复后的独立验收结果: 真实 Codex 0.164.0 local/remote TUI 中,
普通 send、managed session send、managed pane send 的文本与 `/status`
基线共 12 项失败; 显式 bracketed paste 后单独 Enter 的对照均通过,
不需要固定等待.  补丁在 `src/tmux/input.rs` 共用该链路,
使用 UUID 具名缓冲区和 stdin 保留字节, `paste-buffer -p -r -d` 成功后才提交.
失败时清理本次缓冲区并保留具体错误, 不操作其他缓冲区或 clipboard.

补丁用例 57 项通过, 覆盖真实请求、slash 状态面板、Unicode、多行、
bracketed on/off 原始字节、Bash、准确目标、独立故障注入与缓冲区隔离.
原始字节保留尾 LF, Codex 请求文本会 trim 尾 LF, 与显式粘贴对照一致.
Claude/OpenCode 发送本轮未构建模型夹具, Kimi 程序缺失, 不宣称这些 runtime 已验收.
此项通过仅证明发送链路, 不证明 managed Codex 创建或 MCP 身份生命周期.

证据报告: `.e2e-test/runs/send-submit-baseline-20261028-aoe-tester/report.html`.
无真实 tmux 的错误回归: `python3 tests/send_cli_smoke.py target/debug/aoe`.
