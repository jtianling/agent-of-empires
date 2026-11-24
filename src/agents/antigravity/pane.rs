use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};

pub(crate) fn is_launch_descendant() -> Result<bool> {
    let Some(launched) = std::env::var_os("AOE_ANTIGRAVITY_PID") else {
        return Ok(false);
    };
    let pid: u32 = launched
        .to_str()
        .context("Invalid Antigravity launch PID")?
        .parse()?;
    ensure!(pid > 1, "Invalid Antigravity launch PID");
    let child = expected_child(
        std::env::var_os("AOE_ANTIGRAVITY_NAMED").as_deref(),
        std::env::var_os("XATS_AGY_PID").as_deref(),
        pid,
    )?;
    let output = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,comm="])
        .output()
        .context("Reading Antigravity process ancestry")?;
    ensure!(
        output.status.success(),
        "Reading process ancestry: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(owns_launch(
        &String::from_utf8_lossy(&output.stdout),
        crate::process::parent_id(),
        pid,
        child,
    ))
}

fn expected_child(
    named: Option<&std::ffi::OsStr>,
    child: Option<&std::ffi::OsStr>,
    wrapper: u32,
) -> Result<Option<u32>> {
    match named {
        None => Ok(None),
        Some(flag) if flag == "0" => Ok(None),
        Some(flag) if flag == "1" => {
            let child = child
                .and_then(|p| p.to_str())
                .context("Missing or invalid XATS_AGY_PID")?;
            let pid = child.parse::<u32>().context("Invalid XATS_AGY_PID")?;
            ensure!(
                pid > 1 && pid != wrapper && child == pid.to_string(),
                "Invalid Antigravity child PID"
            );
            Ok(Some(pid))
        }
        _ => anyhow::bail!("Invalid Antigravity named launch marker"),
    }
}

fn owns_launch(list: &str, from: u32, launched: u32, child: Option<u32>) -> bool {
    let Some((agy, parent)) = nearest_agy(list, from) else {
        return false;
    };
    match child {
        Some(child) => agy == child && parent == launched,
        None => agy == launched,
    }
}

fn nearest_agy(list: &str, mut pid: u32) -> Option<(u32, u32)> {
    let processes: HashMap<u32, (u32, &str)> = list
        .lines()
        .filter_map(|line| {
            let (id, rest) = line.trim_start().split_once(char::is_whitespace)?;
            let (parent, comm) = rest.trim_start().split_once(char::is_whitespace)?;
            Some((id.parse().ok()?, (parent.parse().ok()?, comm.trim())))
        })
        .collect();
    for _ in 0..64 {
        let &(parent, name) = processes.get(&pid)?;
        if matches!(
            Path::new(name).file_name()?.to_str(),
            Some("agy" | "antigravity")
        ) {
            return Some((pid, parent));
        }
        pid = parent;
    }
    None
}

pub(crate) fn bridge_key() -> Result<Option<String>> {
    if std::env::var("AOE_ANTIGRAVITY_XATS").as_deref() != Ok("1") || !is_launch_descendant()? {
        return Ok(None);
    }
    slot_key(false).map(Some)
}

pub(crate) fn launcher_key() -> Result<String> {
    ensure!(
        [
            "AOE_ANTIGRAVITY_XATS",
            "AOE_ANTIGRAVITY_NAMED",
            "AOE_ANTIGRAVITY_DEFERRED"
        ]
        .iter()
        .all(|name| std::env::var(name).as_deref() == Ok("1"))
            && std::env::var("AOE_ANTIGRAVITY_PID")? == std::process::id().to_string(),
        "Cannot verify deferred Antigravity launch process"
    );
    slot_key(true)
}

fn slot_key(require_pane_root: bool) -> Result<String> {
    let profile = std::env::var("AGENT_OF_EMPIRES_PROFILE")?;
    let instance = std::env::var("AOE_INSTANCE_ID")?;
    let pane = std::env::var("TMUX_PANE")?;
    ensure!(
        pane.strip_prefix('%')
            .is_some_and(|s| s.parse::<u32>().is_ok()),
        "Invalid Antigravity pane"
    );
    crate::tmux::init_tmux_socket_name(Some(crate::tmux::socket_name_for_profile(&profile)));
    if require_pane_root {
        ensure!(
            crate::process::get_pane_pid(&pane) == Some(std::process::id()),
            "Antigravity launcher is not the root process of pane {pane}"
        );
    } else {
        ensure!(
            crate::cli::record_pane::pane_hosts_this_process(&pane) == Some(true),
            "Cannot verify Antigravity pane ownership"
        );
    }
    let digest = if std::env::var("AOE_ANTIGRAVITY_DEFERRED").as_deref() == Ok("1") {
        let value = std::env::var("AOE_ANTIGRAVITY_SLOT_DIGEST")?;
        ensure!(
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Invalid Antigravity slot fingerprint"
        );
        Some(value)
    } else {
        None
    };
    let store = crate::db::Store::open(&profile)?;
    // A fresh pane can initialize its MCP servers before AoE records its slot.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let slots = store.read_slots_for_instance(&instance)?;
        let mut matches = slots.iter().filter(|s| {
            s.tmux_pane == pane
                && digest.as_ref().map_or(true, |expected| {
                    format!("{:x}", Sha256::digest(s.xats_identity_key.as_bytes())) == *expected
                })
        });
        if let Some(slot) = matches.next() {
            ensure!(
                matches.next().is_none() && slot.agent == "antigravity" && slot.cross_agent_team,
                "Antigravity MCP pane has no unique xats slot"
            );
            ensure!(
                crate::hooks::antigravity::valid_id(&slot.xats_identity_key),
                "Invalid Antigravity xats identity key"
            );
            return Ok(slot.xats_identity_key.clone());
        }
        ensure!(
            Instant::now() < deadline,
            "Antigravity MCP pane has no bound slot"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_antigravity_process_distinguishes_nested_clients() {
        let listing =
            " 12   1 /opt/antigravity\n 13  12 helper\n 14  13 /my bin/agy\n 15  14 helper";
        assert_eq!(nearest_agy(listing, 13), Some((12, 1)));
        assert_eq!(nearest_agy(listing, 15), Some((14, 13)));
        assert_eq!(nearest_agy(listing, 1), None);
        assert!(owns_launch(listing, 13, 12, None));
        assert!(!owns_launch(listing, 15, 12, None));
    }

    #[test]
    fn plain_and_legacy_markers_keep_the_direct_exec_contract() {
        use std::ffi::OsStr;
        let child = Some(OsStr::new("22"));
        for marker in [None, Some(OsStr::new("0"))] {
            assert_eq!(expected_child(marker, child, 21).unwrap(), None);
        }
        let named = Some(OsStr::new("1"));
        assert_eq!(expected_child(named, child, 21).unwrap(), Some(22));
        for invalid in ["0", "1", "21", "022", "invalid"] {
            assert!(expected_child(named, Some(OsStr::new(invalid)), 21).is_err());
        }
        assert!(expected_child(named, None, 21).is_err());
        assert!(expected_child(Some(OsStr::new("bad")), child, 21).is_err());
    }

    #[test]
    fn named_launch_requires_the_direct_child_and_nearest_agy() {
        let ps = "21 1 python3\n22 21 /bin/agy\n23 22 helper\n24 23 agy\n25 24 hook";
        assert!(owns_launch(ps, 23, 21, Some(22)));
        assert!(!owns_launch(ps, 23, 21, None));
        assert!(!owns_launch(ps, 25, 21, Some(22)));
        assert!(!owns_launch(ps, 25, 21, Some(24)));
        assert!(!owns_launch(ps, 23, 1, Some(22)));
        assert!(!owns_launch(ps, 23, 21, Some(24)));
    }
}
