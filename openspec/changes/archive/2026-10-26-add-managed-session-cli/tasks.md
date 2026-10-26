## 1. 所有权与存储

- [x] 1.1 实现稳定管理上下文和输入验证, 新 session 持久 owner, 人工接管及转移入口.
- [x] 1.2 实现加锁、原子写入和陈旧快照冲突保护, 避免并发创建丢失及删除复活.
- [x] 1.3 编写隔离存储与权限测试, 覆盖缺失身份、其他 owner、人工 session 和并发保存.

## 2. 受限 CLI

- [x] 2.1 实现 managed 命令分派、JSON 输出和明确错误码, 完整 ID 精确定位.
- [x] 2.2 复用创建和生命周期能力, 实现自己 session 的启动、停止、重启、重命名、发送与读取输出.
- [x] 2.3 实现安全删除, 保留代码与共享资源, 所有副作用前鉴权并防止转移竞争.
- [x] 2.4 实现基本信息列表、详情和状态, 对非 owner 隐藏终端内容及秘密.

## 3. xats 集成

- [x] 3.1 与 xats-main 确认预设身份、冲突保护、就绪及解绑契约, 更新设计.
- [x] 3.2 实现创建时 name/team、精确实例绑定及重启恢复, fork 不复制身份凭据.
- [x] 3.3 实现注册状态查询、部分失败恢复及安全解绑, 保留具体错误原因.

## 4. 验证与文档

- [x] 4.1 添加权限拒绝、并发、xats 冲突和协议失败的测试覆盖, 编译测试目标.
- [x] 4.2 运行 fmt、check/build 和 clippy, 不运行全量测试或宿主 tmux 探针.
- [x] 4.3 在隔离临时数据目录验证无 tmux 的 CLI 创建、列表、越权拒绝和人工接管, 不操作真实 session.
- [x] 4.4 编写中文使用文档, 说明上下文配置、xats、JSON、保护边界及旧 TUI 重启要求, 重新生成 CLI reference.

验证记录: fmt、`cargo check --tests`、`cargo clippy --all-targets -- -D warnings`、`cargo build`、`cargo xtask gen-docs`、两组隔离 CLI 回归和真实 daemon 协议联调通过.  独立 tester 在断网、无宿主挂载的 Linux 容器补齐实际 shell/TUI 生命周期、并发保存、Codex/Claude/OpenCode 自动绑定和重启、旧 fence 拒绝及首次 Claude 失败恢复.  测试发现的 selector、URL 规范化、备用 shell 等待、Clap 属性、reconnect 输入及 loaded-list 误判均已修复并复验通过.  增量独立核验 PASS.  未运行全量 Rust 测试或操作生产 session.
