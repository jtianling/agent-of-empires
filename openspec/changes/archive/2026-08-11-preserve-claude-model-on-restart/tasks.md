## 1. Store: `agent_slot.model` 列

- [x] 1.1 在 `src/db/mod.rs` 的 `ensure_schema()` DDL 里给 `agent_slot` 加 `model TEXT NOT NULL DEFAULT ''`
- [x] 1.2 把 `("model", "TEXT NOT NULL DEFAULT ''")` 加进 `backfill_agent_slot_columns()` 的自愈列表
- [x] 1.3 给 `RawAgentSlot` 与 `AgentSlot` 加 `model: String` 字段, 更新读路径的列映射
- [x] 1.4 新增写入 model 的 store 方法 (`record_slot_model_probe`), 语义为: 传入空值时保留已有值, 不清空
- [x] 1.5 单元测试: 旧库补列后保留原有 row; 重复应用 schema 幂等; model 跨 store 重开读回; 空值 upsert 不清空已有 model

## 2. Transcript 探测

- [x] 2.1 新建 `src/db/claude_transcript.rs`, 从 `src/session/instance.rs` 的 `resolve_claude_session_from_disk()` 抽出或复用 project-hash 目录名推导 (绝对路径 `/` → `-`)
- [x] 2.2 实现 `detect_model(cwd, native_session_id) -> Option<String>`: 定位 `<home>/.claude/projects/<project-dir>/<session-id>.jsonl`, 从文件尾读取 ≥1 MiB 的有界窗口
- [x] 2.3 丢弃窗口内第一条可能被截断的行, 自后向前查找第一条 `type == "assistant"` 且 `isSidechain != true` 且 `message.model` 非空非 `"<synthetic>"` 的行, 返回其 `message.model`
- [x] 2.4 所有失败分支 (文件缺失 / IO 错误 / JSON 解析错误 / 无匹配行) 返回 `None` 并只记 debug 级日志, 不向上抛错
- [x] 2.5 在 `src/db/mod.rs` 挂上 `mod claude_transcript;`
- [x] 2.6 单元测试 (用 `tempfile` 造 transcript, 不碰真实 `~/.claude`): 取最后一条有效条目; 跳过 sidechain; 跳过 `<synthetic>`; 单行 > 256 KiB 仍能取到; 文件缺失返回 `None`; 尾部是坏 JSON 返回 `None`

## 3. reconcile 周期刷新

- [x] 3.1 在 `src/db/reconcile.rs` 的 slot 收敛路径里, 对 `agent == "claude"` 的 slot 调用探测, 非 claude slot 直接跳过
- [x] 3.2 加入文件指纹跳过 (路径 + mtime 秒 + 文件长度), 指纹未变化时跳过读取, 保持已有 model 不变.  指纹持久化在 `agent_slot.model_fingerprint` 而不是进程内: reconcile 由两个进程驱动 (home-view poller 与 notification monitor), 进程内缓存只对先探到该文件的那个进程生效, 另一个进程每 tick 都会重读并覆盖
- [x] 3.3 探测返回 `None` 时保留 slot 已有 model, 不写空值
- [x] 3.4 单元测试: 非 claude slot 不触发文件读取; 指纹未变化时不重读; 探测为空时旧值保留

## 4. 命令注入

- [x] 4.1 在 `Instance::build_base_pane_command()` 的 primary 与 `!is_primary` 两条分支上, 当 pane 为 claude 且有非空 model 时追加 `--model <model>`, 位置在 `extra_args` 之后
- [x] 4.2 确认调用方能把 slot 的 model 传到该函数 (必要时扩参数), 且 `Shift+R` / `Shift+C` / `r` / `c` / cold-start recovery / 多 pane fan-out / fork 全部经由这一处生效, 不新增任何按键特判
- [x] 4.3 单元测试: resume 重启命令同时含 resume flag 与 `--model`; fresh 重启含 `--model` 不含 resume flag; 非 primary claude pane 也带 `--model`; 非 claude pane 命令逐字节不变; model 为空时命令逐字节不变; `extra_args` 已有 `--model sonnet` 时探测值排在其后

## 5. E2E

- [x] 5.1 在 `tests/e2e/` 新增用例, 走 `TuiTestHarness` (私有 socket, 隔离 `$HOME`, 已清 `$TMUX` / `$TMUX_PANE`), 在隔离 HOME 下伪造 `~/.claude/projects/<hash>/<uuid>.jsonl` 与对应 `agent_slot` 行
- [x] 5.2 验证 `Shift+R` 后该 pane 的实际命令行含 `--model <伪造模型>`
- [x] 5.3 验证 `Shift+C` (fresh) 后命令行仍含同一个 `--model`, 且不含 resume flag
- [x] 5.4 用例结束清理自建 session; **禁止 `tmux kill-server`, 禁止按前缀批量杀 session**
- [x] 5.5 只跑本 change 新增的 e2e 用例 (`cargo test --test e2e -- <用例名>`), **不跑全量 `cargo test`**

## 6. 收尾

- [x] 6.1 `cargo fmt`
- [x] 6.2 `cargo clippy` 无新增 warning
- [x] 6.3 `cargo build` 编译校验通过
- [x] 6.4 检查是否有需要同步的用户文档 (本 change 不新增 CLI flag 与配置项, 预期无 `docs/cli/reference.md` 变动; 若 clap help 未变则无需重跑 `cargo xtask gen-docs`)
