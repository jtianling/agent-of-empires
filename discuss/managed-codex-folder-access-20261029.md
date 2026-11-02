steward 新建的 5 组双 pane Codex 虽已启用 YOLO, 仍停在 Folder access 提示.  steward 通过准确 owner context 和 pane ID 发送 1 后继续, 因任务已开始, 没有重启业务 session 来复现.

独立隔离复现确认 managed create 约 0.961 秒返回, 2.27 秒观察到 Folder access, 22.72 秒仍在等待.  服务侧 thread/resume 响应为 approvalPolicy never 和 dangerFullAccess, 说明问题是目录信任确认, 不是 YOLO 权限没生效.  该取证调用建立了临时客户端订阅, 不能称为严格只读.  复现没有模型 turn, 本地模型夹具请求数为 0.

此前模型与 effort 测试预先写入了项目 trust, 掩盖了真实新目录的启动确认.  以后验收无人值守创建路径, 必须至少覆盖一个无 trust 记录的全新目录和一个独立 linked worktree, 不能只测预授权目录.

修复仅针对有 xats 身份的 managed YOLO Codex.  在配置读取子进程、owned app-server 参数和每 pane 独立客户端 profile 中, 给准确工作目录增加进程内 trust 覆盖.  不写共享 Codex 配置, 非 YOLO 启动不增加该覆盖.  TOML 字符串键处理路径里的空格、点和引号.

第一版候选 `a3be85f665873f744910ee6f28c7fe2f4f23c5b2667dbe3eb8ba684e53081050` 独立验收失败, 未安装.  Codex 的 `-c` 覆盖键不会按 TOML 解析带引号的路径键, 直接传 `projects."路径".trust_level` 会生成包含字面引号的错误键.  含引号的路径还可能被 TOML 字符串显示为三引号, 不能当成 dotted key 使用.  该版还在 linked worktree 创建期间触发了共享配置新增主仓库 trust.

修正为顶层 `projects=<TOML inline table>` 参数, 让路径只在值内序列化.  最终候选 SHA-256 为 `e6e620aadc6182154dfffed0bb05026666524719701277d772ea22757f889825`, 已通过编译、格式检查、独立静态审查及 5 项独立 runtime 验收.  普通主目录、特殊字符主目录和 linked worktree 附加 pane 均直接进入输入界面, 冷恢复保持 thread ID 且无提示, 非 YOLO 控制组仍显示目录确认.  共享配置快照哈希全程一致.  全部使用本地 provider 夹具, 模型请求为 0, 不宣称本轮完成真实 provider 的独立验证.

最终报告为 [独立验收 HTML](../.e2e-test/runs/managed-folder-inline-complete-20261029-133707-70u_ib74/report.html).  沿用 jt 当前会话已有的全局安装授权, 已原子替换 `/Users/jtianling/.cargo/bin/aoe`, 安装后哈希一致.  旧版备份为 `target/candidates/aoe-before-global-install-a6534448c86782d62c781ac2c5d9c591d3eda33fcc35e3e58a7e8009dfab4f4e`.  未重启 TUI 或业务 session.  新 CLI 后续创建和恢复时使用修复, 当前旧 TUI 进程若要使用修复后的启动逻辑, 需要由 jt 手动重开; 本次未新增存储字段, 无须为了 CLI 写入而强制重开 TUI.

单独给前台 Codex 增加 YOLO flag 也已隔离对照, 不能消除 Folder access 提示.  原问题报告为 [.e2e-test 本机 HTML](../.e2e-test/runs/managed-folder-access-20261029-132539-tru_30tj/report.html), 第一版失败报告为 [独立验收 HTML](../.e2e-test/runs/managed-folder-trust-fix-20261029-132948-xl80lnya/report.html).
