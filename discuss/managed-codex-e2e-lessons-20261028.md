本项目 managed Codex 验收记忆, 记录于 2026-10-28.  
以下结论来自隔离 Linux 容器中的真实 Codex 0.165.0, 不能仅凭参考源码推定其他版本.  

- `app-server` 不支持 `-p/--profile`, 实际 CLI 会直接退出.  
  私有 profile 可以供 remote TUI 使用, server 配置加载必须另做真实验证.  
- Codex `-c` dotted key 按点拆分, 不会去掉 key 中的字面双引号.  
  header 名本身不需要引号, 仅字符串 value 使用 TOML 引用.  
- managed token 在进程环境中存在, 不等于 MCP 已发送该 header.  
  应分别核对环境匹配布尔, 只读配置键形状和真实 HTTP header 匹配布尔.  
- `gpt-6.1-sol` 的 code-mode 路径不能用 legacy `function_call` fixture 代替.  
  实测请求可以没有 `tools` 字段, 原生 `custom_tool_call(name=exec)` 仍可调用真实 MCP.  
  只有真实 `custom_tool_call_output` 返回所属任务, 才能证明工具链通过.  
- `thread/read=idle` 不等于旧 app-server 已释放持久线程 writer.  
  新连接 `thread/unsubscribe=notSubscribed` 也不能证明已卸载.  
  必须观察准确线程的真实卸载状态, 不停 shared server, 不换 native ID 绕过.  
  精确 0.165 release 源码的默认卸载延迟为 1800 秒, 不能套用旧参考的 60 秒.  
  配置读取返回 null 时, 不能据此推定默认等待时长.  
  实测可仅对准确 idle 线程设置 delay 0, 退订后观察目标消失及旁路仍 loaded.  
  该路径在旧 server 0.164/0.165 到新 owned server 0.165 的真实迁移中均通过.  
- 活跃 steer 验收需要独立释放模型 fixture 的响应闸门.  
  不能一直等待 `send_message` 返回才释放模型, 否则测试自身可能形成等待环.  
- 资源报告应分开 owned server 子树和精确 pane 根下的 TUI/wrapper 子树.  
  同时记录 RSS/PSS, 单核 CPU 百分比和采样窗口, 不把 shared page 重复量当独占成本.  
- 新建空 `CODEX_HOME` 时, 两个 TUI 并发首次初始化曾触发 SQLite migration 竞争.  
  应单独保留该发现, 后续生命周期用例先通过主 TUI 初始化门, 不把子 TUI 未启动记为 PASS.  
- public human CLI 加载的实例可能没有 `source_profile`.  
  owned record 和主 pane slot DB 必须使用实例已有的 selected profile 解析, 不可按 CWD 另选 profile.  
  验收应显式 `-p` 覆盖不同 env/CWD, 同时测试主/附加 pane 移动拒绝及合法 stop 的真实回收.  
- relaunch 验证失败而 launch 未变化时, 不应清理原来仍有效的 backend.  
  单靠 managed CLI 前置 guard 不能覆盖这个私有 backend 失败路径.  
- 动态 process group 验收不能只确认 leader 已退出.  
  必须观测全部原始 witness 都不活而新成员仍活, 新身份已持久, 再验证产品准确回收新成员.  
- startup record I/O 失败时, Child handoff 到 reaper 不等于服务已终止.  
  验收必须在 fixture 恢复前核对已知 Child/同组 child 和监听, 并保留每层具体错误.  

本地证据在 `.e2e-test/runs/owned-codex-single-20261028-aoe-tester/`.  
完整源码和二进制 hash, 请求形状, 断言及原始失败记录随轮次保存.  
不得把中间候选失败与最终候选结论混为一谈, 也不得存储或传播实际 token/header 值.  
