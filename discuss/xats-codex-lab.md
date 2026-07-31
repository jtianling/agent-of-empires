# xats codex 身份恢复: 隔离实验室规格 (草案)

目的: 把 codex↔xats 的 pane 认领 / 身份恢复问题, 从"在 jt 的实时环境里人工陪测"搬进一个与生产完全隔绝、可脚本化重放的实验室.  连测已烧掉多轮人工配合, 且每轮现场都被真实环境的意外污染 (托管 session 被重建导致 pane 号整体漂移、pre-reg 行到期、多 codex 交错注册), 结论难以复现.

## 隔离的四条边界

生产侧当前值仅作对照, 实验室必须全部不同:

| 维度 | 生产 | 实验室 |
| --- | --- | --- |
| daemon 数据目录 | `~/.cross-agent-teams-mcp/` | `$LAB/xats-home/` (`CROSS_AGENT_TEAMS_MCP_HOME`) |
| daemon 端口 | 9100 | 9199 |
| daemon token | `xats` | 实验室专用值 |
| device 标签 | `jt` | `jtlab` |
| tmux socket | 默认 / aoe 托管 socket | `TMUX_TMPDIR=$LAB/tmuxtmp` 解析出的 `$LAB/tmuxtmp/tmux-$(id -u)/default` (`-S` 照传该绝对路径) |
| daemon 侧 tmux 连接 | 继承环境 | `TMUX_TMPDIR=$LAB/tmuxtmp` **且清空 `$TMUX`** |
| codex thread resume 端点 | 生产 app-server `ws://127.0.0.1:8799` | `CROSS_AGENT_TEAMS_CODEX_WS_URL` 指向实验室端口 (8899) |

后两条是实跑才暴露的, 纸面推导不出来:

- **daemon 内部裸调 `tmux`** (源码里没有 `-S` 可传), 所以只给客户端加 `-S` 不够: daemon 那一侧会连共享 server 去 probe / paste **真实 pane**.  必须用 `TMUX_TMPDIR` + 清空 `$TMUX` 才能把 daemon 也关进实验室.
- **codex 注册会触发 daemon 去 resume thread**, 端点默认就是生产 app-server.  不改 `CROSS_AGENT_TEAMS_CODEX_WS_URL` 的话, 每次实验室 codex 注册都在悄悄摸生产.

启动形态:

```sh
CROSS_AGENT_TEAMS_MCP_HOME=$LAB/xats-home \
  node <xats-repo>/dist/cli.js daemon --port 9199 --token <lab-token> --device jtlab
```

`pre-register-codex-pane` 支持 `--port` / `--token`, 因此实验室里所有 CLI 调用都显式带上, 不依赖 pid 文件解析, 杜绝误连生产 daemon.

### tmux 硬约束 (违反会杀掉 jt 的实时 session)

- 私有 server 的口径由 `lab/lab-env.sh` 定, 两侧必须共用:

  ```
  TMUX_TMPDIR=$LAB/tmuxtmp
  socket   = $LAB/tmuxtmp/tmux-$(id -u)/default
  调用     = env -u TMUX -u TMUX_PANE TMUX_TMPDIR=$LAB/tmuxtmp tmux -S <上面那个绝对路径> ...
  ```

  `-S` 照传 (清理红线要求按绝对路径), 但**路径必须是 `TMUX_TMPDIR` 解析出来的那一个**.
- **不要用 `$LAB/tmux.sock`** (本文档早先的写法, 2026-07-31 实测是错的).  lab daemon 内部**裸调 `tmux`** (源码里没有 `-S` 可传), 只能靠 `TMUX_TMPDIR` 找 server; pane 建在 `tmux.sock` 上时 daemon 完全看不见它们, 于是所有"绑没绑对 pane"的断言会因为**候选集为空**而通过 —— 假绿, 而且是最难发现的那种.  该 socket 文件还作为残留存在, 所以写错不会报错, 只会安静地建到另一台 server 上.
- 每条 tmux 命令都要 `env -u TMUX -u TMUX_PANE`.  只设 `TMUX_TMPDIR` **不算隔离**: 进程里 `$TMUX` 有值时 tmux 客户端直连当前 server 并无视 `TMUX_TMPDIR`.
- 清理只允许对上面那个绝对 socket 路径 `kill-server`, 或按精确名字 `kill-session`.  **禁止裸 `tmux kill-server`, 禁止按前缀批量杀 session.**
- 动手前自检 `env | grep TMUX`, `$TMUX` 还在就不要执行任何破坏性 tmux 命令.

## 被测对象: 用真实 codex, tmux 驱动 (jt 定调)

实验室里跑**真实的 codex**, 由 tmux 驱动 (send-keys 下指令, capture-pane 断言), 而不是拿替身糊弄.  理由: 今天两次事故的根因都在"真实进程形态 + 真实注册时序"里 (npm shim 的 wrapper+child、app-server 共享导致调用者报不出自己的 pid、注册与 pre-reg 过期的竞态), 替身最容易恰好绕开这些.

要让真实 codex 完全落在实验室内, 除了前述四条边界, 还要隔离 codex 自身:

| 维度 | 生产 | 实验室 |
| --- | --- | --- |
| codex 配置/会话目录 | `~/.codex/` | `$LAB/codex-home/` (`CODEX_HOME`) |
| codex 的 xats MCP 指向 | 生产 daemon (9100) | 实验室 daemon (9199, 实验室 token) |
| app-server | aoe 共享 app-server (8799) | 实验室专用 app-server (8899), **必须保留 `--remote` 指向它** |

要点:

- `CODEX_HOME` 隔离后, 凭证需要单独放一份 (`cp ~/.codex/auth.json $LAB/codex-home/`), 不要读文件内容, 只做整文件复制.
- 实验室 `config.toml` 必须预置目录信任, 否则真实 codex 首启会停在 "Do you trust the contents of this directory?" 等按键, **`--dangerously-bypass-approvals-and-sandbox` 不跳过这一步** (codex 0.146.0 实测).  写法照抄生产 config.toml 里已有的 projects 段:

  ```toml
  [projects."/tmp/xats-lab"]
  trust_level = "trusted"
  ```

- **被守卫扫描的文件里不能出现被禁的字面量**.  实测踩过: 生成的 config.toml 注释里写了 `9100` 字样, 被自己"grep 9100 即拒绝"的守卫命中, 守卫恒失败.  两侧的 config 生成器都要注意.
- **先覆盖再断言**: 实验室脚本往往从生产 shell 启动, 天然继承 `CROSS_AGENT_TEAMS_MCP_TOKEN` 一类生产凭证 (实测第一次跑就被自己的守卫拦下).  入口处先把 `CODEX_HOME` / token / `CROSS_AGENT_TEAMS_MCP_HOME` 覆盖成实验室值, 断言只看覆盖后的生效值, 继承情况作为诊断行输出.  显式传进来的生产值仍须硬失败.  这与 tmux 夹具开头 `unset TMUX` 是同一套路.
- **`$TMUX_PANE` 也是继承污染的一员**: 实测中脚本观测到的 pane 是 `%2` —— 调用方会话继承来的**生产** pane id.  真跑起来会把生产 pane 号 pre-register 进实验室 DB, 等于把实验室身份绑到 jt 的真实 pane 上.  规则: 只有当 `$TMUX` 确实指向实验室 server 时才允许继承 `$TMUX_PANE`, 否则必须显式传 `--pane`.
- **实验室内两侧夹具必须共用同一个 tmux server**: 因为 daemon 裸调 `tmux`、只能靠 `TMUX_TMPDIR` 解析找 server, 任何一侧另起自己的 socket, 起出来的 pane 对 daemon 都不可见, pane 认领类断言会全部空转.  共用之后 teardown 必须收敛: 默认只按精确名字 kill 自己那个 session, 整服务器收摊要显式开关, 否则会打断另一侧正在跑的场景.
- 实验室 `config.toml` 里的 xats MCP server 必须指向 9199 与实验室 token; 配错就会把实验室流量打进生产 daemon, 这是最需要防的一类误连.
- 启动时照抄生产形态: `-c "xats.agent_id=\"<uuid>\""`, 使 pane argv 与真实环境一致.
- **`--remote` 不能去掉** (2026-07-31 实测推翻了本文档早先的写法): daemon 识别 codex 载体进程的正则 (`src/mcp/auto-bind-codex-pane.ts`) 硬性要求 argv 里出现 `--remote`, 去掉之后真身进程对 daemon **完全不可见**, 注册能成功但 pane 一定绑不上 (`auto-bind skip reason=no_match matches=0`).  实验室保留 `--remote` 但指向实验室 app-server 8899.
  这条之所以拖到真身跑才暴露, 是因为**替身 argv 里照抄了 `--remote`, 反而一直能被识别** —— 替身与真身的形态差异恰好掩盖了它.  这正是"实验室必须跑真实 codex"的一次实证.
- **`exec` 真实 codex 时必须把 `CODEX_HOME` 带上那一行**, 否则 codex 会读写生产 `~/.codex` (auth / sessions / rollout).  实测踩过.
- 真实 codex 会消耗模型额度, 所以每个场景只让它做最小动作 (通常就是"注册一次"或"照恢复话术重注册"), 不做开放式任务.

**替身仅作可选快路径**: 纯绑定/认领类断言 (S1/S1b) 如果需要高频重放, 可以用 argv 带 `xats.agent_id="<uuid>"` 的 idle `node` 进程冒充 codex 占住 pane; 但每个场景至少要有一次真实 codex 的确认跑, 替身结论不单独作数.

替身形态必须照抄以下三条实测事实 (2026-07-31 tester 从现网抓取, 不是推测):

1. **wrapper+child 双进程是默认形态, 不是可选**.  真实一对: 父进程 `node ~/.nvm/.../bin/codex <flags>` (comm=`node`), 子进程 `.../vendor/aarch64-apple-darwin/bin/codex <完全相同的 flags>` (comm=`codex`), 两者 argv 带同一个 agent_id.  即 argv 子串匹配天然命中 2 个进程, 且两个进程的 comm 一个是 `node` 一个是 `codex` —— 靠进程名识别会同时踩两个坑.
2. **agent_id 是大写 UUID** (`uuidgen` 原样输出).  替身不要转小写, 否则会掩盖 daemon 侧大小写敏感比较的潜在 bug.
3. `node -e '<idle>'` 形态**放不下** `-c` 标记 (node 把 `-c` 当 `--check`, 与 `--eval` 冲突直接退出).  替身应写成脚本文件形态 `node $LAB/bin/codex <flags>`, argv 骨架反而与真实 npm shim 同构.

## 必须覆盖的场景

编号沿用连测口径.  每个场景的断言都落在: 实验室 DB (只读 sqlite3) + 实验室 daemon 日志.

- **S1 抢占 (2026-07-31 复发的形态)**: A 自己的 pre-reg 行已过期 / 不存在, B 的行有效.  A 注册 → **不得**认领 B 的行, 不得绑 B 的 pane; B 随后注册应正常拿到自己的 key.  这是当前 daemon 缺陷的最小复现.
- **S1b 无 key 抢占**: 同 S1, 但 A 自己不持 key (存量未播种形态).  此时 key 矛盾证据不存在, 检验归属校验是否仍然拦得住.
- **S2 播种**: 全新 key 无人持有 → 注册后 key 干净附到调用者行, 绑自己的 pane/pid.
- **S3 改名跟随**: 同 thread 改名 X→Y → key 迁到 Y, X 行置空; 重启 + 带 key pre-reg → 恢复话术指名 Y.
- **S4 恢复延迟**: 记录 pre-reg 落库 → poke 落地的分段耗时, 断言不进 30/180/600 重试梯子.
- **S5 session 重建 / pane id 复用**: 座位 key 按 pane id 索引 (由 `~/.xats.sh` 的 `_xats-codex-pane-key` 写在 `~/.config/xats/codex-pane-keys/<pane>`, 不在任何仓库里).  2026-07-31 实验室对照实验测出 tmux 的确切语义:

  | 情形 | server 上是否还有 pane | 新 pane id |
  | --- | --- | --- |
  | 杀 session, 另有 session 存活 | 有 | 递增, 不复用 |
  | 杀 session, server 清空 | 无 | **从 %0 重新开始 (复用)** |
  | server 重启后重建 | 无 | **从 %0 重新开始 (复用)** |

  即 pane id 计数器在**最后一个 pane 消失时重置**.  所以日常 session 重建只造成孤儿 key (恢复能力静默失效); 真正危险的窗口是 **tmux server 被清空或重启** —— 此后新 pane 又叫 `%0`, 会**捡到前一代同号 pane 遗留的座位 key**, 即身份被错误继承.  生产目前就躺着一个 `%68` 孤儿 key.
- **S6 pre-reg 行有效性 (aoe 侧缓解的回归)**: daemon 侧承认存在一个无解残余 —— 双方都无 key 且行内 key 无 holder 时, daemon 手上没有任何证据可判归属.  缓解责任在 aoe: 保证托管 pane 在其 codex 注册前 pre-reg 行始终有效.  本场景断言这条缓解成立 (含 ttl 耗尽与重启竞态).

## 归属划分 (2026-07-31 与 xats-main 商定)

- **xats 侧出题**: 断言落在 daemon 行为的部分 —— 候选资格与认领拒绝、key 附着四分支、座位绑定与条件写、恢复 poke 的调度与话术、seat-follow 迁移.  即 S1/S1b/S2/S3/S4 的判据.
- **aoe 侧出题**: 断言落在 launcher/bootstrap 形态的部分 —— pre-reg 调用的忠实复刻 (含 `--identity-key-env` 与降级路径)、座位 key 目录语义与生命周期、托管 pane 启动形态、S5、S6.
- **夹具与执行**: 实验室基础设施在 xats 仓库; 谁写的场景谁维护脚本; 执行统一交 tester.  跨侧场景 (如 S3) 由 xats 出 daemon 判据、aoe 出 launcher 动作, 拼装归 tester.
- **生产收官**: 只保留最后一次真实 codex 验证, 两侧共同确认, 不再人工陪测排查.

## aoe bootstrap 的真实调用形态 (实验室复刻这个, 不要凭印象重写)

出处 `src/session/instance.rs::codex_xats_bootstrap_command`.  托管 pane 里实际执行的是一段 `sh` 脚本:

1. 前置检查: `TMUX_PANE` 非空; `uuidgen` / `nc` / `npx` 均可用; app-server 端口可连.
2. `xats_agent_id="$(uuidgen)"`, 两道格式校验 (8-4-4-4-12 形状 + 仅十六进制与连字符).
3. pre-register (`pre_register_failed=;` 显式初始化, 防环境变量污染):
   - `XATS_IDENTITY_KEY` 非空: `npx --no-install <pkg> pre-register-codex-pane --pane "$TMUX_PANE" --agent-id "$xats_agent_id" --identity-key-env XATS_IDENTITY_KEY --ttl 600 || pre_register_failed=1`
   - 为空: 同一条命令去掉 `--identity-key-env`, 保留 `--ttl 600`.
   - 任一失败 → 再打一次**完全不带新参数**的裸 pre-reg 作为老版本降级; 再失败才 `exit 1`.
4. `exec codex --remote <app-server-url> -C <project> -c "xats.agent_id=\"$xats_agent_id\"" <用户附加参数>`.

复刻注意点:

- **key 只走环境变量**: `--identity-key-env` 传的是变量名, key 值绝不出现在任何 argv 上 (ps 全机可见).  实验室脚本必须同样守这条, 否则测出来的安全属性是假的.
- TTL 600 秒.  今天事故里"caller 自己的行已过期"正是这个窗口耗尽的结果, 场景脚本要能主动制造过期 (等待或改用更短 ttl).
- 生产走 `npx --no-install <pkg>`, 命中 npm 上的 0.7.7 (静默忽略 `--identity-key-env`).  实验室应指向仓库 `dist/cli.js` 测真代码, 但要**单独留一个场景**验证"CLI 不认识该参数"时的降级路径仍正确.

## 分工建议

- **xats 侧 (xats-main)**: 实验室 daemon fixture (隔离 HOME/端口/token/device)、实验室 codex `config.toml` 里 MCP 指向的正确写法、daemon 侧断言与日志口径.  实验室代码建议落在 xats 仓库, 因为被测对象是 daemon.
- **aoe 侧 (本文档作者)**: 上一节调用形态的忠实复刻与核对、座位 key 目录语义、aoe 托管 pane 的启动形态.
- **tester**: 搭 tmux 私有 socket 夹具与 `CODEX_HOME` 隔离, 用 tmux 驱动真实 codex, 把两侧拼成可一键重放的场景脚本, 跑 S1-S5 并出报告.

## 贯穿全天的一条教训: "唯一性"假设

2026-07-30 到 07-31 踩的坑几乎是同一形状 —— 某处代码假定"全机唯一", 而真实环境里从来不唯一:

| 假设 | 真实情况 | 后果 |
| --- | --- | --- |
| 全机唯一的未过期 pre-reg 行就是我的 | 别的 codex 的行也有效 | 抢走别人的座位与 key |
| argv 匹配到的唯一进程就是 codex | npm shim 是 wrapper+child 两个 | 歧义守卫恒失败, 从未真正绑定 |
| 全机唯一的 tmux server | 共享 socket / 实验室 socket / 临时 tmux 并存 | 清理逻辑删掉活 server 的 key |
| 座位 key 目录名唯一标识一个 server | pane id 计数器会重置; slug 化不是单射 | 身份被错误继承 / 互删 |

落地规则 (tester 版本, 比初版更完整): 凡是"必然属于已死的 X"这类推断, 落地前先问 **"X 在真实环境里唯一吗"**; 如果唯一性来自某个可变换的表示 (例如把路径塌成 slug), 还要再问一次 **"这个表示是单射的吗"**.

## 设计场景前必须知道的两条 daemon 事实

1. **`detect fallback` 的"定位"是打分猜测, 不是证据**.  它调 `detectTmuxPane({ agent })` 时不带 tty / cwd / title 任何过滤, 打分退化成 `匹配进程数 × 10 + (该 pane 是 active 则 +3) + 命令名提示分`, 并列即 ambiguous.  两个 codex pane 各有一个载体时, 进程数与命令名都相同, **唯一的区分项就是那 +3 —— 即"用户此刻正在看哪个 pane"**.  任何基于"fallback 已经定位出 caller"的设计都是错的.
2. **`identity_key_contradiction` 只在首次注册窗口失效**.  它读的是 caller **数据库行上已存的** key, 而注册 upsert 用 `COALESCE(excluded.identity_key, identity_key)`, 不传 key 的注册不会洗掉已有的 key (代码与运行时双路验证过).  所以复现最弱路径**必须用全新名字** —— 复用一个已被附过 key 的名字会命中 contradiction 分支, 走完全不同的代码路径, 场景看起来还在跑但测的已经不是同一件事.

## 与生产的关系

实验室结论只在实验室内成立即可; 生产侧的最终验收仍需一次真实 codex 的收官验证, 但**不再靠人工陪测定位问题**.  修复迭代全部在实验室里完成.
