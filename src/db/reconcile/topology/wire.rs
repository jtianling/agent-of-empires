use anyhow::{ensure, Context, Result};
use std::str;

use super::Server;

// Escape delimiters before tmux applies its output quoting. An expanded control
// byte can then invalidate only its field, never the next pane's framing.
pub(super) const FORMAT: &str = concat!(
    "#{pid} #{start_time} #{server_sessions} #{session_id} #{session_windows} ",
    "#{window_id} #{window_active} #{window_panes} #{window_layout} ",
    "#{==:#{@aoe_agent_pane},#{pane_id}} #{pane_index} #{pane_id} #{pane_pid} ",
    "#{n:session_name} #{n:pane_current_path}|",
    "#{s/%/%25/;s/[|]/%7C/;s/\n/%0A/:session_name}|",
    "#{s/%/%25/;s/[|]/%7C/;s/\n/%0A/:pane_current_path}"
);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Text<'a> {
    wire: &'a [u8],
    len: usize,
}

impl Text<'_> {
    pub fn decode(self) -> Option<String> {
        let mut bytes = Vec::new();
        let mut input = self.wire;
        while let Some((&next, rest)) = input.split_first() {
            if next == b'%' {
                bytes.push(match rest.get(..2)? {
                    b"25" => b'%',
                    b"7C" => b'|',
                    b"0A" => b'\n',
                    _ => return None,
                });
                input = &rest[2..];
            } else {
                bytes.push(next);
                input = rest;
            }
        }
        if bytes.len() != self.len {
            return None;
        }
        String::from_utf8(bytes).ok()
    }
}

#[derive(Clone, Copy)]
pub(super) struct Row<'a> {
    pub server: Server,
    pub session_count: usize,
    pub sid: u32,
    pub window_count: usize,
    pub wid: u32,
    pub active: bool,
    pub pane_count: usize,
    pub layout: &'a str,
    pub primary: bool,
    pub index: u32,
    pub id: &'a str,
    pub pid: u32,
    pub name: Text<'a>,
    pub cwd: Text<'a>,
}

impl Row<'_> {
    pub fn coherent_with(&self, other: &Self) -> bool {
        self.server == other.server
            && self.name == other.name
            && self.pane_count == other.pane_count
            && self.layout == other.layout
            && self.active == other.active
    }
}

pub(super) fn row(line: &[u8]) -> Result<Row<'_>> {
    let mut parts = line.split(|b| *b == b'|');
    let header = parts.next().context("Missing tmux header")?;
    let name = parts.next().context("Missing tmux session name")?;
    let cwd = parts.next().context("Missing tmux cwd")?;
    ensure!(parts.next().is_none(), "Invalid tmux field delimiter");
    let fields = str::from_utf8(header)?.split(' ').collect::<Vec<_>>();
    let [server, started, sessions, sid, windows, wid, active, panes, layout, primary, index, id, pid, name_len, cwd_len] =
        fields.as_slice()
    else {
        anyhow::bail!("Invalid inventory field count")
    };
    let server = Server {
        pid: server.parse()?,
        started: started.parse()?,
    };
    let pid = if pid.is_empty() {
        0
    } else {
        pid.parse::<u32>()?
    };
    ensure!(
        server.pid > 1 && server.started > 0 && pid != 1 && pid <= i32::MAX as u32,
        "Invalid tmux process identity"
    );
    let ident = |text: &str, prefix| -> Result<u32> {
        Ok(text
            .strip_prefix(prefix)
            .context("Invalid tmux identifier")?
            .parse()?)
    };
    ident(id, '%')?;
    let flag = |s: &str| match s {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => anyhow::bail!("Invalid tmux flag"),
    };
    Ok(Row {
        server,
        session_count: sessions.parse()?,
        sid: ident(sid, '$')?,
        window_count: windows.parse()?,
        wid: ident(wid, '@')?,
        active: flag(active)?,
        pane_count: panes.parse()?,
        layout,
        primary: flag(primary)?,
        index: index.parse()?,
        id,
        pid,
        name: Text {
            wire: name,
            len: name_len.parse()?,
        },
        cwd: Text {
            wire: cwd,
            len: cwd_len.parse()?,
        },
    })
}
