本轮独立验收记录, 正式报告保存在 .e2e-test/runs/cli-tui-restart-parity-20261029-102023-oh06bcqa.  tester 未改产品代码或 skill.

- owned xats Codex 的空 slot native 不足以判定热恢复失败.  必须在实际 pane 输入任务, 核对实际工具回执、原生 UUID、历史和新回合.  managed send 只访问保存的 thread, 不能单独证明 GUI 当前会话一致.
- placeholder 不应提前启动并绑定 owned runtime.  原冷 r/R/c/C 因提前 bind 新坐标复现 HTTP 409.
- 冷恢复前要验证并解除已消失的物理 pane 绑定.  tmux server 重建后, 新主 %1 可能碰撞旧子 %1, 保留旧编号会误触 foreign 安全保护.
- pending transition 的 foreign 预检必须在任何换代或 SQL 写入之前.  合法的 disposable pending 种子独立复现过拒绝前 generation 1->2.
- 解绑、placeholder 失败、snapshot 写失败和立即再次冷恢复都影响旧布局编号映射.  不能依赖后台 reconcile 最终补写来证明同步恢复正确.  受控 exit73 和 SQL trigger 只在一次性夹具内使用, 必须如实标明种子及故障注入.
- macOS 证据路径不能仅靠 r/R 或 c/C 区分.  容器原始归档保留, 正式导出用 lower-r/upper-R 等无碰撞名称并记录映射.
- 真实 Codex 输入要等待渲染后提交, 恢复后等待新 PID 和必要的 xats recovery 注册.  本地模型回答使用当前唯一 marker, 不用旧输出冒充新回合完成.
- attach 状态栏会减少一行 viewport.  布局断言比较分割几何, 单独记录可用高度, 避免误报.
- 最终报告按实际二进制说明复用的旧候选已绿矩阵.  无法执行的真实 Claude 边界保持 BLOCKED, 独立静态 review 不替代实际 E2E.
