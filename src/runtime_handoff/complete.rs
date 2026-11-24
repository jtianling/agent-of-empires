use std::time::{Duration, Instant};

use anyhow::{ensure, Context, Result};
use clap::Args;

use super::{
    pending::{self, Pending},
    types::{Runtime, State},
};

#[derive(Args)]
pub struct CompleteArgs {
    #[arg(long)]
    instance_id: String,
    #[arg(long)]
    slot: i64,
    #[arg(long)]
    handoff_id: String,
    #[arg(long)]
    attempt_id: String,
    #[arg(long, conflicts_with = "ui_pid")]
    thread_id: Option<String>,
    #[arg(long, conflicts_with = "thread_id")]
    ui_pid: Option<u32>,
}

pub fn run(profile: &str, args: CompleteArgs) -> Result<()> {
    crate::session::tui_instances::validate_profile(profile)?;
    uuid::Uuid::parse_str(&args.handoff_id).context("Invalid runtime handoff ID")?;
    let storage = crate::session::Storage::new(profile)?;
    let lock = storage
        .operation_lock()
        .context("Locking AoE profile for runtime handoff")?;
    let mut pending = pending::load(profile, &args.instance_id, args.slot)?
        .context("No pending runtime handoff exists")?;
    let response = pending
        .response
        .as_ref()
        .context("Runtime handoff is not prepared")?;
    ensure!(
        response.fence.handoff_id == args.handoff_id,
        "Runtime handoff ID changed"
    );
    ensure!(
        pending.attempt_id.as_deref() == Some(args.attempt_id.as_str()),
        "Runtime handoff launch attempt was replaced"
    );
    verify_slot(profile, &pending)?;
    let runtime = match pending.target_tool.as_str() {
        "codex" => Runtime::Codex {
            thread_id: args
                .thread_id
                .context("Codex handoff requires the actual thread ID")?,
            ws_url: pending
                .codex_endpoint
                .clone()
                .context("Codex handoff endpoint is missing")?,
        },
        "claude" => Runtime::Claude {
            ui_pid: args
                .ui_pid
                .context("Claude handoff requires the actual CLI PID")?,
            channel_session_id: None,
        },
        _ => anyhow::bail!("This helper accepts Codex or Claude runtime coordinates"),
    };
    drop(lock);
    super::resolve::commit_with_recovery(profile, &mut pending, runtime)?;
    println!("Runtime handoff committed");
    Ok(())
}

fn verify_slot(profile: &str, pending: &Pending) -> Result<()> {
    let slot = crate::db::Store::open_with_schema(profile)?
        .read_slots_for_instance(&pending.instance_id)?
        .into_iter()
        .find(|slot| slot.slot == pending.slot)
        .context("Runtime handoff pane disappeared")?;
    ensure!(
        slot.agent == pending.target_tool && slot.xats_identity_key == pending.identity_key,
        "Runtime handoff pane was replaced"
    );
    Ok(())
}

pub(crate) fn commit_opencode(
    profile: &str,
    id: &str,
    slot: i64,
    base_url: &str,
    session_id: &str,
) -> Result<Option<(i64, String, String)>> {
    let storage = crate::session::Storage::new(profile)?;
    let lock = storage.operation_lock()?;
    let Some(mut pending) = pending::load(profile, id, slot)? else {
        return Ok(None);
    };
    if pending.target_tool != "opencode" {
        return Ok(None);
    }
    if pending.response.as_ref().map(|r| r.state) == Some(State::Committed) {
        return Ok(None);
    }
    verify_slot(profile, &pending)?;
    drop(lock);
    let response = super::resolve::commit_with_recovery(
        profile,
        &mut pending,
        Runtime::OpenCode {
            base_url: base_url.into(),
            session_id: session_id.into(),
        },
    )?;
    let generation = response
        .current
        .and_then(|current| current.runtime_generation)
        .context("Committed OpenCode handoff omitted runtime generation")?;
    ensure!(
        generation > 0,
        "Committed OpenCode handoff returned an invalid generation"
    );
    Ok(Some((generation, pending.name, pending.team)))
}

pub(crate) fn bootstrap_prompt(profile: &str, pending: &Pending) -> Result<String> {
    use crate::session::environment::shell_escape;
    let executable = std::env::current_exe().context("Finding AoE handoff helper")?;
    let executable = executable
        .to_str()
        .context("AoE handoff helper path is not UTF-8")?;
    let fence = &pending
        .response
        .as_ref()
        .context("Missing handoff fence")?
        .fence;
    let attempt = pending
        .attempt_id
        .as_deref()
        .context("Missing handoff launch attempt")?;
    let coordinate = match pending.target_tool.as_str() {
        "codex" => "--thread-id \"$CODEX_THREAD_ID\"",
        "claude" => "--ui-pid \"$PPID\"",
        _ => anyhow::bail!("Unsupported handoff bootstrap agent"),
    };
    let command = format!(
        "{} -p {} __complete-runtime-handoff --instance-id {} --slot {} \
        --handoff-id {} --attempt-id {} {coordinate}",
        shell_escape(executable),
        shell_escape(profile),
        shell_escape(&pending.instance_id),
        pending.slot,
        shell_escape(&fence.handoff_id),
        shell_escape(attempt)
    );
    Ok(format!(
        "AoE 正在完成当前 pane 的 xats agent 类型交接. 请先通过 shell 工具执行 {command}. \
        使用当前会话的真实坐标, 不读取或输出凭据文件. 命令成功后, 调用 register_agent 恢复 \
        name={}, team={}, agent_type={}, 并提供当前会话的实际 runtime 参数. \
        若 shell 沙箱报告 Operation not permitted, 请通过 shell 工具的权限审批机制请求在沙箱外重试同一命令. \
        若审批被拒绝或重试仍失败, 说明具体错误并停止注册.",
        pending.name,
        pending.team,
        super::types::agent_type(&pending.target_tool)?
    ))
}

pub(crate) fn wait_committed(profile: &str, id: &str, slot: i64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        {
            let storage = crate::session::Storage::new(profile)?;
            let _lock = storage.operation_lock()?;
            let Some(mut pending) = pending::load(profile, id, slot)? else {
                return Ok(());
            };
            if pending.response.as_ref().map(|r| r.state) == Some(State::Committed) {
                return Ok(());
            }
            let response = super::refresh(profile, &mut pending)?;
            if response.state == State::Committed {
                return Ok(());
            }
        }
        ensure!(
            Instant::now() < deadline,
            "Runtime handoff is pending; the new agent must complete its bootstrap before resume"
        );
        std::thread::sleep(Duration::from_millis(500));
    }
}
