## Why

AoE 创建 Codex + Shell 双 pane session 时, 右侧 shell 会先经过固定的 `bash -lc`, 即使用户的 shell 是 zsh.  这会执行无关的 Bash 登录配置, 当前已确认会卡在 conda hook, 导致目标 shell 永远没有启动.  同时, 继承 session 目录的右侧 shell 不写入 durable slot, 因此 `Shift+C` 重启和冷恢复只覆盖左侧 agent, 与用户选择受管理双 pane session 的预期不符.

## What Changes

- 右侧 shell 使用与主 pane 一致的用户 shell 启动语义, 不再固定经过 Bash 登录 shell.
- 用户明确创建或添加的 shell pane 无论是否继承 session 目录, 都在启动时写入 durable slot.
- `Shift+C` 及其他基于 slot 的重启模式覆盖右侧 shell, 并保持左侧 Codex 的 xats identity key 恢复语义不变.
- 冷恢复根据 durable slots 重建 Codex + Shell 双 pane, 恢复 shell 的目录和 pane 拓扑.
- 增加隔离 tmux socket 的执行级与 E2E 覆盖, 验证 shell 启动不读取无关 Bash 登录配置, 双 pane 都会重启和恢复.

## Capabilities

### New Capabilities

无.

### Modified Capabilities

- `right-pane`: 明确 shell right pane 的启动器和完整 restart/recovery 生命周期.
- `agent-session-store`: 删除同目录 shell pane 的 slotless 例外, 用户明确创建的 shell pane 必须在启动时占用 durable slot.
- `tui`: `%` 添加的 shell pane 与 New Session 的右侧 shell 一样始终作为受管理 pane 进入 durable slot.

## Impact

- 主要影响 `src/session/instance.rs` 的 extra shell command builder、launch-time slot 写入和 shell relaunch 路径.
- `src/tui/app.rs` 与 `src/cli/session.rs` 继续通过同一 managed pane launch path 获得新语义.
- shell pane 将占用一个现有的四 slot 配额, 这是为了换取可重启、可冷恢复和可恢复布局的明确生命周期.
- 已经运行且没有 durable slot 的旧 shell pane 不会被自动收编, 以免误认用户手工创建的 raw split.  用户需要通过受管理入口重建该 pane 或重建 session 后, 才会获得新的 lifecycle.
- 不修改 xats 协议、identity key 格式、Codex bootstrap 或公开 CLI 参数.
