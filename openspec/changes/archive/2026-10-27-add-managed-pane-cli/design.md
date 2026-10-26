## Context

session 已有稳定管理身份、JSON、operation lock 和 xats managed v1.  AgentSlot 记录配置、native session、tmux pane 和 xats key, 但 slot 编号会复用, 尚无 pane 的独立管理 owner.  现有受控 runtime 以主 pane 为入口, 扩展必须避免把主 pane 的 claim/token 复制给附加 pane.

## Goals / Non-Goals

提供 `aoe managed pane list/create/remove/send/capture`, 支持一个 session 中的多个独立协作者及关闭主 pane 后自动接任.  复用既有四 pane 上限与右侧分屏, 不新增布局编辑器或远程接口.

## Decisions

- 目标同时指定完整 session ID 和返回的持久 pane ID.  list 返回 slot、当前 tmux pane、位置及 primary 标识, 但不允许把 slot、左右位置或原始 `%pane` 当作受限操作句柄.
- 主 pane 的管理权来自 session owner; 受限入口创建的附加 pane 明确记录创建者.  人工或普通 CLI 新增的 pane 默认人工所有.  对附加 pane 的读内容、发消息、关闭同时验证父 session owner 和 pane owner, 校验在副作用之前完成.
- 用户已明确要求主 pane 也可单独关闭.  存在其他当前 owner 所有的 pane 时, 选择确定性的接任者并安全更新主 pane/slot 0 指针.  接任者保留原逻辑 ID、运行进程、native session和xats凭据, 不继承已删除主 pane 的身份.  原主 pane ID 作废, 不把 session ID 永久作为当前主 pane 别名.  只剩一个 pane 时关闭等同受限 session 删除.  剩余 pane 全是人工/其他 owner 时拒绝, 不自动认领或关闭它们.
- pane 身份元数据必须持久化并与当前 slot 占用、tmux pane 及精确 session 对应.  对已关闭/replaced pane 的旧 ID 失败关闭, 不把重用 slot 或 `%pane` 自动认领为原 pane.  可用 tmux pane 级身份标记配合 durable 记录核验, 不凭 cwd、标题或位置推断.
- 创建仅允许在自己所有且已运行的 session 内执行, 不隐式启动父 session.  重用现有 pane 配置验证、split、slot 预留与失败回滚能力.  新工具/path/xats 参数不隐式继承主 pane 的身份、命令 override、native session 或配置文件路径.
- 每个 pane 的 xats 独立 name/team/key/claim/token/client config.  复用已有 managed v1 控制面和 Codex/Claude/OpenCode 适配, 将 runtime context 泛化到准确 pane, 不新增虚假 session 记录.  子 pane helper 按父 session、逻辑 pane、当前启动代际核验后才取凭据.
- session stop/remove/restart 在执行前检查实际受影响 pane, 任一人工/其他 owner pane 会受影响则拒绝.  session 级 send/capture 同样核对当前主 pane 身份.  人工 set-owner 转移 session 时只同步原 owner 的受管 pane, 保留人工 pane属性, 原 owner 立即失权.
- 受管附加 pane 参与既有人工重启、冷恢复及父 session 关闭的资源生命周期.  保留逻辑 pane ID, 实际新启动推进独立 xats fence; 人工替换占用不得继承原 owner.  fork 不复制子 pane 管理凭据或 xats claim.
- 删除先保存足以重试的恢复元数据, 再精确释放目标 xats 身份、终止目标 pane 和清理它的配置.  失败不能丢掉 key/fence, 也不允许顺带清理邻 pane、worktree、branch 或代码.
- 使用现有锁和原子存储, 新持久化形态需要迁移时走 migration 系统.  已有 managed session 的主 pane 保留既有权限, 附加 pane 不做自动接管.

## Risks / Trade-offs

- slot 和 tmux ID 复用 → 持久 pane ID 与当前占用联合核验, 删除后重建必须取得新 ID.
- 人工在受管 session 中工作 → 父级操作也执行受影响 pane 的所有权保护, 不只给新命令加检查.
- runtime 配置文件覆盖 → 按 pane 独立命名和绑定, 测试同类型两个 pane 的 xats 身份与释放.
- 旧 TUI 写回或旁路重启 → 保持现有冲突保护并覆盖人工生命周期, 明确拒绝陈旧占用.
- 旧程序不认识新 pane 元数据 → 不支持混用旧版 TUI, 启用 pane API 前重新打开同 profile 的 TUI, 不停止已托管的 agent session.
- 宿主生产 tmux → 只在断网且无宿主挂载的一次性容器做真实定向 E2E, 宿主仅编译和无真实 tmux 的检查.

## Migration Plan

按现有 session owner 保留原主 pane 权限, 新增字段缺失的附加 pane 默认人工所有.  主 pane 接任后持久记录新的逻辑主 pane 指针, 不回退到旧 ID.  slot 迁移与 runtime helper 提交必须协调; 无法安全处理正在启动的候选时明确失败且保留旧主 pane, 不留下半完成接任.  不修改用户真实 session 或运行 daemon 来试验迁移.

## 当前验收范围

用户在实现期间明确要求暂时不处理 Claude.  本轮完成通用 pane 管理和 Codex/OpenCode 实际生命周期验收.  Claude 接入代码保留, 新增 pane 冷恢复连续性的进一步取证和修复按用户要求暂缓, 不将此项记为 PASS.
