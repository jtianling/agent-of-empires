use std::collections::{HashMap, HashSet};

use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;

/// Return the pane ids referenced by a tmux serialized window layout.
pub fn pane_ids(layout: &str) -> Result<Vec<String>> {
    if layout.starts_with('{') {
        return pane_ids_v2(layout);
    }
    let (_, body) = split_checksum(layout)?;
    let mut parser = Parser::new(body, None);
    parser.node()?;
    parser.finish()?;
    Ok(parser.panes)
}

/// Replace layout leaf pane ids and recompute tmux's checksum (or rewrite v2 JSON).
pub fn remap(layout: &str, mapping: &HashMap<String, String>) -> Result<String> {
    if layout.starts_with('{') {
        return remap_v2(layout, mapping);
    }
    let (_, body) = split_checksum(layout)?;
    let mut parser = Parser::new(body, Some(mapping));
    let rewritten = parser.node()?;
    parser.finish()?;
    validate_mapped_panes(&parser.panes, mapping)?;
    Ok(format!("{:04x},{}", checksum(&rewritten), rewritten))
}

fn validate_mapped_panes(panes: &[String], mapping: &HashMap<String, String>) -> Result<()> {
    if panes.len() != mapping.len() {
        bail!("layout pane set does not match pane mapping");
    }
    let expected: HashSet<&String> = mapping.keys().collect();
    let actual: HashSet<&String> = panes.iter().collect();
    if actual.len() != expected.len() || actual != expected {
        bail!("layout has duplicate, missing, or unexpected pane ids");
    }
    Ok(())
}

fn valid_pane_id(id: &str) -> bool {
    id.strip_prefix('%')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

fn pane_ids_v2(layout: &str) -> Result<Vec<String>> {
    let mut doc: Value = serde_json::from_str(layout).context("invalid v2 layout JSON")?;
    let root = v2_root_mut(&mut doc)?;
    let mut panes = Vec::new();
    walk_v2(root, None, &mut panes)?;
    Ok(panes)
}

fn remap_v2(layout: &str, mapping: &HashMap<String, String>) -> Result<String> {
    let mut doc: Value = serde_json::from_str(layout).context("invalid v2 layout JSON")?;
    let root = v2_root_mut(&mut doc)?;
    let mut panes = Vec::new();
    walk_v2(root, Some(mapping), &mut panes)?;
    validate_mapped_panes(&panes, mapping)?;
    serde_json::to_string(&doc).context("serialize v2 layout JSON")
}

fn v2_root_mut(doc: &mut Value) -> Result<&mut Value> {
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("v2 layout must be a JSON object"))?;
    ensure!(
        obj.get("V").and_then(Value::as_u64) == Some(2),
        "unsupported tmux JSON layout version"
    );
    obj.get_mut("L")
        .ok_or_else(|| anyhow::anyhow!("v2 layout missing root node"))
}

fn walk_v2(
    node: &mut Value,
    mapping: Option<&HashMap<String, String>>,
    panes: &mut Vec<String>,
) -> Result<()> {
    let obj = node
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("v2 layout node must be an object"))?;
    let kind = obj
        .get("t")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("v2 layout node missing type"))?;
    match kind {
        "p" => {
            ensure!(
                !obj.contains_key("c"),
                "v2 pane leaf must not have children"
            );
            let old = obj
                .get("I")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("v2 pane leaf missing id"))?
                .to_owned();
            ensure!(valid_pane_id(&old), "invalid v2 pane id {old}");
            ensure!(!panes.contains(&old), "duplicate pane id {old}");
            panes.push(old.clone());
            if let Some(map) = mapping {
                let new_id = map
                    .get(&old)
                    .ok_or_else(|| anyhow::anyhow!("missing mapping for {old}"))?;
                let normalized = if new_id.starts_with('%') {
                    new_id.clone()
                } else {
                    format!("%{new_id}")
                };
                ensure!(
                    valid_pane_id(&normalized),
                    "invalid mapped pane id {new_id}"
                );
                obj.insert("I".into(), Value::String(normalized));
            }
            Ok(())
        }
        "h" | "v" => {
            let children = obj
                .get_mut("c")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| anyhow::anyhow!("v2 layout container missing children"))?;
            ensure!(!children.is_empty(), "v2 layout container has no children");
            for child in children {
                walk_v2(child, mapping, panes)?;
            }
            Ok(())
        }
        other => bail!("unsupported v2 layout node type {other}"),
    }
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

    #[test]
    fn parses_and_remaps_tmux_3_8_json_v2_layouts() {
        let single = r#"{"V":2,"L":{"t":"p","w":80,"h":24,"x":0,"y":0,"a":true,"i":0,"I":"%0"}}"#;
        assert_eq!(pane_ids(single).unwrap(), vec!["%0"]);
        let remapped_single = remap(single, &map(&[("%0", "%9")])).unwrap();
        assert_eq!(pane_ids(&remapped_single).unwrap(), vec!["%9"]);

        let nested = concat!(
            r#"{"V":2,"L":{"t":"h","w":80,"h":24,"x":0,"y":0,"c":["#,
            r#"{"t":"p","w":40,"h":24,"x":0,"y":0,"l":1,"i":0,"I":"%0"},"#,
            r#"{"t":"v","w":39,"h":24,"x":41,"y":0,"c":["#,
            r#"{"t":"p","w":39,"h":12,"x":41,"y":0,"l":0,"i":1,"I":"%1"},"#,
            r#"{"t":"p","w":39,"h":11,"x":41,"y":13,"a":true,"i":2,"I":"%2"}"#,
            r#"]}]}}"#
        );
        assert_eq!(pane_ids(nested).unwrap(), vec!["%0", "%1", "%2"]);
        let remapped = remap(nested, &map(&[("%0", "%10"), ("%1", "%11"), ("%2", "%12")])).unwrap();
        assert_eq!(pane_ids(&remapped).unwrap(), vec!["%10", "%11", "%12"]);
        assert!(!remapped.contains(' '));

        let dup = r#"{"V":2,"L":{"t":"h","c":[{"t":"p","I":"%1"},{"t":"p","I":"%1"}]}}"#;
        assert!(pane_ids(dup).is_err());
        assert!(remap(nested, &map(&[("%0", "%10")])).is_err());
        assert!(pane_ids(r#"{"V":3,"L":{"t":"p","I":"%0"}}"#).is_err());
    }
}
