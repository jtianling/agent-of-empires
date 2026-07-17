use std::collections::{HashMap, HashSet};

use anyhow::{bail, Result};

/// Return the pane ids referenced by a tmux serialized window layout.
pub fn pane_ids(layout: &str) -> Result<Vec<String>> {
    let (_, body) = split_checksum(layout)?;
    let mut parser = Parser::new(body, None);
    parser.node()?;
    parser.finish()?;
    Ok(parser.panes)
}

/// Replace layout leaf pane ids and recompute tmux's checksum.
pub fn remap(layout: &str, mapping: &HashMap<String, String>) -> Result<String> {
    let (_, body) = split_checksum(layout)?;
    let mut parser = Parser::new(body, Some(mapping));
    let rewritten = parser.node()?;
    parser.finish()?;
    if parser.panes.len() != mapping.len() {
        bail!("layout pane set does not match pane mapping");
    }
    let expected: HashSet<String> = mapping.keys().cloned().collect();
    let actual: HashSet<String> = parser.panes.into_iter().collect();
    if actual.len() != expected.len() || actual != expected {
        bail!("layout has duplicate, missing, or unexpected pane ids");
    }
    Ok(format!("{:04x},{}", checksum(&rewritten), rewritten))
}

fn split_checksum(layout: &str) -> Result<(&str, &str)> {
    let (sum, body) = layout
        .split_once(',')
        .ok_or_else(|| anyhow::anyhow!("layout lacks checksum"))?;
    if sum.len() != 4 || !sum.chars().all(|c| c.is_ascii_hexdigit()) || body.is_empty() {
        bail!("invalid layout checksum prefix");
    }
    Ok((sum, body))
}

fn checksum(body: &str) -> u16 {
    body.bytes().fold(0u16, |sum, byte| {
        sum.rotate_right(1).wrapping_add(u16::from(byte))
    })
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
    mapping: Option<&'a HashMap<String, String>>,
    panes: Vec<String>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, mapping: Option<&'a HashMap<String, String>>) -> Self {
        Self {
            input,
            pos: 0,
            mapping,
            panes: Vec::new(),
        }
    }

    fn node(&mut self) -> Result<String> {
        let geometry = self.geometry()?;
        match self.peek() {
            Some('[') | Some('{') => {
                let open = self.take().unwrap();
                let close = if open == '[' { ']' } else { '}' };
                let mut out = format!("{geometry}{open}");
                loop {
                    out.push_str(&self.node()?);
                    match self.take() {
                        Some(',') => out.push(','),
                        Some(c) if c == close => {
                            out.push(close);
                            break;
                        }
                        _ => bail!("malformed layout container"),
                    }
                }
                Ok(out)
            }
            Some(',') => {
                self.take();
                let pane = self.number()?;
                let old = format!("%{pane}");
                if self.panes.contains(&old) {
                    bail!("duplicate pane id {old}");
                }
                self.panes.push(old.clone());
                let new = match self.mapping {
                    Some(map) => map
                        .get(&old)
                        .ok_or_else(|| anyhow::anyhow!("missing mapping for {old}"))?
                        .trim_start_matches('%')
                        .to_string(),
                    None => pane,
                };
                Ok(format!("{geometry},{new}"))
            }
            _ => bail!("layout node lacks pane or children"),
        }
    }

    fn geometry(&mut self) -> Result<String> {
        let start = self.pos;
        self.number()?;
        self.expect('x')?;
        self.number()?;
        self.expect(',')?;
        self.number()?;
        self.expect(',')?;
        self.number()?;
        Ok(self.input[start..self.pos].to_string())
    }

    fn number(&mut self) -> Result<String> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.take();
        }
        if self.pos == start {
            bail!("expected number in layout");
        }
        Ok(self.input[start..self.pos].to_string())
    }

    fn expect(&mut self, expected: char) -> Result<()> {
        if self.take() != Some(expected) {
            bail!("expected '{expected}' in layout");
        }
        Ok(())
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn take(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn finish(&self) -> Result<()> {
        if self.pos != self.input.len() {
            bail!("trailing data in layout");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).into(), (*b).into()))
            .collect()
    }

    #[test]
    fn parses_and_remaps_horizontal_vertical_and_nested_layouts() {
        for body in [
            "80x24,0,0[40x24,0,0,1,39x24,41,0,2]",
            "80x24,0,0{80x12,0,0,1,80x11,0,13,2}",
            "80x24,0,0[40x24,0,0,1,39x24,41,0{39x12,41,0,2,39x11,41,13,3}]",
        ] {
            let layout = format!("0000,{body}");
            let ids = pane_ids(&layout).unwrap();
            let mapping = ids
                .iter()
                .map(|id| {
                    (
                        id.clone(),
                        format!("%{}", id[1..].parse::<u32>().unwrap() + 10),
                    )
                })
                .collect();
            let rewritten = remap(&layout, &mapping).unwrap();
            assert!(rewritten.len() > body.len());
            assert_eq!(pane_ids(&rewritten).unwrap().len(), ids.len());
        }
    }

    #[test]
    fn rejects_malformed_duplicate_missing_and_mismatched_ids() {
        assert!(pane_ids("bad").is_err());
        assert!(pane_ids("0000,80x24,0,0[40x24,0,0,1,39x24,41,0,1]").is_err());
        let layout = "0000,80x24,0,0[40x24,0,0,1,39x24,41,0,2]";
        assert!(remap(layout, &map(&[("%1", "%8")])).is_err());
        assert!(remap(layout, &map(&[("%1", "%8"), ("%2", "%9"), ("%3", "%10")])).is_err());
    }
}
