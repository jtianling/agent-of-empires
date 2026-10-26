# Agent 管理 CLI

`aoe managed` 让本地管理 agent 创建和管理工作 session.  所有变更只允许操作当前管理身份创建的 session, 人工 session 和其他管理者的 session 只提供基本信息与已保存状态.  终端内容读取也受所有权检查保护.

这是接口防误操作机制.  拥有同一系统用户完整 shell 权限的 agent 仍可绕过此入口调用人工 CLI 或 tmux.  首次启用前需要更新并重启原有 TUI, 旧版本的 TUI 不具备并发保存保护.

## 建立管理上下文

每个管理 agent 建立独立上下文, 多次对话和重启使用同一个文件.  上下文包含随机凭据, 不要提交到仓库、输出给其他 agent 或共享给子 agent.

```sh
aoe managed --context /absolute/path/manager.json init
export AOE_MANAGER_CONTEXT=/absolute/path/manager.json
```

上下文父目录必须存在.  `init` 不覆盖已有文件.  返回的 owner 是公开标识, 不能仅凭这个标识获得管理权限.  子 agent 的 xats name/team 与管理 owner 独立.

## 创建与管理

```sh
aoe managed create /absolute/path/project --tool codex --title coder

aoe managed create /absolute/path/project \
  --tool codex --title collaborator \
  --xats-name coder --xats-team project --launch

aoe managed list
aoe managed show <完整-session-id>
aoe managed start <完整-session-id>
aoe managed stop <完整-session-id>
aoe managed restart <完整-session-id>
aoe managed rename <完整-session-id> --title new-title
aoe managed send <完整-session-id> '请检查任务状态'
aoe managed capture <完整-session-id> --lines 100
aoe managed remove <完整-session-id>
```

未指定 `--launch` 时创建 session 记录但不启动 agent 进程; 带 xats 参数时还会预留协作身份.  项目目录必须已存在, 此入口不会创建或删除 worktree、分支.  `remove` 保留代码目录与共享资源.  所有目标操作必须使用返回的完整 16 字符 session ID, 不接受标题和短前缀.

自动 xats 身份启动首批支持 `codex`、`claude` 和 `opencode`.  `kimi` 的共享引擎目前不能注入会话独立的 managed 凭据, 因此 `--tool kimi` 与 xats 参数组合会在创建前拒绝.  不带 xats 参数的 managed Kimi session 可正常创建.

自动注册需要配套支持 `/api/managed/{prepare,bind,status,release}` v1 的 xats daemon.  旧 daemon 不会被自动更新, 接口不可用时会明确失败.  配套实现已完成独立容器中的 Codex、Claude、OpenCode 实际启动和重启验收, 尚未替换生产 daemon.

使用 `aoe -p <profile> managed ...` 指定 profile.  管理上下文可以跨 profile 使用, 每个 session 的 owner 仍独立校验.

## 人工接管与转移

人工 TUI 和原有 CLI 保持完整管理权限.  人工可以显式接管或转移一个 session:

```sh
aoe session set-owner <完整-session-id>
aoe session set-owner <完整-session-id> --context /absolute/path/other-manager.json
```

省略 `--context` 表示改为人工所有.  旧管理者随后不能再修改或读取该 session 的终端内容.  接管不改变现有工作内容或 xats 身份.

## JSON 和恢复

成功输出形如 `{"ok":true,"data":...}`, 失败输出形如 `{"ok":false,"error":{"code":"permission_denied","message":"...","session_id":"..."}}`, 失败退出码非零.  诊断写入 stderr.  空列表返回 JSON 数组, 不返回说明文字.

创建可能在记录保存后启动或注册失败.  此时 `create_incomplete` 错误携带 `session_id`, 应使用这个 ID 查看和重试, 不要盲目重复创建.  session 进程状态与 xats 注册就绪是两个独立结果, 不应把创建成功当作已经加入协作.  xats 身份冲突会失败, 不自动改名或覆盖已有协作者.

列表中的 `status_source: "stored"` 表示状态来自 AoE 保存的数据, 不是实时进程探测.  xats 就绪由精确身份状态查询确认.  并发修改同一记录可能返回冲突, 应重新读取后再操作, 不要直接覆盖存储文件.

自动 xats Codex 暂不接受配置中的自定义启动命令或额外启动参数, 避免参数覆盖受控 thread 或 profile.  模型使用当前 Codex 默认配置.

本次执行了编译、严格 clippy、隔离 CLI 与 xats 协议回归.  `tests/managed_cli_smoke.py` 和 `tests/managed_xats_cli_smoke.py` 使用临时 HOME 与 tmux 替身.  `tests/managed_xats_integration.mjs` 接受 AoE、Codex、xats CLI 的绝对路径, 验证独立 daemon 与 app-server 的协议绑定; 它不替代实际 launcher 验收.

独立 tester 另在断网、无宿主挂载的一次性 Linux 容器中验证了实际 shell 生命周期、TUI 并发保存、人工重启删除、三种 agent 的自动绑定与重启身份保持、旧 fence 拒绝、Claude 首次失败恢复及资源回收.  最终用例没有模型请求.  版本为 tmux 3.4、Codex 0.164.0、Claude Code 2.1.299、OpenCode 1.19.4.  未运行宿主真实 tmux 测试或全量测试套件.
