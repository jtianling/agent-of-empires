## Why

一个 session 一旦以 claude 或 codex 创建, 就无法更换 agent, 想换只能删掉重建, 这会丢掉 session 在 xats 中的 name/team 身份, 其他 agent 需要重新认识它.  需要一个在原地切换 agent 的入口, 同时保住 xats 身份.

## What Changes

- Home 视图新增快捷键 `A` / `a`: 对选中 session 弹出 agent 选择弹窗(目前只支持 claude 与 codex 互切).
- 确认后把 session 的 agent 切换为目标 agent, 并对 session 做 fresh 重启(不 resume, 相当于清空上下文).  `A` 重启后 attach 进入 session(同 `C`), `a` 留在 home 后台重启(同 `c`).
- 切换作用于 session 的所有 claude/codex pane(每个 slot 都换成目标 agent); 其他类型的 pane(如 shell)保持原 agent.
- 每个 pane 保留原有 xats identity key, 新 agent 通过它恢复原 agent 在 xats 中的 name/team.
- 帮助覆盖层, 状态栏提示与按键文档同步新增 `A` / `a`.

## Capabilities

### New Capabilities
- `session-agent-switch`: 在 home 视图原地切换 session 的 agent(claude <-> codex), fresh 重启并保留 xats 身份.

### Modified Capabilities

## Impact

- `src/tui/home/input.rs`, `src/tui/home/mod.rs`, `src/tui/home/render.rs`: 按键, 弹窗状态与渲染.
- `src/tui/dialogs/`: 新增 agent 选择弹窗.
- `src/session/instance.rs`: 新增切换 agent 的实例方法(tool, command, extra_args, 会话身份字段).
- `src/db/mod.rs`: 新增按实例切换 slot agent 的存储方法(清会话, 清 pane_live 捕获与 codex_binding, 保留 identity key).
- `src/tui/components/help.rs`, `docs/quick-start.md`, `docs/guides/workflow.md`: 按键说明.
