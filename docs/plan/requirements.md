- [ ] 增加 Ctrl+b a 回到 aoe 自己的 main session.
- [ ] 增加鼠标支持, 点击 aoe main session list的不同 session 名字, 可以直接跳转到aoe管理的对应 session 
- [ ] aoe 的 session 会 size 比较窄, 很多地方需要做显示的优化:
    1. Agent of Empires [ xxx profile ] 的标题改成 AoE[xxx profile]
    2. session 的 list 里面目录改成 aoe 启动的相对路径, 从工作区 ./ 开始
- [ ] New Session 的界面在aoe 显示宽度较窄时, 显示不下, 一些选项改简短的名字
    1. Right Pane=> R-Pane
    2. YOMO Mode后面的 Skip permission prompts 直接去掉, 只留一个 [x] 框
    3. Worktree Branch => Worktree
    4. Sandbox: [ ] Run in Docker => Sandbox: [ ] in Docker
    5. Tab next <-/-> 的工具栏, 按空格分隔, 超出当时的宽度就换行



## 延伸需求文档

以下几件由 2026-07-28 的排查沉淀而来, 各自单独成篇:

- [e2e 高负载不稳定治理](e2e-load-flakiness.md) —— 一天内三方撞到六次"全量跑挂一条、单跑必过", 已在污染每轮验收的信噪比
- [xats 身份连续性](xats-identity-continuity.md) —— aoe 侧已交付并实机验证, 端到端阻塞在 daemon 的 npm 发布; 含一个"看起来在收敛、实际是死循环"的坑; 结论是不做绕路, 等发布
- [codex 的 pane 从来没有被追踪过](codex-pane-tracking.md) —— 后果不是恢复得不好, 而是恢复成了别的 agent
- [测试的清理必须覆盖失败路径](test-cleanup-on-failure.md) —— Drop guard 的做法和验证方法
