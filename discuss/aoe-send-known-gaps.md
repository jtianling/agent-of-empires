# `aoe send` 的两个已知缺口

记录于 2026-08-01.  两条都是 jt 在真机上用 `aoe send` 给 Cross Agent Team 的 codex 会话发消息时遇到的.  **目前不修**, 先记录, 免得下次重新发现一遍.

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
