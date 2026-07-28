# xats 身份连续性: aoe 侧已交付, 端到端阻塞在 daemon 发布

## 现状

AoE 侧该做的都做完并实机验证过了:

- 为每个 pane 铸一个 uuid 身份键, 主 pane 的存在实例记录, adopted pane 的存在 `agent_slot.xats_identity_key`
- 重建 pane 时把它注入环境变量.  实机抓到过真实启动命令:

```
AOE_INSTANCE_ID='40b979abe875406b' XATS_IDENTITY_KEY='168411af-951d-42e0-a065-dc8b08ac7c54' \
  /bin/zsh -lc 'stty susp undef; exec env claude --session-id ... --dangerously-load-development-channels ...'
```

端到端仍然不工作, 原因不在 aoe.

## 根因 (2026-07-28 由 xats-main 提供一手证据)

启动提示优先尝试 `$XATS_IDENTITY_KEY` 的那一半**代码已上但没发布**:

- 实现在 `cross-agent-teams-mcp` commit `87697ed` (2026-07-27), identity 分支拼接顺序排在 "Do NOT register automatically" 之前
- npm 上 `cross-agent-teams-mcp` 的 latest 仍是 **0.7.6** (2026-07-21), 早于该 commit
- 走 `npx -y -p cross-agent-teams-mcp@latest` 的 pane 命中 npx 缓存里的 0.7.6, 那份 dist 里**完全没有 `XATS_IDENTITY_KEY` 这个字符串**

所以一个刚起来的 agent 按启动提示的字面指引根本不会去读这个环境变量.  同一台机器上一部分 pane 走本地构建 (含该分支)、一部分走 npx 0.7.6 (不含), 这就是这件事看起来自相矛盾的原因.

daemon 侧现在就能用: `register_agent` 和 `reconnect` 两个接口都已接受 `identity_key`.

## 一个必须知道的坑: 当前状态下不会自我收敛

设计文档原先把"键没绑 → 回落手动注册"写成一次性收敛步骤.  **这个结论是错的**, 已在 `openspec/changes/preserve-xats-identity-across-restart/design.md` 更正:

在 hint 生效之前, 那次回落的 `register_agent` **不带 identity_key**, 于是键仍然没绑上; 下次重启 `reconnect` 依然 `need_register`.  **两边都不报错, 但永远收敛不了.**

实测佐证: 活跃库 `~/.cross-agent-teams-mcp/data.db` 上 `SELECT COUNT(*), SUM(identity_key IS NOT NULL) FROM agents` 结果是 **540 | 0** —— 540 个身份, 零绑定.  列和 UNIQUE 索引都在, 只是从来没有一次注册带过键.

(取证提醒: repo 根目录下那个 `data.db` 是没有 `identity_key` 列的旧库, 别拿它验证.)

## 结论: 不做过渡方案, 等发布

排查过程中一度把它当成"要等很久的外部依赖", 于是评估过两条绕路: 把 pane 的 `.mcp.json` 指到本地构建, 或者由 aoe 在恢复出来的 pane 的首轮 prompt 里代替启动提示点名.  **两条都不做.**

前者绑死本地 checkout —— aoe 是要给别人用的工具, 恢复路径不该依赖某台机器上的某个 git 工作副本; 作为验证手段可以, 作为产品形态不行.

后者等于把 xats 的注册契约复制一份进 aoe (而且必须复制全: `reconnect` 和回落的 `register_agent` 都要带键, 只做前者等于白跑).  发布之后这份复制就成了两处会各自漂移的真相.

而这个前提本身是错的: 2026-07-28 jt 明确说, **只要 xats 那边测试通过, 随时可以发版本**.  没有需要熬过去的过渡期, 就不该为它付复杂度.  正确的动作是把发布做掉, 而不是在 aoe 里绕过去.

教训值得记一句: "外部依赖要等很久"是个假设, 不是事实.  在为它设计绕路之前, 先去确认那个依赖到底卡在哪 —— 这次卡的只是没人去发.

## 待办

- [ ] xats 侧发布 0.7.7 (含 commit `87697ed` 的启动提示 identity 分支)
- [ ] 发布后走真实路径端到端验证一次: 恢复一个 CAT 实例, 确认 agent 不需要人工注册就回到原身份
- [ ] 验证时注意第一次仍会走回落 (全库当前零绑定), 关键是看那次回落之后键有没有绑上 —— 绑上了才算收敛

## 一个相邻事实

`codex` 的 pane 目前**完全不进 aoe 的追踪** (见 `codex-pane-tracking.md`).  在那件事解决前, **aoe 无法为任何 codex pane 恢复身份** —— 因为它连那个 pane 存在过都不知道.  如果收到"codex 重启后身份没恢复"的反馈, 根因可能在 aoe 这一侧而不是 xats 那边.
