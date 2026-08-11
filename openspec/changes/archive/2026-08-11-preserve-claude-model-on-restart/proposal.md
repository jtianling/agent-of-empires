## Why

AoE 重启一个 claude pane 时, 拼出来的命令里没有任何模型信息, claude 于是回落到自己的默认模型.  用户在会话里用 `/model` 切到 Fable 或 Opus 5, 按 `Shift+R` / `Shift+C` 之后就悄悄变回默认模型, 界面上没有任何提示.

根因是 AoE 完全没有"模型"这个概念: 模型只能作为 `Instance::extra_args` 里的自由文本进入命令行, 而 `/model` 是 claude 的纯 session 内存态, 既不写 `~/.claude/settings.json`, 也不写 `~/.claude.json` 或 `~/.claude/sessions/<pid>.json`, `claude --resume <uuid>` 也不恢复它.  唯一的磁盘痕迹是 claude transcript 里 assistant 条目上的 `message.model`.

## What Changes

- 新增 claude 模型探测: 从 pane 自己的 claude transcript (`~/.claude/projects/<project-hash>/<session-uuid>.jsonl`) 尾部倒读, 取最后一条有效 assistant 条目的 `message.model`.
- 新增 `agent_slot.model` 列, 按 pane (slot) 而非按 instance 持久化观测到的模型.  模型是"座位"属性而不是"对话"属性: `Shift+C` 开新对话后, 该 slot 仍沿用上次观测到的模型.
- `build_base_pane_command()` 在为 claude pane 拼命令时追加 `--model <id>`, 位置在 `extra_args` 之后.  这是所有重启路径的公共出口, 因此 `Shift+R` / `Shift+C` / 小写 `r` / 小写 `c` / cold-start recovery / 多 pane fan-out / fork 一致生效.
- 顺带补上一个既有缺口: `build_base_pane_command()` 的 `!is_primary` 分支原本只拼 `binary + resume_flag` 就返回, 非 primary 的 claude pane 连 `extra_args` 都吃不到; 模型注入对 primary 与非 primary pane 一视同仁.
- reconcile 周期性刷新 `agent_slot.model`, 并按廉价文件指纹跳过未变化的 transcript.

明确不做的事:

- **只覆盖 claude.**  codex / opencode / kimi 各自的"当前模型"读法完全不同, 不在本次范围内.
- **不保留 `[1m]` 变体标记.**  transcript 不记录它.  实测 claude 的报错原文表明 `[1m]` 只影响 Claude Code 对 auto-compact 窗口的假设, 不改变实际运行的模型, 因此接受丢失.
- **完全不在 TUI 展示模型.**  既不进 settings TUI (这是探测出来的运行时状态, 不是配置项, 不需要新增 `FieldKey` / `build_*_fields()` 条目或 `*ConfigOverride` 字段), 也不在 home 列表行加 badge: 左右分屏时列表行本来就窄, 模型名会把行撑爆, 收益不抵成本.  `extra_args` 仍然是用户显式配置模型的地方.

行为变更 (非 BREAKING, 但值得注意): 一旦探测到模型, 该 pane 的每次重启都会带上显式 `--model`, 也就是脱离了 claude 的 default 轨道.  探测值优先于 `extra_args` 里写死的 `--model`, 依赖 claude 的 last-wins 语义.

## Capabilities

### New Capabilities
- `claude-model-continuity`: claude pane 当前模型的探测、按 slot 持久化, 以及在所有启动与重启路径上的注入.

### Modified Capabilities
- `agent-session-store`: `agent_slot` durable record 新增 `model` 列, 并纳入 legacy database 的 schema 自愈.
- `agent-pane-restart`: "Agent launch command is reusable" 枚举的命令组成 (binary, extra_args, yolo flags, env vars, custom instruction) 需要加入模型 flag.

## Impact

代码:

- `src/db/mod.rs`: `ensure_schema()` DDL、`backfill_agent_slot_columns()` 自愈列表、`AgentSlot` 结构与 `upsert_agent_slot` / `read_slots_for_instance`.
- `src/db/claude_transcript.rs` (新): transcript 尾部有界读取与模型解析, 形态对标既有的 `src/db/codex_rollout.rs`.
- `src/db/reconcile.rs`: 在 slot 收敛时刷新 `model`, 带文件指纹缓存.
- `src/session/instance.rs`: `build_base_pane_command()` 注入 `--model`; 可复用 `resolve_claude_session_from_disk()` 的 project-hash 推导逻辑.

本 change 不改动任何 TUI 渲染代码.

不受影响: `agent-resume-restart` / `agent-fresh-restart` / `multi-pane-resume-restart` / `cold-start-recovery` 的既有 requirement 文本仍然成立 (fresh restart 依旧不带 resume flag), 模型注入这条横切规则由新 capability 统一持有, 不在这四份 spec 里各写一遍.

依赖与风险:

- transcript 可以很大 (实测本项目最大 23MB) 且单行最大 268KB, 尾部窗口必须 ≥1MB 并丢弃窗口内第一条可能被截断的行.
- 解析必须过滤 `isSidechain == true` 的子代理条目和 `message.model == "<synthetic>"` 的合成条目.
- 探测失败 (新会话尚无 assistant 消息 / 文件缺失 / 解析错误) 必须保留上一次已知值, 且绝不阻塞或改变重启行为.
