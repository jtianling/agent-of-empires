2026-10-30, 用户要求检查并恢复 30 号 remindctl session.  按当前 manual 排序确认完整 ID 为 `e75c2114ac47433c`, profile 为 `auto-coworkspace-bedc`, 原 pane 为 `%82`、`%83`.

两个旧客户端均显示持续约 28 小时的 reconnect 失败.  AoE 的两个 native_session_id 为空, codex_binding 也为空.  xats 预登记行有 launch ID, 但 agent_id 和 thread_id 均为空; 原注册调用未传 recovery_nonce, 注册记录也没有正确对应当前 pane.  注册时间在预登记有效期内, 因此不能将本次直接等同于 16 号的迟到注册超时案例.

原始两条 thread 分别为 `01a185b0-8e54-7831-89f8-48557e153aa8` 和 `01a185b0-8e7f-7650-b4d0-9dd27942674d`.  只读 thread/read 确认二者存在、目录准确且为 notLoaded, 再按用户明确授权恢复.  诊断早期的临时脚本仅匹配小写 UUID, 一度误判没有 launch ID, 已核对大写 UUID 并向用户更正; 生产恢复入口的 UUID 解析未受此脚本问题影响.

恢复前完成一致性 SQLite、session/group 记录及两个 pane 的备份, 私有目录为 `target/recovery-backups/session-e75c2114ac47433c-before-resume-20261030T084844Z/`.  主 pane 有未发送草稿, 单独备份后再操作.  使用正式 `session recover-codex` 为两个准确槽位补齐原 UUID, 随后以正式 `session restart` 走逐 pane Resume, 没有手改持久绑定或重建对话.

恢复后两个原 UUID 均为 idle, 新客户端参数明确 resume 对应原 UUID, 原 `%82`、`%83` 和布局保留, 槽位代次为 1.  两个界面均无 reconnect 错误, 主 pane 草稿已原样放回输入框且未按 Enter.  共享 Codex、xats、tmux server 和其他 session 未重启.  最终恢复记录及屏幕证据在上述私有备份目录.

当前 codex_binding 为 awaiting_launch, 保留已验证的原 UUID.  不将 CLI resume 成功或 thread/read idle 宣称为新 launch 的 xats 自动登记已经完成.  后续恢复可依据持久原绑定及准确 resume 参数进行校验, 不按 cwd 或最新历史猜选对话.
