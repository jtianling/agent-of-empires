use std::io::{self, BufRead, Read};
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitStatus, Output};

const MAX_RESPONSE: usize = 8 * 1024 * 1024;

fn invalid(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Id {
    time: u64,
    number: u32,
    flags: u8,
}

fn id(line: &[u8], prefix: &[u8]) -> Option<Id> {
    let fields = line.strip_prefix(prefix)?.strip_suffix(b"\n")?;
    let mut fields = std::str::from_utf8(fields).ok()?.split(' ');
    let value = Id {
        time: fields.next()?.parse().ok()?,
        number: fields.next()?.parse().ok()?,
        flags: fields.next()?.parse().ok()?,
    };
    fields.next().is_none().then_some(value)
}

fn end(line: &[u8], frame: Id) -> Option<bool> {
    if id(line, b"%end ") == Some(frame) {
        Some(true)
    } else if id(line, b"%error ") == Some(frame) {
        Some(false)
    } else {
        None
    }
}

fn notification(line: &[u8]) -> io::Result<()> {
    if line == b"%exit\n" || line.starts_with(b"%exit ") {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            String::from_utf8_lossy(line).trim_end().to_owned(),
        ));
    }
    let name = line
        .split(|&b| b == b' ' || b == b'\n')
        .next()
        .unwrap_or_default();
    if !name.starts_with(b"%") || matches!(name, b"%begin" | b"%end" | b"%error") {
        return Err(invalid("Unexpected data outside tmux control frame"));
    }
    Ok(())
}

struct Lines<'a, R> {
    reader: &'a mut R,
    left: usize,
    line: Vec<u8>,
}

impl<R: BufRead> Lines<'_, R> {
    fn next(&mut self) -> io::Result<&[u8]> {
        self.line.clear();
        let n = self
            .reader
            .by_ref()
            .take(self.left as u64)
            .read_until(b'\n', &mut self.line)?;
        self.left -= n;
        if !self.line.ends_with(b"\n") {
            if self.left == 0 {
                return Err(invalid("tmux control response exceeds size limit"));
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "tmux control connection closed before response completed",
            ));
        }
        Ok(&self.line)
    }

    fn begin(&mut self, flags: u8) -> io::Result<Id> {
        loop {
            let line = self.next()?;
            if let Some(frame) = id(line, b"%begin ") {
                if frame.flags == flags {
                    return Ok(frame);
                }
                if frame.flags != 0 {
                    return Err(invalid("Unexpected tmux command frame"));
                }
                while end(self.next()?, frame).is_none() {}
            } else {
                notification(line)?;
            }
        }
    }
}

fn check_gap(bytes: &[u8], startup: bool) -> io::Result<()> {
    let mut hook = None;
    let mut offset = 0;
    for line in bytes.split_inclusive(|&b| b == b'\n') {
        if let Some((frame, start)) = hook {
            if let Some(success) = end(line, frame) {
                if startup && !success {
                    return Err(io::Error::other(format!(
                        "tmux initialization command failed: {}",
                        String::from_utf8_lossy(&bytes[start..offset]).trim_end()
                    )));
                }
                hook = None;
            }
        } else if let Some(frame) = id(line, b"%begin ").filter(|f| f.flags == 0) {
            hook = Some((frame, offset + line.len()));
        } else {
            notification(line)?;
        }
        offset += line.len();
    }
    if hook.is_some() {
        return Err(invalid("Incomplete tmux hook frame"));
    }
    Ok(())
}

pub(super) fn response(reader: &mut impl BufRead, flags: u8, marker: &[u8]) -> io::Result<Output> {
    let mut lines = Lines {
        reader,
        left: MAX_RESPONSE,
        line: Vec::new(),
    };
    let request = lines.begin(flags)?;
    let mut bytes = Vec::new();
    let mut closing = None;
    let mut previous_begin = None;
    loop {
        let line = match lines.next() {
            Ok(line) => line,
            Err(e) => {
                // A failed initial attach can exit before it reads the marker.
                if let Some((size, _, false)) = closing.filter(|_| flags == 0) {
                    return Err(io::Error::new(
                        e.kind(),
                        format!(
                            "{e}: {}",
                            String::from_utf8_lossy(&bytes[..size]).trim_end()
                        ),
                    ));
                }
                return Err(e);
            }
        };
        if line.strip_suffix(b"\n") == Some(marker) {
            let (offset, fence) = previous_begin
                .filter(|(_, f): &(usize, Id)| f.flags == 1 && *f != request)
                .ok_or_else(|| invalid("Missing tmux marker frame"))?;
            if end(lines.next()?, fence) != Some(true) {
                return Err(invalid("Mismatched tmux marker terminator"));
            }
            let (size, after, success) = closing
                .filter(|(_, after, _)| *after <= offset)
                .ok_or_else(|| invalid("Missing tmux command terminator"))?;
            check_gap(&bytes[after..offset], flags == 0)?;
            bytes.truncate(size);
            return Ok(output(bytes, success));
        }
        // Only the last matching terminator before the private fence is authoritative.
        if let Some(success) = end(line, request) {
            closing = Some((bytes.len(), bytes.len() + line.len(), success));
        }
        previous_begin = id(line, b"%begin ").map(|f| (bytes.len(), f));
        bytes.extend_from_slice(line);
    }
}

fn output(bytes: Vec<u8>, success: bool) -> Output {
    let (stdout, stderr) = if success {
        (bytes, Vec::new())
    } else {
        (Vec::new(), bytes)
    };
    Output {
        status: ExitStatus::from_raw(if success { 0 } else { 1 << 8 }),
        stdout,
        stderr,
    }
}
