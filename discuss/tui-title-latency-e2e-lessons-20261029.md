本轮标题编辑延迟定位的项目经验.  

- rename_selected 在 UI 线程同步拿 profile operation 锁.  reconcile_all 整个 profile 扫描持同一锁时, JSON/SQLite 本身读得快也不能排除十秒卡顿.  应分别测 e 到 dialog 与 Enter 到保存, 并对齐实际锁 owner.  
- Store::open_with_schema 会 prune_events, 不是只读查询.  SQLite writer 可使恢复标记查询等待多轮 busy timeout, 只读路径应单独验证 writer 活跃时的响应.  
- 分段释放后台锁后, GC 不仅要比较首尾 JSON.  中途缺席后原样恢复的 ABA 也可能误删旧 capture, 需要持久记住本轮曾观察到元数据变化.  
- 性能 trace 有自身开销.  同一夹具做前后比较, 并保留无 trace 的原生对照.  受控 Linux 慢调用结果不能声称是现场 Mac 的实测耗时.  
- 真实 shell 重启夹具需接住产品附加参数.  单纯 sh override 遇 YOLO 参数会直接退出, 后续 primary pane 丢失不能先归因产品.  
