## Context

AoE 的 managed extra pane 共用 `build_extra_pane_command`.  agent tool 会通过 agent registry 构造命令, 但 `shell` 走独立分支.  该分支读取 `$SHELL` 作为最终目标, 却固定使用 `bash -lc` 作为外层 login shell.  因此用户配置为 zsh 时, 右 pane 仍会先执行 Bash 登录初始化.  当前环境的 Bash 初始化卡在 conda hook, 所以 `exec /bin/zsh` 永远不会执行.

AoE 的 restart 和 cold recovery 以 durable slots 为恢复源.  managed shell pane 如果继承 session 目录, 当前会被 launch-time writer 特判为 slotless.  这使 New Session 明确创建的 Codex + Shell 双 pane 在运行时有两个 pane, durable state 却只有 Codex.  `Shift+C` 和 cold recovery 因而只能恢复左 pane.

本变更同时修正启动边界和 durable lifecycle.  两者必须一起处理, 否则 shell 即使能启动, 仍然不属于 restart/recovery fan-out.

## Goals / Non-Goals

**Goals:**

- extra shell pane 使用 POSIX shell 作为 outer login wrapper, host 上最终 interactive shell 保留用户配置的原始 shell.
- AoE 通过 New Session、Fork、`%` 或 CLI managed-pane action 明确创建的 shell pane 始终写入 durable slot.
- `Shift+C` 重启所有 tracked panes 时同时重启 Codex 和 shell, 并保持 Codex 的 xats identity key.
- cold recovery 从 slots 重建 Codex + Shell 双 pane, 并恢复每个 pane 的目录.
- 使用私有 tmux socket 和隔离 HOME 验证真实 shell 启动、restart 和 recovery 行为.

**Non-Goals:**

- 不追踪用户通过原生 `prefix + %` 手工创建的 raw tmux pane.
- 不修改 xats identity 协议、nonce 语义或 Codex bootstrap 参数.
- 不推断或回填已经运行但从未写入 slot 的旧 shell pane.
- 不增加新的 slot 类型或突破现有四 slot 上限.

## Decisions

### 1. 区分 POSIX outer wrapper 与用户 interactive shell

extra shell command SHALL 使用 `user_posix_shell()` 选择外层 shell, 并由该 shell 执行 login command.  host 上最终 interactive shell SHALL 使用 `user_shell()` 保留用户的原始选择, 因此 fish、nu 或 pwsh 用户只在 outer wrapper 降级为 POSIX shell, 最终仍进入其配置的 shell.  两层 shell executable 都必须经过 shell quoting, 并保留 `stty susp undef` 和目标目录处理.

实现应提取或复用小型 command builder, 使 primary shell 和 extra shell 不再分别硬编码不同的外层 shell.  不在本变更中重构 agent command path.

备选方案是继续使用 `bash -lc`, 再增加 `--noprofile --norc`.  该方案仍然强制 POSIX shell 用户依赖 Bash, 与用户选择不一致, 并且会制造另一套初始化语义, 因此不采用.  对非 POSIX shell, outer wrapper 仍按既有 helper 退回 Bash, 但只用于解释 POSIX command string.

### 2. AoE 明确创建的 shell pane 一律占用 durable slot

删除 `shell` 且 cwd 等于 instance cwd 时跳过 slot 的例外.  `record_launched_extra_pane` 对 shell 和 agent 使用相同的 launch-time durable slot 规则, 差异只保留在 identity key 和 resume token 上.

这项决策把 "managed" 定义为 AoE 通过带 tool/cwd 选择的 action 创建 pane, 而不是按 pane 中是否运行 agent 判断.  raw tmux split 继续没有 slot, 因为 AoE 没有为其分配 lifecycle.

### 3. 复用现有 slot-driven restart 和 cold recovery

现有 `resume_launch_pane` 已能识别 `shell` slot 并调用 shell command builder.  现有 `resume_all_tracked_panes` 和 cold recovery 也已经遍历 slots.  本变更不新增第二套恢复流程, 而是确保 shell 在 launch 时进入现有恢复源, 并修正其 builder.

Codex pane 的 xats identity key 仍从原 slot/instance durable state 恢复.  shell slot 不创建 identity key.  测试必须同时证明 Codex key 未变化和 shell pane 被重新创建.

### 4. 不回填既有 slotless shell pane

旧运行中 pane 没有可靠的 durable ownership marker.  仅凭 cwd 或 pane command 推断可能错误收编用户手工 split 的 pane.  因此本变更只保证新创建、重启后重建或冷恢复后重建的 managed shell pane进入 slot.

### 5. tmux 验证必须完全隔离

所有新增 tmux 测试 SHALL 使用私有绝对 socket, 并从子进程环境移除 `TMUX` 和 `TMUX_PANE`.  HOME 和 shell profile 使用临时目录.  测试只清理自己创建的精确 session 或私有 socket, 不接触默认 tmux server.

shell 启动测试在临时 Bash profile 写入可观察 sentinel 或阻塞命令, 同时设置用户 shell 为 zsh.  断言目标 zsh 到达可交互状态且 Bash sentinel 未执行, 从执行结果证明未读取无关 Bash 登录配置.

## Risks / Trade-offs

- [同目录 shell 开始占用一个 slot] -> 这是完整 restart/recovery lifecycle 的必要成本.  四 slot 上限保持不变, UI 和错误路径继续复用现有 cap handling.
- [用户 shell 的 login 初始化本身仍可能阻塞] -> 这是用户所选 shell 的真实行为.  本变更只消除无关 Bash 初始化, 不绕过用户 shell 自身配置.
- [旧 session 的 slotless shell 不会原地获得 recovery] -> 避免错误收编 raw pane.  用户下一次通过受管理 action 创建 pane, 或 session 被 durable state 重建后, 新规则即生效.
- [primary 和 extra command builder 共享后可能改变 quoting] -> 增加包含空格目录和 shell executable 的 focused tests, 并保持现有 `shell_escape` 模式.

## Migration Plan

不需要 schema migration.  durable slot schema 已支持 `agent = shell`、pane id 和 cwd.  部署后新 launch 写入 shell slot.  既有数据库记录保持不变, 不做推断式 backfill.

回滚只需恢复旧 command builder 和 slotless 例外, 不涉及 schema rollback.  回滚后已经存在的 shell slot仍是合法记录, 但旧版本对其后续行为需要按旧实现判断, 因此本变更不承诺向后兼容回滚.

## Open Questions

无.
