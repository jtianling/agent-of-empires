## 1. Agent 能力判据重构

- [x] 1.1 在 `AgentDef` 增加 exact-session runtime 形态字段, 区分 AoE 自有 server 与共享单例 server
- [x] 1.2 用注册表字段替换 `src/session/instance.rs` 中"使用 AoE 自有 runtime""identity key 不进 pane 环境""支持 Cross Agent Team""resume 前校验 session id"四类字符串比较
- [x] 1.3 以 OpenCode 既有聚焦测试作为回归网, 确认重构后行为零变化
- [x] 1.4 增加断言测试, 证明能力由注册表字段决定而非 tool 名

## 2. Agent 与 capability wiring

- [x] 2.1 新增 kimi `AgentDef` 条目, 配置 host launch、`--session {}` resume、YOLO 与状态检测函数
- [x] 2.2 实现 kimi 状态检测函数并覆盖 idle、running、waiting 与选择提示形态
- [x] 2.3 将 kimi 加入 per-pane Cross Agent Team capability, 保持 checkbox、默认值与 sibling 状态独立
- [x] 2.4 为 kimi extra args 增加 launch-safe allowlist, 拒绝 runtime 自有参数与会改变会话选择的参数

## 3. 共享 server 发现

- [x] 3.1 实现 instance 注册表读取: 目录枚举、`.json` 后缀过滤、单条 4 KiB 上限、宽容解码
- [x] 3.2 实现实例选择: 按 `started_at` 升序取首个存活实例, 存活判据只看 pid 且遵循 ESRCH 死 / EPERM 活 / 其他保守判活
- [x] 3.3 解析失败条目不删除, 无存活实例时 fail closed 并给出可操作诊断
- [x] 3.4 读取共享 bearer token, 缺失时 fail closed
- [x] 3.5 实现启动前能力探测 server 侧: 校验所选实例的 IPC socket 存在且与该实例 pid 存活绑定判断, 不用版本号
- [x] 3.6 实现启动前能力探测 CLI 侧: 要求 kimi pane 显式配置二进制路径, 不取 PATH 默认, 任一侧不满足则拒绝并说明缺什么
- [x] 3.7 为发现路径增加聚焦测试, 覆盖多实例排序、死 pid、超限、损坏条目、空目录、遗留 socket 与两侧能力探测失败

## 4. 准确 session 生命周期

- [x] 4.1 实现 fresh 铸造: 创建 session 并使 `metadata.cwd` 等于 pane working directory, 响应使用 schema 验证
- [x] 4.2 实现 profile 设置 (model 与 permission mode), 失败即中止启动
- [x] 4.3 实现 main agent 物化, 使用同步读路径, 不发送任何消息, 不轮询文件系统
- [x] 4.4 实现 resume: 使用 durable slot 的准确 session id, 缺失或无效时返回 per-pane error 而不 fresh fallback
- [x] 4.5 在启动 pane 进程前把准确 session id 写入 durable slot
- [x] 4.6 为 session 生命周期增加 fake loopback kimi server 测试, 覆盖铸造、profile、物化、cwd 不匹配与错误传播

## 5. xats commit 客户端

- [x] 5.1 复用既有 daemon 发现、pid 校验、bearer 与脱敏基础设施, 新增 kimi commit 客户端
- [x] 5.2 实现严格请求与响应 schema, 区分 committed、need_register 与各 fail-closed outcome
- [x] 5.3 只对 session 探活失败做有界重试并使用完全相同的 tuple, 其余 outcome 立即 fail closed
- [x] 5.4 把 `session_claimed_by_other_agent` 作为致命错误在启动 TUI 前中止并上报
- [x] 5.7 session id 变更时先用旧坐标 commit 让 daemon 采纳 identity key, 再用新坐标 commit 刷新; session 未变时只调一次
- [x] 5.5 不把 `probed:false` 的成功当作会话健康证据
- [x] 5.6 为 commit 客户端增加 fake loopback daemon 测试, 覆盖每个 outcome、重试分类、超时、大小上限与脱敏

## 6. MCP 配置校验

- [x] 6.1 读取用户级 kimi MCP 配置并校验 xats server 条目存在、`scope` 为 session、带 session id header 模板
- [x] 6.2 校验失败时 fail closed 并打印用户需要粘贴的配置, 不写入任何配置文件
- [x] 6.3 为校验增加聚焦测试, 覆盖缺失文件、缺条目、缺 header、错误 scope 与合法配置

## 7. Launch、restart 与 recovery 集成

- [x] 7.1 普通 host kimi primary/secondary launch 使用统一准备路径并立即建立可 capture 的 slot
- [x] 7.2 环境注入实现先删后设, 覆盖 base URL、session id 与远程引擎模式
- [x] 7.3 保证 identity key 既不进入 pane command argv 也不进入 pane 环境, 只从 durable slot 读出用于 xats 请求体
- [x] 7.4 `Shift+R` 对每个 kimi slot 使用准确 durable session
- [x] 7.5 `Shift+C` 为每个 kimi slot 铸造新 session 并保留 identity key
- [x] 7.6 `Shift+C` 严格串行: 终止旧 pane 并确认进程退出后, 再铸造、commit 与启动新 pane
- [x] 7.7 commit 之后不触发任何其他 xats 注册动作
- [x] 7.8 sandboxed kimi 在 session 铸造前 fail closed, 不回退 shell pane
- [x] 7.9 cold recovery、single-pane fallback 与 added-pane flow 复用同一准备路径, 不影响 Claude/Codex/OpenCode/shell sibling

## 8. 验证

- [x] 8.1 添加双 kimi pane 测试, 证明同 cwd 两个 pane 使用不同 session 与 identity key
- [x] 8.2 添加 C/R 聚焦测试, 证明 C 更换 session、R 保留 session、identity key 两者都保留
- [x] 8.3 添加 pane command 与环境断言, 证明 identity key 不出现在 argv 也不出现在 pane 环境
- [x] 8.4 运行 `cargo fmt --check`、`cargo clippy` 与不触碰实时 tmux 的聚焦测试, 记录因实时 session 跳过的 E2E 边界
- [ ] 8.5 端到端验证时写死被测配置 (二进制来源与远程引擎模式是否生效), 不用具备能力的构建的绿去否定发布版配置的风险
- [ ] 8.6 待 xats `POST /api/runtime/kimi/commit` 上线后补端到端验证, 并用绑定确认判据证明 handshake 生效
