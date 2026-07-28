# codex 的 pane 从来没有被追踪过

## 现象

AoE 追踪过的每一个 pane 都是 Claude 的 pane.  不是选择, 是构造决定的.

实机数据 (2026-07-28, 一台每天都在跑 codex 的机器): `agent_slot` 十七行、`pane_live` 八行, **没有一条是 codex**.

## 根因

pane 追踪完全由 agent 自己驱动: agent 的状态 hook 调 `aoe __record-pane`, 由它写入 `pane_live` 行, 再由 reconciler 快照成durable 的 `agent_slot`.  一个不装 hook 的 agent 不产生行, 就永远进不了 slot, 对恢复而言等于不存在.

`src/agents.rs` 里只有 `claude` / `gemini` / `cursor` 有 `hook_config`, **`codex` 是 `None`**.  而 `aoe __record-pane` 的 `--agent` 参数省略时默认写死 `"claude"`.

## 后果不是"codex 恢复得不好", 而是"恢复成了别的东西"

一个曾经跑 Claude、后来被换成 Codex 的 pane, 会一直保留那条陈旧的 `claude` 记录 —— 因为 Codex 做的任何事都无法纠正它.  恢复于是忠实地把 Claude 重新拉起来, 放进一个用户在跑 Codex 的 pane 里.

2026-07-28 实机复现: `helpers` (sub2api) 实例两个 slot 都记着 `claude`, 采纳时间是 07-27 16:43 和 16:55; 用户后来在其中一个 pane 里换成了 codex, 无人记录; 恢复后变成两个 claude.

## 需求

已有 openspec change 承载: `openspec/changes/track-codex-agent-panes` (18 个 task, `openspec validate --strict` 通过).

要点:

- codex 0.145 有一套和 Claude 结构接近同构的 hooks 系统, 配在 `~/.codex/config.toml`, 事件含 `SessionStart` / `UserPromptSubmit` / `Stop` 等
- native session id 不必去猜 codex 的 hook stdin 形状: codex 0.124+ 导出 `$CODEX_THREAD_ID`, 那正是 resume 和 xats reconnect 用的同一个值
- 因此 session id 的来源要改成**按 agent 具名声明**, 而不是假定一种形状.  声明的来源取不到值时不写行, 也不许回落到别的 agent 的来源

## 两个必须守住的约束

**写用户的 `~/.codex/config.toml` 必须是合并而非重写.**  和 `~/.claude/settings.json` 不同, 这个文件是用户自己在编辑的 —— 机器上那份里有指向别的工具的 `notify` 条目和二十多个 `[projects."..."]` 段.  安装只增删 aoe 自己认得出的条目, 其余逐字节保留; 卸载也只删自己的, 文件里还有别的内容时不删文件.

**codex 的 hook 有信任门, 不许绕过.**  新装的 hook 默认 untrusted, 要用户在 codex TUI 里信任一次才会运行.  `--dangerously-bypass-hook-trust` 存在, 但用它等于 aoe 悄悄安排自己的代码在用户的 agent 里运行, 而绕过 codex 特意要求的审查.  安装时应当如实告知这一步.

## 一个会被这件事激活的潜伏缺陷

codex pane 能进 slot 之后, "异构 adopted slot" 就从罕见变成常见.  Cross Agent Team 的装饰和身份键归属曾经按**实例的 tool** 判断, 这在异构 slot 上必然出错.  该问题已在 2026-07-28 独立修复 (改成按 pane 自己的 agent 判断), 但它是这件事的直接邻居, 后续改动要注意别退回去.
