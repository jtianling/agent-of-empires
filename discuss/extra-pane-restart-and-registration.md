# 让额外 agent pane 也能重启并自动注册 (设计记录, 未实施)

记录于 2026-08-01.  jt 的要求原话: **"右边的应该也需要重启并自动注册"**.  本文只记设计, **不实施**.

## 现状: 三样东西都不到位

### 1. `Shift+C` 按 `agent_slot` 扇出, 而右 pane 进 slot 是延迟的

重启的扇出面是 `agent_slot` 表里的行 (`src/tui/app.rs` 的扇出分支).  而 codex 没有 status hook, 它的 pane 要**先被 rollout 认领**才会进 slot, 认领又依赖 rollout 文件出现 —— 而 **rollout 是延迟落盘的: 文件要到该 pane 第一轮对话之后才写出来** (文件名保留会话开始时间).

实测的延迟: 真身 codex 44s / 77s; 另一次 slot 0 与 slot 1 相隔 2 分 37 秒.

**后果**: 在右 pane 被认领之前按 `Shift+C`, **只有左 pane 重启**, 右 pane 原地不动, 而界面上什么都不说.

### 2. 右 pane 的 tool 根本没有被持久化

它是新建对话框的一次性值 (`src/tui/app.rs` 的 `take_pending_right_pane_tool()`), 用完即弃.  `sessions.json` 里没有"这个会话有一个跑 X 的右 pane"这条信息.

**后果**: 即使想在没有 slot 的情况下重启它, **aoe 也不知道该启动什么**.

### 3. identity key 是按 slot 铸的, 没进 slot 就没有 key

`build_extra_pane_command` 传的是 `slot_identity_key = None`; key 由 `ensure_slot_identity_keys` 在**重启时**为已有的 slot 行铸出.

**后果**: 没进 slot → 没有 key → daemon 认不出这个 pane 属于哪个身份 → 不会给它发恢复通知 → **不会自动注册**.  这也是 spec 里记的"额外 pane 首轮无 key, 第二轮才有"的两轮语义的来源.

## 设计: 把"它是什么"和"它的 key"记在 pane 自己身上

用 tmux 的 **pane 用户选项**, 在启动额外 pane 的那一刻写入:

- `@aoe_pane_tool` = 该 pane 要跑的 agent 名;
- `@aoe_pane_key` = 为该 pane 铸的 identity key.

aoe 已经在用这套机制 (`@aoe_agent_pane`、`@aoe_waiting`、通知监视器的 pid/mtime 选项), 不是新造机制.

### 为什么是 pane 选项

当初 design (`extra-agent-pane-parity`) 否掉"启动时铸 key"的理由是: **没地方放 pane→key 映射** —— 新表要迁移, 挂在实例记录上要 GC.  **pane 用户选项的生命周期恰好等于 pane 的生命周期**, tmux 跟着 pane 一起回收, 不需要任何清理.  这个位置解决了当时的反对意见.

### 三件一起解决

1. **扇出面扩大**: `Shift+C` 除了覆盖 slot 里的 pane, 再覆盖"带 `@aoe_pane_tool` 但还没进 slot"的 pane, 用 `build_extra_pane_command(tool)` 重启 —— 不必等认领;
2. **首轮就有 key**: key 在启动时铸并注入, 所以第一次就能被 daemon 认出身份, 两轮语义消失;
3. **安全边界**: 有 mark 才动.  用户自己在分屏里手工起的东西没有这个选项, 永远不会被 aoe 重启掉.

### 一个必须做对的细节

reconciler 后来把这个 pane 收进 slot 时, **slot 必须接管 pane 选项上那把 key, 而不是另铸一把** —— 否则身份会在"被采纳"那一刻变掉, 而 xats 侧会把它看成一个新身份.

### 恢复路径

`recover_from_slots` 会重建会话并**创建新 pane** (新 pane id), 此时 pane 选项不存在.  那条路径上 key 由 slot 行携带, 是既有机制; 新建的 pane 需要重新写入两个选项.

## 未决 / 待验

- 扇出面扩大后, "活 pane 数 > slot 数"这个状态是否还需要给用户提示?  原本的轻量方案就是只补提示 (代码里已有先例: 读 slot 失败时会说 `restarted primary pane only`).  本设计如果实施, 提示可能不再必要, 但两者不冲突;
- shell 类型的右 pane 不应被当作 agent 重启 —— `@aoe_pane_tool` 写不写 `shell`, 以及扇出时如何跳过, 需要在实施时定;
- 本设计**没有**触及"两个 pane 的会话被互相绑错"那条 (见 `find_rollout` 的时间匹配), 那是独立问题.

## 相关

- `openspec/specs/cross-agent-team/spec.md` — 额外 pane 的 key 语义;
- `openspec/specs/pane-session-capture/spec.md` — slot 采纳与认领;
- 归档的 `extra-agent-pane-parity` design — 当初否掉"启动时铸 key"的原始理由.
