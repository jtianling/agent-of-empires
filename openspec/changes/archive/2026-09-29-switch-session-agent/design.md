## Context

Session 的 agent 由 `Instance.primary_pane.tool` 决定, 多 pane 重启则按每个 `agent_slot.agent` 拉起.  代码里没有修改已有 session agent 的入口.  重启已有 `RestartMode::Fresh` 全套路径(respawn / 后台 RestartPoller / cold recovery).  xats identity key 按 slot 持久化且跨重启稳定; xats daemon 按 `(device, identity_key)` 查身份, 不看 agent_type, 所以同一把 key 换 agent 后可以接回原 name/team.

## Goals / Non-Goals

**Goals:**
- `A`/`a` 弹窗切换 claude <-> codex, 确认后 fresh 重启, 保留 xats 身份.
- 复用现有 fresh 重启路径, 不新增重启逻辑.

**Non-Goals:**
- 不支持 claude/codex 以外的 agent, 不支持 sandbox session.
- 不迁移会话内容(切换即清上下文).
- 不改 xats daemon.

## Decisions

1. **先提交, 后重启.**  确认后在事件循环里同步提交切换(实例 + slot 表 + 保存), 再返回 `restart_action(Fresh, post)` 走现有 Action.  理由: 后台 worker 用克隆实例, `apply_restart_results` 只合并会话身份字段, 不带 tool; 先提交则所有路径天然看到新 agent.
2. **存储层单事务 `Store::switch_instance_agent(instance_id, from_agents, target)`**: 对 agent 属于 {claude, codex} 的 slot 改 agent, 清 native_session_id/model/model_fingerprint, 删除这些 slot 的 pane_live 行和 codex_binding 行; 不动 identity key, runtime generation, cwd, yolo, cross_agent_team.  删 pane_live 是为了防止 reconcile 用旧 agent 的捕获把 slot agent 改回去; 删 codex_binding 是因为旧 binding 描述的是已丢弃的会话.
3. **无 slot 时先记 slot 0.**  单 pane 且无 slot 的 session 走 `respawn_single_pane`, 其中 `pane_agent_overriding_instance_tool` 会按 pane 里正在跑的进程把 agent 拉回旧的.  切换时若 tmux session 存在且无 slot, 先用 `record_launched_slot_config_if_absent` 为主 pane 记 slot 0, 使重启走 slot 路径(与 `respawn_single_codex` 同一做法).
4. **实例层 `Instance::switch_agent(target, &Config)`**: `set_primary_pane_config` 更新 tool(经 normalized); command 按 builder 同一规则重算(agent `set_default_command` 则为 binary, 再被 `agent_command_override[target]` 覆盖); extra_args 取 `agent_extra_args[target]`; 清 resume_token/agent_session_id/fork_pending.  identity key 不动.
5. **弹窗**: 新 `SwitchAgentDialog`, 参照 `add_pane.rs` 的工具选择器; `HomeView` 持有 `Option<(SwitchAgentDialog, PostRestart)>`.

## Risks / Trade-offs

- [claude -> codex 的 xats 恢复依赖 daemon 的 recovery poke, 前提是旧 claude 进程已死] -> fresh respawn 先杀进程树再拉起, 满足前提; e2e 无法覆盖真实 daemon, 需线上人工验证一次.
- [提交后重启失败] -> 实例已是新 agent, 错误显示在 session 上, 用户可再按 `c`/`C` 重试; 不回滚, 避免半回滚状态.
