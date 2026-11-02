# steward 切换账号后的恢复记录

2026-11-02, steward session `33c7003b20bf4dc5` 的原 thread 为
`01a17d35-b67d-73e0-a0c6-3ab83b29b25e`, pane 为 `%59`, profile 为
`auto-coworkspace-bedc`.  用户从 cc-switch proxy 切换到本地 Codex 账号后,
当前 provider 为 `openai`, thread 数据库记录仍为 `custom`.

Codex 0.166.0 的真实 RPC 返回 `modelProviderNotConfigured`, 即使
`thread/resume` 显式传入 `modelProvider=openai` 也拒绝恢复.  请求级临时补齐旧
provider 定义后, 返回 `modelProviderMismatch`, 仍不允许跨 provider 恢复.
不能再把旧版本允许切换 provider 的行为当作当前版本保证.

确认原 thread 为 `notLoaded`, UUID 和 cwd 与持久绑定一致后, 备份原数据库行和
完整 rollout, 仅按完整 UUID、旧 provider 和 cwd 做 CAS 更新, 将
`state_5.sqlite` 中这一行的 `model_provider` 从 `custom` 改为 `openai`.
没有改写 rollout 或对话内容, 没有重启共享 Codex、xats 或 tmux server.
这是本次有备份的人工数据恢复, 尚未实现为 AoE 自动迁移功能.

原始 AoE 状态备份位于
`target/recovery-backups/steward-20261102T041139Z/`.
原 thread 行和完整 rollout 备份位于
`target/recovery-backups/steward-provider-20261102T041517Z/`.

验收中的临时 Rust 程序调用了 AoE 库的 Resume 路径.  该路径使用
`current_exe()` 启动 `__codex-name` 后台助手, 临时程序未分发这些参数, 导致
助手再次执行 Resume, 产生额外重启及未完成的启动 fence.  已纠正临时程序的
隐藏子命令分发, 确认没有残留诊断进程, 备份并回滚未完成 fence, 保留递增代次和
原验证 thread, 再执行一次正常 Resume.  最终 generation 为 13,
状态为 `awaiting_launch`, 不宣称新 launch 的 xats 绑定已经完成.

项目诊断教训: 通过外部小程序调用启动流程前, 必须搜索 `current_exe()` 用法.
涉及自启动子命令的库代码应由真实 AoE 可执行程序承接隐藏子命令, 临时程序不得
忽略参数后再次执行主操作.

最终真实界面显示原历史及 `steward@default`, `thread/read` 确认同一 UUID,
provider 为 `openai`, 状态为 `idle`.  两个新增或变更的 hook 选择本次不信任并继续,
没有写入永久信任.  没有发送模型测试消息.  `cargo build --lib --bin aoe` 通过,
因机器存在实时 session, 未运行 tmux 测试或全量测试.  AoE 源码未因本次修复而修改.

额外发现: 普通 CLI restart 会因已关闭的历史 shell pane `%60` 拒绝整组重启.
本次仅通过 TUI 相同的逐 pane Resume 路径恢复准确的 slot 0, 没有删除历史 shell
slot, 没有扩大修改范围.

后续用户要求将 provider 对齐纳入 Resume 和 Restart, 已实现共用恢复入口的自动
备份、准确记录迁移及恢复结果校验.  本地恢复连接真实 daemon, 不使用独立
stdio app-server 的加载状态判断正在运行的 daemon.  `codex app-server proxy`
转发原始 WebSocket 字节, 不是 JSONL RPC 桥接; 必须先完成 WebSocket 握手.

恢复失败后只在同一服务再次确认 `notLoaded` 时回滚.  超时、断连和取消不能证明
恢复没有执行, 此时保留目标 provider 和私有备份, 明确记录结果未知, 不盲目回滚.
加载状态只代表被查询服务, 不能宣称跨 app-server 的全机器排他保证.

用户要求停止 Docker 验收, 随后要求编译并安装全局版本供手动试用.  已安装
`target/dev-release/aoe` 到 `~/.cargo/bin/aoe`, 原二进制备份为
`target/candidates/aoe-before-provider-alignment-20261102`.
本机临时 HOME 和 CODEX_HOME 中的真实 Codex 0.166.0 验收通过, 没有复制账号或
真实历史, 没有发送模型消息.  本轮证据位于
`.e2e-test/runs/codex-provider-resume-20261102-043607-8fq11kyq/`.
