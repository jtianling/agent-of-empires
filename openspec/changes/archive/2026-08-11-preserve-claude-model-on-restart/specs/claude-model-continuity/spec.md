## ADDED Requirements

### Requirement: 模型探测仅适用于 claude pane

模型连续性 SHALL 只对 `agent` 为 `claude` 的 pane 生效.  其他 agent (codex, opencode, kimi, shell 等) 的 pane SHALL 既不被探测, 也不被注入模型 flag, 其启动命令 SHALL 与本 capability 引入前逐字节相同.

#### Scenario: 非 claude pane 不受影响
- **WHEN** 一个 slot 记录的 `agent` 不是 `claude`
- **THEN** 系统 SHALL NOT 读取任何 transcript 来推断它的模型
- **AND** 为它构建的启动命令 SHALL NOT 包含模型 flag

#### Scenario: claude pane 参与探测
- **WHEN** 一个 slot 记录的 `agent` 是 `claude`
- **THEN** 系统 SHALL 尝试从该 slot 的 claude transcript 探测当前模型

### Requirement: 当前模型从 claude transcript 尾部探测

系统 SHALL 从 pane 自身会话对应的 claude transcript 文件推断该 pane 当前使用的模型.  transcript 路径 SHALL 由 slot 的 `cwd` 推导出 project 目录名 (绝对路径中的 `/` 全部替换为 `-`), 与 slot 的 `native_session_id` 组合为 `<home>/.claude/projects/<project-dir>/<native_session_id>.jsonl`.

系统 SHALL 从文件尾部读取一个有界窗口 (至少 1 MiB), 丢弃窗口内第一条可能被截断的行, 然后自后向前查找第一条同时满足以下全部条件的 JSON 行, 并取其 `message.model` 作为探测结果:

- `type` 等于 `"assistant"`
- `isSidechain` 不为 `true`
- `message.model` 存在, 非空, 且不等于 `"<synthetic>"`
- `message.model` 是一个合法的模型标识: 长度不超过 128, 且只由 ASCII 字母数字与 `-` `_` `.` `:` 组成

读取 SHALL 有界, 不得为了寻找匹配行而读入整个文件.

探测值最终进入由 shell 执行的 `--model <model>` 命令行, 因此模型标识校验 SHALL 在探测时与命令构建时各执行一次, 与持久化 resume token 的处理方式一致.  不通过校验的值 SHALL 被视为"未观测到", 既不写入持久化模型, 也不注入命令.

模型标识 SHALL 按 transcript 中记录的原样保存.  transcript 不记录 `[1m]` 这类上下文窗口变体标记, 系统 SHALL NOT 尝试推断或补回该标记.

#### Scenario: 取最后一条有效 assistant 条目的模型
- **WHEN** 一个 claude pane 的 transcript 尾部依次包含 `claude-opus-5` 与 `claude-fable-5` 两条有效 assistant 条目
- **THEN** 探测结果 SHALL 为 `claude-fable-5`

#### Scenario: 子代理条目被跳过
- **WHEN** transcript 中最后一条 assistant 条目的 `isSidechain` 为 `true`
- **AND** 它之前存在一条 `isSidechain` 不为 `true` 的 assistant 条目
- **THEN** 探测结果 SHALL 取那条非 sidechain 条目的模型
- **AND** SHALL NOT 取 sidechain 条目的模型

#### Scenario: 合成条目被跳过
- **WHEN** transcript 中最后一条 assistant 条目的 `message.model` 为 `"<synthetic>"`
- **AND** 它之前存在一条模型值有效的 assistant 条目
- **THEN** 探测结果 SHALL 取那条有效条目的模型

#### Scenario: 超长行不阻碍探测
- **WHEN** transcript 尾部存在单行长度超过 256 KiB 的 assistant 条目
- **THEN** 尾部读取窗口 SHALL 足够大以完整覆盖该行
- **AND** 探测 SHALL 返回该行的模型

#### Scenario: 不合法的模型标识被丢弃
- **WHEN** transcript 中最后一条 assistant 条目的 `message.model` 含有 shell 元字符 (例如 `x; rm -rf /`)
- **AND** 它之前存在一条模型标识合法的 assistant 条目
- **THEN** 探测结果 SHALL 取那条合法条目的模型
- **AND** 不合法的值 SHALL NOT 出现在任何启动命令中

#### Scenario: 变体标记不被补回
- **WHEN** 某个会话实际以 `claude-opus-5[1m]` 启动, 而 transcript 记录的是 `claude-opus-5`
- **THEN** 探测结果 SHALL 为 `claude-opus-5`
- **AND** 系统 SHALL NOT 追加 `[1m]`

### Requirement: 观测到的模型按 slot 持久化并周期刷新

探测到的模型 SHALL 持久化在该 pane 的 `agent_slot` 记录上, 而不是 instance 级别.  同一 instance 的不同 slot SHALL 能各自保存不同的模型.

reconcile 在收敛 slot 时 SHALL 刷新该值.  刷新 SHALL 使用廉价的文件指纹 (transcript 路径、mtime 与文件长度) 跳过自上次探测以来未变化的文件, 不得在每个 tick 重读未变化的大文件.

该指纹 SHALL 与模型一样按 slot 持久化, 而不是保存在进程内: reconcile 由多个进程驱动 (home-view poller 与 notification monitor), 进程内缓存只能让先探到某文件的那个进程跳过, 另一个进程仍会每 tick 重读.  指纹 SHALL 在每次探测后写入, 包括探测未观测到模型的那次, 使无 assistant 条目的 transcript 也不会被反复重读.

模型 SHALL 是 slot 的属性而非会话的属性: 一次 fresh restart (开启新对话) 之后, 该 slot SHALL 继续沿用上一次观测到的模型.

#### Scenario: 同一 instance 的两个 pane 保存不同模型
- **WHEN** instance 的 slot 0 跑 `claude-opus-5`, slot 1 跑 `claude-fable-5`
- **AND** reconcile 完成一轮收敛
- **THEN** slot 0 的持久化模型 SHALL 为 `claude-opus-5`
- **AND** slot 1 的持久化模型 SHALL 为 `claude-fable-5`

#### Scenario: 模型跨进程重启存活
- **WHEN** 一个 slot 保存了非空模型
- **AND** AoE 关闭后重新打开
- **THEN** 该 slot SHALL 读回同一个模型值

#### Scenario: 未变化的 transcript 不被重读
- **WHEN** 一个 claude pane 的 transcript 自上次探测以来 mtime 与长度均未变化
- **AND** reconcile 再次运行
- **THEN** 系统 SHALL NOT 重新读取该文件内容
- **AND** 该 slot 的持久化模型 SHALL 保持不变

#### Scenario: fresh restart 后模型仍然沿用
- **WHEN** 一个 slot 观测到的模型为 `claude-fable-5`
- **AND** 用户对该 session 触发 fresh restart, 开启一个全新对话
- **THEN** 该 slot 的持久化模型 SHALL 仍为 `claude-fable-5`

### Requirement: 每条 claude pane 启动命令都注入观测到的模型

当一个 claude pane 存在非空的持久化模型时, 为它构建的启动命令 SHALL 追加 `--model <model>`, 且 SHALL 追加在实例 `extra_args` 之后.

该注入 SHALL 发生在公共命令构建出口上, 因此 SHALL 对所有启动与重启路径一致生效, 包括但不限于 `Shift+R` (resume + attach)、`Shift+C` (fresh + attach)、小写 `r` 与 `c` (不 attach)、cold-start recovery、多 pane fan-out 与 fork.

注入 SHALL 同时适用于 primary pane 与非 primary pane.

当 `extra_args` 中已经写有 `--model`, 探测值 SHALL 生效: 系统 SHALL 依赖 claude 对重复 `--model` 取最后一个的语义, SHALL NOT 解析或改写 `extra_args`.

#### Scenario: resume 重启带上观测到的模型
- **WHEN** 一个 claude pane 观测到的模型为 `claude-fable-5`
- **AND** 用户触发 resume 重启
- **THEN** 该 pane 的启动命令 SHALL 同时包含 resume flag 与 `--model claude-fable-5`

#### Scenario: fresh 重启带上观测到的模型
- **WHEN** 一个 claude pane 观测到的模型为 `claude-opus-5`
- **AND** 用户触发 fresh 重启
- **THEN** 该 pane 的启动命令 SHALL NOT 包含 resume flag
- **AND** SHALL 包含 `--model claude-opus-5`

#### Scenario: 非 primary claude pane 也带模型
- **WHEN** 一个 instance 的非 primary slot 记录的 agent 是 `claude` 且有观测到的模型
- **AND** 该 pane 被重启
- **THEN** 它的启动命令 SHALL 包含 `--model <model>`

#### Scenario: 探测值排在 extra_args 之后
- **WHEN** 实例 `extra_args` 为 `--model sonnet`
- **AND** 该 pane 观测到的模型为 `claude-fable-5`
- **THEN** 启动命令中 `--model claude-fable-5` SHALL 出现在 `--model sonnet` 之后

### Requirement: 探测失败不改变启动行为

当模型无法被探测时 (transcript 文件不存在、尚无 assistant 条目、读取或解析失败), 系统 SHALL 保留该 slot 上一次已知的模型值, SHALL NOT 将其清空.

当该 slot 从未观测到任何模型时, 启动命令 SHALL NOT 包含模型 flag, 且命令内容 SHALL 与本 capability 引入前一致.

探测失败 SHALL NOT 阻塞、延迟或以任何方式改变重启流程, 也 SHALL NOT 使重启报错.

#### Scenario: 新会话尚无 assistant 消息
- **WHEN** 一个 claude pane 刚启动, transcript 中还没有任何 assistant 条目
- **AND** 该 slot 从未观测到模型
- **THEN** 探测 SHALL 返回空
- **AND** 启动命令 SHALL NOT 包含模型 flag

#### Scenario: transcript 缺失时保留旧值
- **WHEN** 一个 slot 已保存模型 `claude-opus-5`
- **AND** 其 transcript 文件之后不可读或被删除
- **THEN** 该 slot 的持久化模型 SHALL 仍为 `claude-opus-5`
- **AND** 后续重启 SHALL 仍然注入 `--model claude-opus-5`

#### Scenario: 解析错误不影响重启
- **WHEN** transcript 尾部内容不是合法 JSON
- **AND** 用户触发重启
- **THEN** 重启 SHALL 正常完成
- **AND** SHALL NOT 因探测失败而报错或中止
