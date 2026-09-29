## 1. Storage

- [x] 1.1 `Store::switch_instance_agent`: 单事务切换 claude/codex slot 的 agent, 清会话/模型, 删 pane_live 与 codex_binding, 保留 identity key
- [x] 1.2 单元测试: claude->codex 切换, shell slot 不动, identity key 保留, pane_live/codex_binding 被清

## 2. Instance

- [x] 2.1 `Instance::switch_agent(target, &Config)`: 更新 primary pane tool, 重算 command/extra_args, 清 resume_token/agent_session_id/fork_pending
- [x] 2.2 单元测试: claude->codex command 为 `codex`, codex->claude command 为空, 配置 override 生效, identity key 不变

## 3. TUI

- [x] 3.1 新增 `SwitchAgentDialog`(←/→/Tab/Space 切换, Enter 确认, Esc 取消, 预选另一个 agent), 含单元测试
- [x] 3.2 HomeView: `a`/`A` 按键门控(选中 session, 非 Deleting/restart_in_flight, agent 为 claude/codex, 非 sandbox), 不支持时弹 info 提示
- [x] 3.3 确认后提交切换(无 slot 时先记 slot 0, 存储切换, 实例切换, save), 失败则写错误不重启; 成功则返回 `restart_action(Fresh, post)`
- [x] 3.4 渲染弹窗, 纳入"有弹窗打开"判断

## 4. Docs & hints

- [x] 4.1 help 覆盖层与状态栏提示新增 `A`/`a`
- [x] 4.2 `docs/quick-start.md` 与 `docs/guides/workflow.md` 按键表

## 5. Verify

- [x] 5.1 `cargo fmt`, `cargo clippy`, 聚焦单元测试(不跑全量 cargo test)
- [x] 5.2 e2e: 隔离 harness 下按 `a` 切换 stub session, 验证 tool 变更与弹窗交互
