## 1. Agent 与 capability wiring

- [x] 1.1 将 OpenCode 设为 host-launch agent, 配置 `--session {}` resume, 并让 primary/secondary command builder 走统一注册表路径
- [x] 1.2 将 OpenCode 加入 per-pane Cross Agent Team capability, 保持 checkbox、默认值和 sibling 状态独立
- [x] 1.3 为 OpenCode extra args 增加 attach-safe allowlist, 在 generation 推进前拒绝默认 TUI 专属、未知或 runtime-owned 参数

## 2. Durable slot generation 与 migration

- [x] 2.1 为 `agent_slot`、读取模型和全部 upsert 路径增加 `xats_runtime_generation`
- [x] 2.2 实现 per-slot runtime preparation transaction, resume 保留 session, fresh 清空 session, 两者都只推进目标 slot generation
- [x] 2.3 新增 v010 migration 与幂等 schema healing, 覆盖旧 profile 和 legacy store
- [x] 2.4 更新 reconcile、launch-time record 和 capture 写入, 确保 generation 不被空 capture 或 config rewrite 清除

## 3. xats launcher control-plane client

- [x] 3.1 新增 xats control-plane client 与结构化结果验证, identity key 不进入 URL、response、日志或诊断
- [x] 3.2 实现启动前 REST reserve, 区分 reserved、already_reserved、need_register 与 fail-closed domain error
- [x] 3.3 实现 session-ready 后 REST commit, 仅对 HTTP 503、transport I/O、partial commit 与 exact probe failure 做 bounded retry, 其他 outcome 立即 fail closed
- [x] 3.4 为 REST request、日志脱敏、状态解析和错误分层增加聚焦测试
- [x] 3.5 以 xats loopback REST reserve/commit adapter 替换 PATH CLI transport, 实现 pid file discovery、pid 存活校验、bearer auth、严格 status/schema 和 key 脱敏
- [x] 3.6 将同步 reserve 放入独立 Tokio runtime, 将 runtime commit 改为 async 调用, 并删除仅供旧 CLI transport 使用的 process-group 代码
- [x] 3.7 对 daemon pid file 和无 Content-Length 的 response 实施真正的 bounded read, 并脱敏 identity key、bearer token 与 control-plane Debug 输出

## 4. OpenCode server/attach runtime

- [x] 4.1 新增隐藏 `aoe __opencode-runtime` CLI 与独立模块, 校验 profile、instance、slot、generation、working directory 和 optional resume id
- [x] 4.2 在 loopback 独立端口启动 `opencode serve`, 等待 health, 并显式处理 bind、timeout 和 child exit
- [x] 4.3 Fresh 通过 `POST /session` 创建准确 session, Resume 通过 exact `GET /session/<id>` 验证 session, 全部响应使用 schema 验证
- [x] 4.4 在 attach 前写 `pane_live` 和 matching durable slot session id, 使用 pane ancestry 检查与有界 slot materialization wait
- [x] 4.5 运行 `opencode attach <base_url> --session <id>`, 仅透传 attach-safe 参数, 并在 attach 退出或错误时只清理 owned server child
- [x] 4.6 从 matching durable slot 加载 identity key, 保证 key 不进入 tmux pane command argv, 只注入 OpenCode server/attach child 环境

## 5. Launch、restart 与 recovery 集成

- [x] 5.1 普通 host OpenCode primary/secondary launch 使用 runtime wrapper并立即建立可 capture 的 slot
- [x] 5.2 Cross Agent Team OpenCode 在 tmux create/respawn 前持久化 generation 并同步 reserve, runtime 通过 instance、slot 与 generation 回读 matching key
- [x] 5.3 `Shift+R` 对每个 OpenCode slot 使用准确 durable session, 缺失/无效 session 返回 per-pane error而不 fresh fallback
- [x] 5.4 `Shift+C` 对每个 OpenCode slot 清空旧 session 并创建新 session, 同时保留 identity key 并推进 generation
- [x] 5.5 cold recovery、single-pane fallback 和 added-pane flow 复用同一 runtime preparation, 不影响 Claude/Codex/shell sibling

## 6. 验证与回归

- [x] 6.1 添加 store/migration 单元测试, 证明 generation per-slot、fresh/resume transaction 和 legacy healing
- [x] 6.2 添加 command/runtime 单元测试, 证明双 OpenCode pane 同 cwd 使用不同 key、generation、endpoint 和准确 session
- [x] 6.3 添加 C/R/recovery 聚焦测试, 证明 C 更换 session、R 保留 session、旧 generation 不进入新命令
- [x] 6.4 运行 `cargo fmt --check`、`cargo check`、`cargo clippy` 及不触碰实时 tmux 的聚焦测试, 记录因实时 session 跳过的 E2E 边界
- [x] 6.5 添加 fake loopback OpenCode HTTP 与 owned server child 测试, 验证 session API 和 cleanup 错误传播
- [x] 6.6 添加并发 extra-pane reservation 与 exact bind token 测试, 证明租约内 pending slot 不会复用
- [x] 6.7 添加 stale bound、expired pending 与 post-snapshot bind 测试, 证明关闭或崩溃遗留的 extra pane slot 可安全复用, 且旧 live snapshot 不覆盖刚绑定的 row
- [x] 6.8 添加 fake loopback xats daemon 测试, 覆盖 discovery、auth、严格 request/response、retry/fatal 分类、partial commit、hard size limit、timeout 和 secret 脱敏
- [x] 6.9 添加 durable runtime identity 测试与 pane command 断言, 证明 stale generation 被拒绝且 identity key 不进入 argv

> 验证边界: 已完成格式、全目标编译与 47 个 OpenCode 聚焦测试。  generation、post-snapshot bind、C/R action、严格静态检查与 OpenSpec gate 会在最终验证轮重新运行。  当前环境存在实时 tmux session, 按安全规则不运行 tmux E2E 和全量测试。
