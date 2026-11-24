# Antigravity CLI

在 AoE 的新建会话窗口选择 `antigravity`, 或用 CLI 在当前目录打开原生 `agy` 界面.

```sh
aoe add . -c agy --launch
```

需要团队通信时, 为对应窗格勾选 `Cross Agent Team`, 或在创建时加上参数.

```sh
aoe add . -c agy --cross-agent-team --launch
```

机器上安装了多个版本时, `-c` 也接受 `agy` 可执行文件的绝对路径.  例如 `aoe add . -c "$HOME/.local/bin/agy" --cross-agent-team --launch` 会保留这份可执行文件用于恢复.

首次加入团队时, 在 agy 中明确给出自己的名称和团队, 例如 `注册到 xats, 名称 writer, 团队 docs`.  AoE 新建窗格时先启动尚未命名的受控 PTY.  第一次 `register_agent` 成功后, 同一运行中的窗格即可被新邮件唤醒.  不同成员请使用不同名称.

AoE 通过 Antigravity 的 `SessionStart` hook 记录每个窗格的 conversation UUID.  按 `r` / `R` 恢复时, AoE 用该窗格的 `agy --conversation <UUID>` 打开原对话.  如果首次模型调用尚未发生, 或采集时本机存储持续被占用, 窗格可能还没有 UUID, 此时精确恢复会报错.  按 `c` / `C` 清空上下文时, AoE 不传 `--conversation`, 在新对话中以该窗格持久化的身份 key 通过 `xats-agy` 自动恢复原来的名称、团队和 PTY 空闲唤醒.  重新连接本机 xats 时, MCP 进程用该窗格持久化的身份恢复名称和团队.

回复同团队成员时, 可将邮件里的 `from_agent_id` 作为 `send_message_by_id` 的 `to_agent_id`, 并在 `body` 中填写回复.  跨团队回信使用 `send_message`, 将邮件里的 `from_name` 和 `from_team` 分别传给 `to_agent_name` 和 `to_team`.

旧版本已经用普通 `agy` 启动并注册的窗格, 可在新版 AoE 首页选中后按小写 `r` 恢复一次.  AoE 按该窗格的身份 key 查询已有名称和团队, 用 `xats-agy` 打开原对话.  重开首页只会更新 AoE 界面进程.

新建窗格需要 PATH 上的 `xats-agy` 和 `xats-agy-mcp` 均支持延后注册, 且本机 xats daemon 支持 Antigravity 唤醒通道.  当 `xats-agy --capabilities-json` 报告 `agentapi_channel: 1` 且 PATH 存在 `xats-agy-channel-bind` 时, AoE 会启用无 PTY 的 `xats-agy --agentapi` 模式, 让 `agy` 直接运行在原生 tmux TTY 上, 并通过会话级 `--settings` 合并用户已有 `permissions` 后精确放行 `xats-agy-channel-bind` (不改写磁盘 `settings.json`, 完整保留用户 `statusLine` 等设置).  Named 恢复 (`r`/`R`、`c`/`C`、冷恢复) 会自动通过 `-i` 触发一次极简握手绑定原生 `agy agentapi send-message` 通道; Deferred 新建不传 `-i`, 待首次 `register_agent` 后执行 `xats-agy-channel-bind` 即可升级; 会话内触发 `SessionStart` 时 `aoe __record-pane` 也会同步更新 `conversation_id`.  旧版 `xats-agy` 自动退回原 PTY 唤醒模式.  AoE 使用自己的 `aoe-xats` MCP 条目启动桥.  要无人值守地读信回信, 需在 agy 中允许该服务的收信及对应回信工具.

AoE 在宿主机用户的 `~/.gemini/config/hooks.json` 安装会话采集 hook, 并在启动前自动将窗格目录写入 `~/.gemini/antigravity-cli/settings.json` 的 `trustedWorkspaces` 以跳过文件夹信任确认.  开启团队通信还会在同目录的 `mcp_config.json` 添加名为 `aoe-xats` 的 stdio 服务, 由当前 AoE 可执行文件提供.  本机 xats daemon 需处于运行状态.  配置启用 bearer 认证时, 需让启动环境提供 `CROSS_AGENT_TEAMS_MCP_TOKEN`.  凭据由本机 MCP 进程处理.  此接入使用普通 AoE 窗格, `managed create --xats-name` 的命名托管流程尚未支持 Antigravity.
