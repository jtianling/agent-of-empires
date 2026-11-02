# Codex 原生会话名与 xats 身份

2026-10-29, 用户要求 AoE 启动 Codex 时, 尤其 resume 和 restart 后, 将原生会话名改成 `name@team`.

本机 Codex 0.165.0 源码确认 `/rename` 调用 `thread/name/set`.  AoE 复用此接口, 不向模型发送改名指令, 不向正在使用的输入框注入文字.  managed Codex 原来已在准备线程时调用此接口, 但用的是 AoE 标题; 本轮改用各 pane 明确配置的协作身份.

普通 TUI 的 xats Codex 在 preregister 成功后启动一次命名助手, 继承本 pane 的 launcher key 和实际 tmux 连接.  助手等待当前启动的 thread 绑定及已注册身份, 命名前复核启动代次、thread、name 和 team, 再改名并读取确认.  未注册或无法确认身份时不猜名称, 成功后不持续覆盖用户手动改名.

等待最多 120 秒, tmux 和 ps 查询各有 5 秒超时.  子进程使用异步输出和 `kill_on_drop`, 避免同步 `Command::output` 卡住外层 timeout.  原启动进程退出后助手停止等待, 普通助手失败原因追加到 AoE 数据目录的 `debug.log`.  managed 准备阶段的改名失败沿用原有显式错误和资源回收行为.

项目经验: Tokio timeout 不能中断同步子进程等待.  声明有界等待的异步流程应让子进程等待可取消并回收准确的自有 child.  此处只取消 tmux 客户端或 ps, 不停止 tmux server.

候选源码及验证记录位于 `target/review-builds/20261029-codex-name/`.  独立 reviewer Carver 复核后 PASS, 原超时 WARNING 已关闭.  实际 Codex 与 tmux 验收在断网、无宿主挂载的一次性 Linux 容器中进行, 不操作业务会话.

最终独立验收 47 项全部 PASS, 包括 managed 主/附加 pane 的启动、恢复、Fresh 和接任, 普通 TUI 的真实首次注册及 R/C 恢复, 手动名称保留、旧代与邻 pane 拒绝、profile 连接冲突和助手退出.  tmux/ps 故障分别约 5 秒超时, 自建 child 已回收, 真实 Codex 不受影响.  宿主编译、Clippy、格式检查及两种 Mac 构建的 tmux 替身 CLI 回归检查通过.

普通 R/C 保留原有 xats 恢复通知和注册回合.  与旧基线对照, 模型请求数均为初始 3、R 后 5、C 后 8, 改名没有新增请求或 `/rename` prompt.  managed 路径的命名步骤没有模型请求.

Linux 被测二进制 SHA256 为 `f8d41642cb6388d7ebde538c9425ad7f0b4d1a276da0c2e20dd6a04ef1db83fa`, 205 项生产 Rust/Cargo 输入与最终 Mac 源码逐项匹配, 无漂移.  报告位于 `.e2e-test/runs/codex-native-xats-name-20261029-090646-rjmffmag/report.html`, 本机地址 `http://127.0.0.1:61991/report.html`, 至 2026-10-31 17:35 新加坡时间自动退出.  报告交互和桌面/窄屏均已核验, 自开 tab 已关闭, 测试容器已精确删除.

已沿用此前全局安装授权, 安装同源 `dev-release` 优化构建至 `/Users/jtianling/.cargo/bin/aoe`, SHA256 为 `a3ab5129f95a20a3e03bf026bcbe5fcf80d1f46e32e6d9e857cbe07d8869959b`.  原全局版本已备份, 安装信息记录在候选目录的 `global-install-info.json`.  运行进程及业务会话没有重启.
