# CLI session 的 worktree 分支显示

2026-10-29, 用户反馈第 18 号 session 缺少 worktree 后缀.  按现场 manual 排序核对, 对应 `105a0688d54b4ed8`, 标题为 `monkeys-copy`.  只读 Git 查询确认其目录是 `auto-similar-settings` linked worktree, 但 session 和主 pane 没有 worktree 元数据.

TUI 创建 worktree 时会保存分支及清理归属信息.  managed CLI 只接收已有目录, 不创建或接管 worktree, 因而没有这些元数据.  原列表仅依据元数据追加分支名, 导致实际目录属性与显示不一致.

本轮在 HomeView 加载和 reload 时只读识别没有元数据的本机 linked worktree 根目录.  分支名仅放入独立内存缓存, 渲染仍优先使用已保存的 workspace 和 worktree 信息.  普通仓库、非 Git 目录和 detached HEAD 不追加补充分支名.

项目经验: 分支显示与 worktree 清理归属是两件事.  不能为了补显示而写入托管元数据, 否则可能改变删除 session 时的行为.  显示测试必须同时验证持久元数据不变、外部 worktree 保留, 并覆盖已有 CLI session.

候选产物及验证记录位于 `target/review-builds/20261029-worktree-label/`.  独立 reviewer 为 Hypatia, 静态审查 PASS.  宿主编译、Clippy 和格式检查通过; 独立 tester 在无宿主挂载的断网容器中完成 21 项验收, 全部 PASS.  最终 Linux 构建与 Mac 候选的 203 项生产源码和 Cargo 输入逐项匹配.  测试容器已按准确名称删除, 未操作业务 session.

证据报告位于 `.e2e-test/runs/cli-worktree-suffix-20261029-050216-m4kf4xbh/report.html`.  Tailscale 报告服务被自动审批拒绝, 原因为缺少向具体网络目的地共享证据的明确授权, 因此保留本机报告.

已沿用用户此前的全局安装授权更新 `/Users/jtianling/.cargo/bin/aoe`, SHA256 为 `ce246985a1a659a73e19a05b156d4eb76c01413a5c88d221f8503cc32c495491`.  旧二进制备份及安装记录位于候选目录, 未重启运行进程.  用户正常重新打开 TUI 后, 已有 session 即可显示补充分支名.
