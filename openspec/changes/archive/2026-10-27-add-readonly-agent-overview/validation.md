# 验证记录

2026-10-27, 独立规格核验第二轮 PASS, 无 CRITICAL/WARNING.  首轮发现的 TUI 切换 profile 登记遗漏、空白 JSON 被误作空列表均已修复并覆盖.  

宿主通过 cargo build、cargo check --tests、cargo clippy --all-targets -- -D warnings、cargo fmt --check 与无真实 tmux 的 overview_cli_smoke.  未在宿主运行全量测试或 tmux 探针.  Linux unused Command 导入警告经基线对照确认是既有问题.  

aoe-tester 独立 E2E: 12 组 PASS, 0 最终 FAIL, 0 BLOCKED; Claude 1 组 user-deferred.  证据位于 `.e2e-test/runs/readonly-overview-independent-20261027/`, 含 report.html、manifest.json 和原始日志.  

- 真实多 pane、多 owner、空层级、marker 替换、三个 TUI 与 profile 切换和退出隔离通过.  
- 缺失/空白/损坏数据、活动 WAL、原文件无修改或新增 sidecar、旧登记/PID 复用模拟/PID 1 通过.  
- 7 个精确过滤纯单测、smoke、21 项 HTTP 协议模拟及超时/redirect 通过.  
- 真实 OpenCode 前台进程经真实 tmux/xats 可取得 verified, 登记由隔离 SQLite 夹具构造, 不声称验证了注册流程.  ready 仅模拟, 真实 Codex launch 行、Claude channel 和 remote 403 未实测.  
- xats 首轮真实 503 来自 tmux 3.4 对控制分隔符的转义, 对方修复后复验通过.  

Linux AoE 二进制 SHA256: `80a4bc17e5058736b6aaec9096810c7c3ba40cb2073997812639ef9c868cebb2`.  xats 构建 SHA256: `50028467ef7c4826110c57db8d65a6816e55acdb5993fedc1fb85ebb470802fa`.  容器及工作区源码 hash 均与冻结 manifest 一致.  

隔离容器无宿主挂载、无网络、无付费模型调用, 已删除并确认不存在.  仅保留授权工具镜像与测试证据, 未部署或重启生产 xats.  
