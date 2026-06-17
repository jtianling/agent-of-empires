# Agent Session 持久化 -- w03 / w04 路线文档

承接已合并的 `agent-session-recording` (w01+w02).  这份文档把后续两个 change 的设计前提, 缺口, 实现位置, 任务大纲固化下来, 方便随时开工.  开工方式见末尾.

## 一, 现在已有的地基 (w01+w02, 已在 main)

录制骨架已经落地, 后续两步是"消费"这些记录, 不再需要重新设计存储.

- SQLite store `aoe.db` (profile 作用域), 见 `src/db/mod.rs`:
  - `agent_slot(instance_id, slot 0..3, agent, native_session_id, cwd, tmux_pane, last_seen_at)` -- 耐久的 "pane -> agent 的 native session id" 映射, **重启电脑后仍在**.  读 API: `read_slots_for_instance(instance_id)`.
  - `pane_live(tmux_pane, agent, native_session_id, cwd, updated_at)` -- 易失的最新捕获.
  - `events(...)` -- 状态/adopt/capture 流水.
- 捕获链: 状态 hook 从 stdin JSON 读 `session_id`, 按 `$TMUX_PANE` 经隐藏子命令 `aoe __record-pane` 入库 (`src/hooks/mod.rs`, `src/cli/record_pane.rs`).  对 AoE 启动的和**手动跑的** agent 都生效.
- reconciler: `src/db/reconcile.rs::reconcile_all`, 挂在 `src/tui/status_poller.rs` 的 tick 上.  `assign_slots` 是 **sticky** 的 (已占 slot 的 pane 黏住, 新 pane 只填空闲 slot, 主 `@aoe_agent_pane` 钉 slot 0, 4 上限).
- 多 agent/session 模型: 任意 pane 的 agent 被 adopt, CLI 动作 `aoe session add-agent-pane <session>` (`src/cli/session.rs:170`).
- Agent 能力声明: `src/agents.rs::ResumeConfig` (退出键序列, 输出 token 正则, resume flag), 以及 `session_id_flag` (claude = `--session-id {}`).  claude `--resume <id>`, codex `resume <id>`.

关键结论: **w03/w04 需要的"哪个 pane = 哪个 agent 的哪个 session id"已经被持久记录了**.  剩下的是"用它来重启/重建".

## 二, 现状 restart 流程 (w03 要改的对象)

- `R` 键 -> `Action::RespawnAgentPane(id)` (`src/tui/home/input.rs:781`, 处理在 `src/tui/app.rs:494`).
- 只作用于**单个** `@aoe_agent_pane` (`src/tmux/utils.rs:778::get_agent_pane_id`).
- 优雅 resume 路径: `can_gracefully_restart` (`src/session/instance.rs:405`) -> `initiate_graceful_restart` -> tick 驱动的 `tick_pending_resume` (`src/tui/app.rs:273`) -> 抓 token -> `respawn_agent_pane_with_resume`.
- 状态机 `pending_resume: Option<PendingResume>` 是 **per-instance 单个** (`src/session/instance.rs:249`).
- 不支持 resume 的 agent (gemini/vibe/cursor/shell/opencode) 走 kill + fresh.

---

## 三, w03: 统一 R 键 restart (resume 全部 pane)

建议 change 名: `restart-resume-all-panes`

### 目标
按一次 `R`, 把这个 session 里被追踪的全部 agent pane (1~4 个) 一起优雅重启, 每个用自己的 `native_session_id` 走 `--resume`, 而不是只重启 `@aoe_agent_pane`.  对应用户原话 "一个 session 的统一按 R 键的 Restart, 应该一样可以恢复".

### 现状缺口
1. `RespawnAgentPane` 只处理一个 pane; 要扩成"遍历该 instance 的全部 `agent_slot` 行".
2. `pending_resume` 是单个状态机; 多 pane 并行优雅退出需要 **per-slot/per-pane** 的状态 (一个 `Vec<PendingResume>` 或 `HashMap<slot, PendingResume>`).
3. 每个 pane 的 agent 可能不同 (`agent_slot.agent`), resume 命令要按各自 agent 的 `ResumeConfig` 构造.
4. 非 resume agent 的 pane 优雅降级为 fresh 重启.

### 已定的决策 (探索阶段确认过)
- R = 整个 session 全部 pane 一起 resume (不是只当前聚焦 pane).
- N <= 4.

### 关键实现位置
- `src/tui/app.rs::RespawnAgentPane` 分支: 改为读 `store.read_slots_for_instance(&inst.id)`, 对每个 slot 取 `tmux_pane` + `agent` + `native_session_id`, 在**对应 pane** 里发退出键并以 resume 命令重生.  注意现状是在 `@aoe_agent_pane` 单 pane 上操作, 要泛化到任意 `tmux_pane`.
- `src/session/instance.rs`: `pending_resume` 状态机改成可并行多份 (per slot).  `tick_pending_resume` 相应改成遍历推进每个 pane 的退出/抓取/重生阶段.
- resume 命令构造: 复用现有 `respawn_agent_pane_with_resume` 的逻辑, 但目标 pane 与 agent 参数化.  `native_session_id` 已经在库里, 优先直接用它 (`claude --resume <id>` / `codex resume <id>`), 比"退出时从 pane 输出抓 token"更稳, 可以省掉抓取阶段.
- 状态展示: 多 pane restart 期间的 `Restarting` 状态聚合呈现.

### 开放问题
- 是否保留一个"只重启当前聚焦 pane"的单独键 (探索时用户选了 R=全部, 但可加第二键).  默认先不加.
- 优雅退出超时 / 部分 pane resume 失败时, 失败的 pane 走 fresh, 不影响其他 pane.
- 是否需要 `pane_live` 作为 `native_session_id` 的兜底 (当某 slot 快照滞后).  见 w01 design 的 open question.

### 验收要点 (e2e, 参考 `tests/e2e/multi_agent_session.rs` 模式)
- 一个 session 起 2~3 个 pane 各跑 agent (用 `aoe __record-pane` 注入 capture + reconcile 落 `agent_slot`), 按 R 后断言: 每个 pane 的进程被重启, 且新命令含对应 `--resume <native_session_id>`.
- 非 resume agent 的 pane 走 fresh, 不报错.
- 注意 tmux 多 pane 的窗口尺寸 (复用 `harness.resize_window` + tiled), sqlite CLI 读加 `.timeout` (w01 踩过的坑).

---

## 四, w04: 冷启动手动恢复

建议 change 名: `cold-start-session-recovery`

### 目标
重启电脑后 tmux 全没了, 但 `agent_slot` 还在.  AoE 启动时把这些 session 标为"可恢复", 用户**手动**逐个进入/按键触发: 重建 tmux session, 按 slot 重建 pane, 每个 pane 用 `native_session_id` 走 `--resume` 拉起对应 agent.  对应用户原话 "重启了电脑以后, 都还可以恢复" + 冷启动选"手动逐个恢复".

### 现状缺口
1. 现状重启电脑后, AoE 从 `sessions.json` 加载 instance 配置, 但**不知道每个 session 内有哪些 agent pane 及其 session id** -- 现在 `agent_slot` 提供了, 但没有"用它重建多 pane + 各自 resume"的路径.
2. 没有"可恢复"标识 (有 `agent_slot` 记录但 tmux 已死) 与对应的恢复入口键.
3. 重建后 pane id 全是新的, 要重新写回 `agent_slot.tmux_pane` 并重设 `@aoe_agent_pane`.

### 已定的决策
- 冷启动 = **手动逐个恢复** (列出可恢复 session, 用户进哪个/按键才重建+resume), 不自动全量拉起.

### 关键实现位置
- 恢复入口: TUI 里对"有 `agent_slot` 记录但 tmux session 不存在"的 instance 标记可恢复, 加一个恢复动作键 (注意 keybinding 生命周期清单: `setup_session_cycle_bindings` / `cleanup_session_cycle_bindings` / 状态栏提示, 见 AGENTS.md).
- 重建逻辑: 复用 `src/session/instance.rs` 的 session 创建 + `add_agent_pane` 的 split 逻辑.  按 `read_slots_for_instance` 的 slot 顺序: slot 0 = `@aoe_agent_pane` 主 pane, 其余 split 出来; 每个 pane 以对应 agent + `--resume <native_session_id>` (cwd 用 `agent_slot.cwd`) 启动.
- 重建后回写: 新 pane id -> `upsert_agent_slot` 更新 `tmux_pane`; 重设 `@aoe_agent_pane`.
- w04 复用 w03 的"按 agent 构造 resume 命令"逻辑, 所以 **w04 依赖 w03 先做**.

### 开放问题
- 死 pane 的 slot 是否可回收 (w01 当前是耐久保留, sticky 不回收).  冷启动恢复时是否需要"丢弃过期记录"的策略 (比如 last_seen_at 太老).
- 某个 agent 的 `native_session_id` 在 agent 侧已失效 (jsonl 被清) 时, `--resume` 会失败 -> 降级 fresh 并提示.
- 恢复时的 cwd / worktree / sandbox 上下文是否需要随 instance 配置一起还原 (与 `sessions.json` 的 worktree_info/sandbox_info 协同).

### 验收要点 (e2e)
- 构造一个有 `agent_slot` 记录的 instance, 模拟 tmux 不存在 (kill session), 触发恢复, 断言: tmux session + 正确数量的 pane 被重建, 每个 pane 命令含对应 `--resume <id>`, `agent_slot.tmux_pane` 被更新为新 pane id.
- 真实 claude 全链路同 w01: 受授权门限制, 默认用 `__record-pane` 注入 + 命令断言, 不强跑真 claude 改真实 `~/.claude`.

---

## 五, 依赖与顺序

```
w01+w02 (已合并)
   └─> w03 restart-resume-all-panes        <- 先做, 产出"按 agent 构造 resume 命令 + 多 pane 操作"的能力
          └─> w04 cold-start-session-recovery   <- 复用 w03 的 resume-launch 逻辑
```

w03 与 w04 共享"针对任意 pane, 按其 agent 用 native_session_id 拉起 resume"这块核心, 建议在 w03 就把它抽成可复用函数, w04 直接调用.

## 六, 如何开工

每个 change 独立走一遍标准 pipeline (与 w01 相同):

```
/jt-os-propose restart-resume-all-panes --e2e --archive
# 完成后
/jt-os-propose cold-start-session-recovery --e2e --archive
```

或先 `/openspec-explore` 把上面的开放问题敲定再 propose.  e2e 阶段可继续找 team 的 `aoe-tester` 做独立验收, 在 `~/workspace/test` 隔离 HOME 下跑.

注意: 在 worktree 下工作时 openspec 命令在 worktree 目录执行; 需要新分支用新 worktree, 不要单独 checkout 分支.
