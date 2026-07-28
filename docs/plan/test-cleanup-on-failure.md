# 测试的清理必须覆盖失败路径

## 问题

在最后一行清理 tmux session 的测试, 只在它通过时才清理.  中间的每一条断言都是一个提前退出, 会把 server、它的 shell 和那个 shell 的子进程一起留下.  而由 pid 派生的 session 名, 在 pid 被复用之后还能污染后续测试.

2026-07-28 由 reviewer 在 `src/tmux/session.rs` 的新单测上发现, 从 WARNING 升为 CRITICAL —— 因为那条测试的 pane 命令是 `sh -c 'while :; do sleep 60; done'`, 泄漏的不是一个会自己结束的 `sleep 30`, 而是一个永不退出的循环.

## 做法

清理属于值的生命周期, 而不是控制流的最后一行:

```rust
/// Kills one session by its exact name when dropped.
/// 精确名字, 绝不用模式匹配, 绝不用 kill-server.
struct SessionGuard(String);

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let _ = crate::tmux::tmux_command()
            .args(["kill-session", "-t", &self.0])
            .output();
    }
}
```

建 session 成功之后立刻挂上:

```rust
assert!(created.status.success(), "new-session failed: {}", ...);
let _guard = SessionGuard(session_name.clone());
```

正常路径仍然显式清理并**校验结果**, guard 只做 unwind 时的兜底 —— 两者不是二选一: 显式清理让"清理失败"这件事本身可见, guard 让永远到不了那一行的路径也不泄漏.

## 验证方法

不要只靠"看起来对了".  在断言之后注入一个 `panic!`, 跑一次, 然后数残留进程:

```
ps -eo pid,command | grep -c "[w]hile :; do sleep 60"
```

2026-07-28 用这个方法确认 guard 生效: 注入 panic 后无限 shell 残留数为 0.

## 适用范围

- [ ] `src/tmux/session.rs` 里其余创建 tmux session 的单测, 同一模式仍在
- [ ] `tests/e2e/` 的 harness 已有 `Drop` 清理并按私有 socket 精确销毁, 不受此问题影响; 但单独建 session 的 e2e 用例 (如 `attach_reconcile.rs` 的 `Cleanup`) 值得按同一标准复核

## 相关

`AGENTS.md` "Tmux Session Safety" —— 清理只许按精确 session 名或私有 socket 绝对路径, 任何情况下不用 `kill-server`, 不按前缀批量杀.
