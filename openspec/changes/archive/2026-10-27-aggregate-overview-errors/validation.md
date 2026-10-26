# 验证记录

独立规格核验 PASS, 2 项需求、3 个场景无 CRITICAL/WARNING.  fmt、check --tests、build、clippy 和 sentinel smoke 通过.  

aoe-tester 窄范围 E2E 8 组 PASS, 无失败或阻塞.  使用真实 AoE CLI、可控 tmux inventory 和本地 HTTP 服务, 不声称重跑前轮真实 tmux/xats 完整验收.  

- JSON、纯文本、HTML 的 HTTP 404 均为 46 个独立 pane 聚合一条错误, affected_count 为 46, pane 原因与 partial/unknown 保留.  
- 混合来源、完整原因文本、跨 profile 分离计数, 成功 pane 不计入失败数, 首次 observed_at 保留.  
- 404 正文延迟 6 秒时 CLI 约 38 毫秒返回, 不读取正文.  400/503 具体错误保持.  
- 每目标每轮只调用一次 lookup, 无重试/缓存/控制操作, 查询前后文件树、内容 hash、mtime 和权限不变.  

证据: `.e2e-test/runs/overview-errors-independent-20261027/`.  Linux 二进制 SHA256: `3c886ec17b2bf818dccbafee3e7a04530640888a7598c0dca17a77d479a3b2a8`.  容器与工作区源码均匹配冻结 manifest, 一次性断网无挂载容器已删除.  
