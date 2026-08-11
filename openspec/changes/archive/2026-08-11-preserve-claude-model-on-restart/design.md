## Context

今天 AoE 拼 claude 启动命令的唯一出口是 `Instance::build_base_pane_command()` (`src/session/instance.rs` 约 2905 行):

```
build_base_pane_command(agent, resume_token, is_primary)
  ├── !is_primary ──▶  binary [+ resume_flag]              ← 到此 return, 连 extra_args 都没有
  └──  is_primary ──▶  get_tool_command()                  ← "claude" 或 command override
                     + resume_flag / fork_template / session_id_flag
                     + self.extra_args                     ← 模型唯一的入口
```

模型只是 `extra_args` 里的一段自由文本, AoE 不理解它.  用户在会话里按 `/model` 切换之后, AoE 一无所知, 重启时照旧拼回原来的 `extra_args`, claude 于是回落到默认模型.

在本机核过的事实 (不要重新推导):

| 位置 | 是否记录当前模型 |
| --- | --- |
| `~/.claude/settings.json` | 否 |
| `~/.claude.json` (全局 / per-project) | 否, 只有 `additionalModelOptionsCache` 这类缓存 |
| `~/.claude/sessions/<pid>.json` | 否 (16 个字段里没有) |
| transcript 的 assistant 条目 `message.model` | **是, 唯一来源** |
| transcript 里的模型切换事件 | 不存在 (11 种 entry type 全查过) |

`claude --resume <uuid>` 只恢复对话, 不恢复模型, 与用户观测一致.

约束:

- transcript 可以很大.  本项目目录下最大 23 MB, **单行最大 268 KB**.
- 同一个 transcript 里混有子代理条目 (`isSidechain == true`) 和合成条目 (`message.model == "<synthetic>"`), 都会污染结果.
- `[1m]` 不写进 transcript.  claude 自己的报错原文表明它只影响 Claude Code 对 auto-compact 窗口的假设, 不换模型.
- 实测重复 `--model` 是**后者赢** (`claude --model sonnet --model no-such-model-xyz` 报后者的错).

## Goals / Non-Goals

**Goals:**

- claude pane 重启后仍然跑在用户上一次实际使用的模型上, 无论用户是启动时用 `extra_args` 指定的, 还是会话里用 `/model` 切的.
- 所有重启路径行为一致, 不出现"按 `R` 保住了模型, 按 `r` 没保住"这种分裂.
- 探测是纯旁路: 任何失败都不得影响重启本身.

**Non-Goals:**

- 不覆盖 codex / opencode / kimi.  它们各自的"当前模型"读法完全不同 (codex 要读 rollout, opencode 走 REST), 值得单独一个 change.
- 不保留 `[1m]` 变体标记.
- 不把模型做成配置项.  不进 settings TUI, 不进 profile override, 不进 New Session 对话框.
- 不在 TUI 任何地方展示模型.  一度设计过 home 列表行的 dimmed badge, 后来撤掉: 左右分屏时列表行本来就窄, 模型名会把行撑爆.
- 不区分"用户显式选了 X"与"default 恰好解析成 X".  transcript 给不出这个信息.

## Decisions

### D1: 探测源用 transcript, 不用抓屏, 不用 statusLine hook

考虑过三条:

| 方案 | 否决理由 |
| --- | --- |
| 抓 pane 内容 | claude 显示的是 display name (`Opus 5`), 不是 `--model` 的合法值; 且受主题、宽度、截断影响 |
| 接管 `statusLine` hook 读 `.model.id` | 能拿到权威 id, 但要求 AoE 改写用户的 `~/.claude/settings.json` 里的 statusLine.  用户往往已有自己的脚本 (本机就有), 侵入性太强 |
| **读 transcript** | **采用.**  纯读, 零侵入, 进程死了也还在磁盘上, 这一点对 cold-start recovery 尤其重要 |

transcript 还有个现成优势: `resolve_claude_session_from_disk()` (`instance.rs` 约 4273 行) 已经在推导 project-hash 目录名了, 直接复用.

### D2: 尾部有界读取, 窗口 ≥1 MiB

必须从尾部倒读, 不能从头扫 23 MB.  窗口大小由单行最大长度决定: 实测最大单行 268 KB, 256 KiB 窗口会正好切在这种巨型 assistant 消息中间.  取 1 MiB 留足余量.

读到窗口后丢弃第一条 (很可能被截断的) 行, 然后自后向前找第一条同时满足 `type == "assistant"`、`isSidechain != true`、`message.model` 有效且非 `<synthetic>` 的行.

形态对标 `src/db/codex_rollout.rs` 已有的 `.take(4 * 1024 * 1024)` 有界读取, 不引入新模式.

### D3: 存 `agent_slot.model`, 不存 `Instance`

按 slot 而不是按 instance, 有两个理由:

1. 同一 instance 的多个 claude pane 可以各跑各的模型, instance 级别表达不了.
2. `build_base_pane_command()` 的 `!is_primary` 分支本来就拿不到 instance 的 `extra_args`.  把模型挂在 slot 上, 非 primary pane 天然也能拿到, 顺手补上这个既有缺口.

代价: 模型成了"座位"属性而不是"对话"属性.  `Shift+C` 开新对话后, 该 slot 仍沿用上次观测到的模型.  这正是需求要的语义, 但必须在 spec 里写死, 否则 fork / recovery 路径会各自理解一遍.

加列走现成通路: `ensure_schema()` 的 DDL + `backfill_agent_slot_columns()` 的 `ALTER TABLE` 列表, 与 `tmux_pane` / `xats_identity_key` 一模一样.

### D4: 探测值赢, 不解析 `extra_args`

直接把 `--model <id>` 追加在 `extra_args` 之后, 靠 claude 的 last-wins 语义生效.

- 选探测值赢: 用户手动 `/model` 切过, 说明当下意图比启动时写的那句更新.
- 不解析 `extra_args`: 解析要处理 `--model=x` / `--model x` / 引号 / 转义等一堆形态, 换来的只是一个更"整洁"的命令行.  不划算.

代价: "我在 `extra_args` 里写死了 `--model sonnet` 却不生效"会有点反直觉.  接受.

### D5: 改在公共出口, 不特判键位

只改 `build_base_pane_command()` 一处, `Shift+R` / `Shift+C` / `r` / `c` / cold-start recovery / 多 pane fan-out / fork 全部一致生效.

替代方案是只在 `Shift+R` / `Shift+C` 两条路上特判 —— 那样 `r` 和 `R` 行为不同, 更糟.

因此 `agent-pane-restart` 的 "Agent launch command is reusable" 需要把 model flag 纳入枚举, 而 `agent-resume-restart` / `agent-fresh-restart` / `multi-pane-resume-restart` / `cold-start-recovery` 的既有 requirement 文本仍然成立 (fresh restart 依旧不带 resume flag), 不必各写一遍 delta.

### D6: reconcile 周期刷新 + 持久化文件指纹

放在 `src/db/reconcile.rs::reconcile_all()` 里, 与 `db::codex_rollout` 同型: 都是"扫 agent 自己的磁盘 transcript 回填 slot".

之所以周期刷新而不是只在重启那一刻探测: 重启路径需要的是一个**已经就绪**的值.  cold-start recovery 时 pane 早已死亡, resume 重启时 pane 正要被杀, 都不是做 IO 和容错的好位置; 让 reconcile 提前把值备好, 重启路径只做一次读表.

必须带指纹 (transcript 路径 + mtime 秒 + 文件长度), 未变化就跳过, 否则每个 tick 对每个 claude pane 读 1 MiB.

**指纹必须持久化在 slot 上, 不能放进程内**: `reconcile_all()` 由两个进程驱动 —— `src/tui/status_poller.rs:111` 的 home-view poller, 和 `src/tmux/notification_monitor.rs:727` 的常驻 monitor (TUI 在 `src/tui/app.rs:134` 拉起它).  进程内缓存只让先探到某文件的那个进程跳过, 另一个进程每 tick 照读不误.  落在 `agent_slot.model_fingerprint` 上还顺带让 AoE 重启后不必重扫全部 transcript.

### D7: 模型不出现在任何 UI 上

`AGENTS.md` 要求"每个可配置字段都要能在 settings TUI 编辑".  `agent_slot.model` 不是配置字段, 是探测出来的运行时状态, 所以不适用那条 —— 不加 `FieldKey`, 不加 `build_*_fields()` 条目, 不加 `*ConfigOverride`.  用户显式指定模型的地方仍然是 `extra_args`.

也不在 home 列表行展示.  曾设计过一个 `theme.dimmed` 的 badge (剥掉 `claude-` 前缀, 放在 branch 之后、状态类 badge 之前), 实现并通过测试后撤除: 左右分屏时列表行本来就窄, 再塞一个模型名会把行撑爆, 而这个信息在 claude 自己的界面里随时可见, 不值得占这个宽度.

于是本 change 完全不碰 TUI 渲染代码, `tui` capability 也不需要 delta.

## Risks / Trade-offs

- **[脱离 default 轨道]** 一旦探测到模型, 该 pane 每次重启都带显式 `--model`, 以后 Anthropic 换默认模型时这个 pane 跟不上 → 用户可以删掉会话重建, 或在会话里 `/model` 切回想要的模型 (下一轮 reconcile 会跟上).  已知且接受.
- **[`[1m]` 丢失]** 从 `[1m]` 会话重启后 auto-compact 会按较小窗口假设工作 → 对 claude 认得的模型 (fable / opus-5) 它有内建窗口值, 影响有限; 必要时用 `CLAUDE_CODE_MAX_CONTEXT_TOKENS` 兜.  已决定接受.
- **[大文件 IO]** 每个 claude pane 每轮最多读 1 MiB → 指纹缓存跳过未变化文件; 且 reconcile 本来就是周期性的低频路径.
- **[transcript 定位错]** slot 的 `cwd` 与 claude 实际的 project 目录不一致时 (例如 worktree / 容器内路径), 会找不到文件 → 探测返回空, 按"探测失败保留旧值、不注入"处理, 不会拼出错误的模型.
- **[非 primary pane 首次拿到 flag]** 这是行为变化: 之前非 primary claude pane 的命令只有 `binary + resume_flag` → 只增加 `--model`, 不引入 `extra_args`, 变化面可控.
- **[e2e 触碰 tmux]** 本机有几十个实时 session → 测试必须走 `TuiTestHarness` 私有 socket, 同时清掉 `$TMUX` / `$TMUX_PANE`; 绝不 `tmux kill-server`, 不按前缀批量杀 session; 编译校验只用 `cargo build` / `cargo check`, 不跑全量 `cargo test`.

## Migration Plan

- Schema: `agent_slot` 加 `model TEXT NOT NULL DEFAULT ''`, 通过 `backfill_agent_slot_columns()` 幂等自愈, 旧库不需要手工迁移, 也不需要新的 `src/migrations/` 版本.
- 首次运行: 所有 slot 的 `model` 为空, 行为与今天完全一致 (不注入 flag).  第一轮 reconcile 之后逐步填上.
- 回滚: 移除注入点即可; 多出来的列对旧版本无害 (旧版 `upsert_agent_slot` 不写它, 但它有 DEFAULT).

## Open Questions

- fork 不继承父会话的模型: fork 拿到新 instance id, 还没有 slot, 首次启动因此不带 `--model`.  按"模型是座位属性"讲得通, 但当前 spec 文本并不蕴含它; 若要继承, 需要单独一条 requirement.
