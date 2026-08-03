## Context

OpenCode 当前只有 primary pane 依赖默认 command override 才能在 host 启动, secondary pane 会被 `supports_host_launch = false` 拒绝。  它没有 `ResumeConfig` 或可靠 capture, fork 只能按 cwd 选择最新 session。  该推断在同 cwd 双 pane 下无法区分 conversation。

xats 与 AoE 已对齐两阶段 recovery 协议。  AoE 必须在 OpenCode 启动前 reserve durable runtime generation, 在准确 session ready 后 commit 完整 delivery, 再由 xats 向该 session 注入 recovery prompt。  xats control-plane commit 不等于 OpenCode MCP connection 已绑定, 最终绑定由该 prompt 触发的 agent turn 调用 `reconnect` 完成。

## Goals / Non-Goals

**Goals:**

- 让 host OpenCode primary/secondary pane 都使用准确、独立的 native session。
- `Shift+C` 创建全新 conversation, `Shift+R` 恢复原 conversation。
- 在 C/R/live/cold launch 中保持每个 slot 的 xats identity, 并用 generation fence 阻止旧 runtime 回写。
- 不修改用户的 OpenCode 配置, 不安装全局 plugin, 不把 identity key 放入 argv 或日志。
- 对 xats CLI、OpenCode server、session 创建/读取和 durable store 的每层错误显式失败。

**Non-Goals:**

- 不修改 xats daemon、CLI 或 MCP 工具实现。
- 不为 sandboxed OpenCode 增加 xats 支持。
- 不保留按 cwd/latest session 查找作为 restart fallback。
- 不改变 Claude、Codex 或其他 agent 的启动协议。
- 不为 managed host OpenCode 实现 fork。  在 exact-session runtime fork 可用前, AoE 显式拒绝该操作; sandboxed OpenCode 保持原生 fork。

## Decisions

### 1. 使用 AoE runtime wrapper, 不安装 OpenCode plugin

Host OpenCode 由隐藏入口 `aoe __opencode-runtime` 管理。  wrapper 启动独立的 `opencode serve --hostname 127.0.0.1 --port <allocated>`, 等待 health, 然后 fresh 创建 session 或 resume 校验指定 session, 最后运行 `opencode attach <base_url> --session <id>`。

wrapper 在 attach 退出时终止自己创建的 server child, restart 的现有 process-tree teardown 也能清理整个 runtime。  该方案直接掌握 endpoint 与 session id, 不依赖 plugin event 顺序, 也不写 `~/.config/opencode`。  备选方案是安装 global plugin, 但它会修改用户配置目录, 仍不能保证空 TUI 已经创建 session, 因此不采用。

### 2. Fresh 与 resume 都使用准确 session API

Fresh runtime 通过 loopback server 的 `POST /session` 创建 conversation, 并使用返回对象中的 `id` attach。  Resume runtime 使用 `GET /session/<encoded-id>` 确认 durable id 存在, 再 attach 同一 id。  请求携带目标 working directory, session id 必须通过 OpenCode token schema, URL 和 HTTP response 必须通过边界验证。

这样 `Shift+C` 不需要虚假 bootstrap prompt 来物化 session, xats recovery prompt 可以成为新 conversation 的首个真实 turn。  `Shift+R` 不需要 session list, 同 cwd sibling 不会参与选择。

### 3. per-slot generation 在 launch 前持久化并 reserve

`agent_slot` 新增正整数 `xats_runtime_generation`, legacy row 从 0 开始。  每次启用 Cross Agent Team 的 OpenCode slot 启动前, AoE 在 store 中原子递增 generation。  Fresh 同一事务清空旧 `native_session_id`, resume 保留它。

新增 OpenCode pane 会先严格读取当前 tmux live pane 集合, 再在 SQLite immediate transaction 中原子选择 extra slot。  缺失 row 优先并以 generation 1 插入; slots 已满时, 只 CAS 替换已绑定但不在 live 集合中的 stale row, generation 在旧值上递增。  `tmux_pane=''` 的 pending row 即占位, 并发 add 不会复用。  split 后的 bind 和失败 rollback 都使用原始 `(slot, generation, identity_key)` token, 且只匹配仍未绑定、session 为空的 row。

AoE 随后同步执行 `cross-agent-teams-mcp reserve-opencode-runtime --identity-key-env XATS_IDENTITY_KEY --runtime-generation N`, key 只从子进程环境读取。  CLI 默认输出单行 JSON envelope, 不接受额外 `--json`。  reserve 必须在 tmux respawn/create 前成功。  `need_register` 是首次 identity 的允许状态, 其他非成功状态阻止该 pane 启动。

备选方案是只在 session ready 后提交 generation, 但较新 runtime 尚未 ready 时, 较旧 runtime 仍可覆盖 delivery, 因此不采用。

### 4. session ready 后由 wrapper commit delivery

wrapper 得到准确 `(base_url, session_id)` 后执行 `cross-agent-teams-mcp commit-opencode-runtime --identity-key-env XATS_IDENTITY_KEY --runtime-generation N --base-url <url> --session-id <id>`, 同样只通过环境传 identity key。  xats 负责 exact probe、CAS、recovery prompt 和后续 MCP reconnect。  unknown key 允许 attach, 等用户首次正常注册; 其他失败由 wrapper 输出诊断并停止 runtime。

AoE 只消费 xats 已对齐的状态机, 不把 `Clear`/`Resume` mode 传入 xats。  paired CLI 通过 PATH 中的 `cross-agent-teams-mcp` 发现, CLI/daemon protocol mismatch 必须由 CLI 非零失败, AoE 不使用 `npx @latest` 或旧参数 fallback。

### 5. runtime 直接写准确 capture

wrapper 在 attach 前以继承的 `TMUX_PANE`、profile、instance id 和 slot 写 `pane_live`, 并更新已存在 durable slot 的 `native_session_id`。  写入沿用 `__record-pane` 的 pane ancestry 验证和 store schema validation。  primary launch 的 slot 在 tmux create 返回后立即建立, wrapper 对 slot materialization 使用有界等待, 不按 cwd 猜测。

非 Cross Agent Team 的 host OpenCode 也使用同一 wrapper, 因而同样获得准确 C/R。  xats reserve/commit 只在 pane 开启 Cross Agent Team 且 key/generation 有效时执行。

### 6. OpenCode 专属参数由 wrapper 拆分

AoE 拥有 server 的 `--hostname`、`--port` 以及 attach 的 `--session`。  Cross Agent Team 或 resume runtime 遇到用户 extra args 中冲突的 `--hostname`、`--port`、`--session`、`--continue` 或 `--fork` 时显式拒绝。  其余 TUI 参数传给 attach, YOLO 环境变量和 working directory 保持现有语义。

AoE 创建的 server 是仅供当前 pane 与 xats exact probe 使用的临时 loopback endpoint, 不是用户配置的共享 OpenCode server。  wrapper 显式移除 `OPENCODE_SERVER_USERNAME` 与 `OPENCODE_SERVER_PASSWORD`, 因为配对协议不传递 OpenCode auth secret, 且继承 auth 会让 health 与 xats exact probe 不可达。  这是 managed host OpenCode 的兼容性变化: 本机其他进程在 runtime 存活期间可访问该随机端口, 但 endpoint 只绑定 loopback, 由 wrapper 在 attach 退出或启动失败时清理。

## Risks / Trade-offs

- [xats 配对 CLI 尚未发布] → OpenCode Cross Agent Team 启动明确失败并保留诊断, 普通 OpenCode 启动不受影响。
- [loopback 端口分配存在 bind 竞态] → wrapper 对 bind 失败使用新的受限端口重试, 每个 pane 独立分配。
- [server 已启动但 commit 失败] → wrapper 终止自己创建的 server, 不留下看似可用但未 fenced 的 pane。
- [reserve 成功但 OpenCode 启动失败] → xats row 保持 recovering, 下一次更高 generation 重试; 旧 endpoint 不会复活。
- [旧数据库没有 generation] → v010 migration 和每次 store open 的幂等 schema healing 添加默认 0 列, 首次 launch 推进到 1。
- [headless server + attach 比单进程多一个 child] → wrapper 明确拥有 child 生命周期, 测试验证 attach 退出和错误路径清理。

## Migration Plan

1. v010 为所有 profile 的 `agent_slot` 添加 `xats_runtime_generation INTEGER NOT NULL DEFAULT 0`。
2. 新二进制首次启动时完成 schema healing; 旧 row 保留 identity/session/config。
3. 首次 OpenCode launch 将 generation 从 0 推进到 1, xats legacy row 由 reserve/首次 register 进入新协议。
4. 回滚旧二进制时新增列会被忽略, 原 session 与 identity 字段仍可读取; 新 runtime fencing 功能停止。

## Open Questions

无。  xats 的 reserve/commit、partial state、prompt 与 reconnect 语义已与 `xats-main` 对齐, AoE 只实现消费端。
