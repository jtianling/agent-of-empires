## ADDED Requirements

### Requirement: 稳定 pane 标识与列表

系统 SHALL 提供 `aoe managed pane list <session-id>`, 返回结构化 pane 元数据、受管 pane 的完整持久 ID、slot、当前 tmux pane 和主 pane标识.  人工或未知 pane SHALL 明确标记非受管, 操作 ID 可以为空, 不为列表查询认领或写入人工 pane.  位置 SHALL 仅作为展示信息.  目标操作 SHALL 同时验证完整 session ID 和 pane ID, 不接受原始 tmux ID、位置或可复用 slot 代替.

#### Scenario: 布局变化
- **WHEN** 同一 pane 的位置发生变化
- **THEN** 原持久 ID 仍指向同一受管 pane, 不因左右位置改变而改投其他 pane

#### Scenario: slot 被重新占用
- **WHEN** 旧 pane 消失后另一 pane 使用其 slot 或 tmux 编号
- **THEN** 旧持久 ID 不能控制新 pane

### Requirement: pane 所有权检查

主 pane SHALL 保留 session 的管理权, 新增受管 pane SHALL 记录创建者.  人工添加及未知来源的附加 pane SHALL 默认非受管.  send/capture/remove SHALL 在任何副作用前同时核对父 session、pane owner 和实际占用.  列表 SHALL 不泄露 key、token、配置秘密或非 owner 的终端内容.

#### Scenario: 人工附加 pane
- **WHEN** 管理 agent 在自己的 session 中看到人工新增 pane
- **THEN** 可以查看基本信息, 但不能通过 pane 命令发送、读取内容或删除它

#### Scenario: 跨 session 或跨 owner
- **WHEN** 提交其他 session 的 pane ID 或其他 owner 的 pane
- **THEN** 明确拒绝, 不产生 tmux、xats 或存储变更

### Requirement: 独立创建附加 pane

系统 SHALL 提供 `pane create <session-id> --tool <tool> [--path <path>] [--xats-name <name> --xats-team <team>]`, 只在当前 owner 所有且已运行的 session 中创建.  SHALL 复用四 pane 上限, 默认右侧分屏, 返回新持久 ID.  xats 参数 SHALL 成对验证, 未支持的组合在副作用前拒绝.

#### Scenario: 创建工作 pane
- **WHEN** 在运行中的自有 session 创建一个 pane
- **THEN** 新 pane 拥有独立 ID 和 owner, 原 pane 的进程、内容和身份保留

#### Scenario: 父 session 停止或满额
- **WHEN** 父 session 未运行或四 pane 容量已满
- **THEN** 创建被拒绝, 不隐式启动 session 或占用 xats 身份

### Requirement: 定向输入与输出

系统 SHALL 提供 `pane send <session-id> <pane-id> <message>` 和 `pane capture <session-id> <pane-id> [--lines N]`.  输入 SHALL 原样发送并提交到准确 pane, 输出 SHALL 仅来自准确 pane, 不依赖当前焦点或 pane index 0.

#### Scenario: 两个 pane 分别输入
- **WHEN** 向两个自有 pane 发送不同标记并分别 capture
- **THEN** 每个标记只出现在对应目标, 主 pane 与其他 pane 不收到误投输入

### Requirement: 单独关闭与失败恢复

`pane remove` SHALL 仅删除准确的自有 pane, 清理其 slot 占用、xats claim 和私有启动配置, 保留相邻 pane及代码资源.  删除主 pane 时 SHALL 自动从剩余自有 pane 选择接任者, 保留接任者原逻辑 ID、进程、对话及 xats 身份.  最后一个 pane 的删除 SHALL 等同受限 session 删除.  若剩余 pane 均非当前 owner 所有, SHALL 拒绝而不认领或伤害它们.  失败 SHALL 保留 pane ID 和可重试的 fence/资源状态, 旧 ID 不得在重建后复用.

#### Scenario: 删除附加 pane
- **WHEN** 删除一个自有附加 pane
- **THEN** 其他 pane 继续运行且可正常收发, 代码不变, 后续创建获得新 pane ID

#### Scenario: 删除主 pane
- **WHEN** 使用 pane remove 指定主 pane, 并有另一个当前 owner 所有的 pane
- **THEN** 仅关闭原主 pane, 幸存 pane 自动接任且原 ID、运行状态和协作身份不变, 旧主 pane ID不能再操作新主 pane

#### Scenario: 关闭最后一个 pane
- **WHEN** 删除 session 中最后一个自有 pane
- **THEN** 执行受限 session 删除, 精确释放对应资源并保留代码目录

#### Scenario: 只有人工 pane 可接任
- **WHEN** 请求删除主 pane而剩余 pane 均为人工或其他 owner 所有
- **THEN** 拒绝且全部 pane 保持原状, 不自动转移所有权

#### Scenario: 释放后本地清理失败
- **WHEN** xats 已释放但本地清理失败
- **THEN** 保留准确恢复信息, 重试同一 ID 可完成而不影响邻 pane

### Requirement: 每 pane 独立 xats 和生命周期

每个受管 pane SHALL 使用独立 xats name/team、identity key、claim、token 和运行配置.  SHALL 支持已有 Codex、Claude、OpenCode 自动绑定, 不注入 register/reconnect prompt.  人工/CLI 重启与恢复 SHALL 保留逻辑 pane 身份并推进实际启动的独立 fence, 不修改邻 pane 的身份.  helper SHALL 拒绝旧 pane 占用或旧 fence.  父 session 删除 SHALL 精确释放其可管理 pane 的全部资源.

用户在实现期间明确暂缓 Claude, 本轮交付验收 SHALL 以 Codex、OpenCode 及通用 pane 操作为范围.  Claude 已有接入代码保留, 新增 pane 的冷恢复连续性 SHALL 标为暂缓, 不宣称已通过或已修复.

#### Scenario: 同类双 agent 协作
- **WHEN** 在同一 session 创建两个相同类型但不同 xats name 的 agent pane
- **THEN** 两者可独立就绪, name/team、准确运行坐标与配置互不覆盖, 删除一个不释放另一个

#### Scenario: 重启和旧 helper
- **WHEN** 一个 pane 被重新启动而旧启动 helper 延迟执行
- **THEN** 新代使用原逻辑身份, 旧 helper 被拒绝, 邻 pane 保持原绑定
