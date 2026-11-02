Claude managed CLI 的验收需要同时核对逻辑 pane ID, native conversation ID, xats agent ID 和 cursor.  仅看到 runtime_ready 或 JSON UUID 不变不能证明对话已恢复.  本轮用真实 CLI 的工具结果以及同一 transcript 中恢复前后的唯一任务确认连续性.

无 TUI 的 managed CLI 路径不能依赖 TUI reconciler 才把 pane_live 的真实对话保存到 durable slot.  停止前应按当前 pane/identity/generation 验证后保存, 创建新 pane 后不可再用可能复用的旧 tmux 编号判断原位置.

Fresh 失败后的合法中间状态可能是 JSON 已保存新 UUID B, slot/live 仍是旧 A.  重试必须保留 Fresh 意图, 不能以恢复旧 A 换取启动成功, 也不能让这个中间状态永久阻塞恢复.  CAS 更新应等真实新 capture 与保存的 UUID 一致后再做.

普通 managed restart 与 restart --yolo 的重启范围不同.  前者只重主 pane, 后者停止并恢复全体 pane.  验收断言必须对应公开入口的实际语义, 不将未请求重启的 sibling 当作恢复失败.

失败候选可能连正常 remove 都被同一校验错误阻塞.  因此本项目的生命周期负例必须始终局限于一次性断网容器, 记录产品清理错误后精确删除整个自建容器, 不转而清理宿主 tmux.
