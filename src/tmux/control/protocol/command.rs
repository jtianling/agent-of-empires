use std::{io, process::Command};

use super::super::literal_format;

const MAX_INPUT: usize = 64 * 1024;

fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason)
}

fn word(out: &mut String, value: &str) {
    out.push_str(" \"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\042"),
            '$' => out.push_str("\\044"),
            '\\' => out.push_str("\\134"),
            '~' => out.push_str("\\176"),
            '\n' => out.push_str("\\012"),
            '\r' => out.push_str("\\015"),
            '\t' => out.push_str("\\011"),
            _ => out.push(ch),
        }
    }
    out.push('"');
}

pub(in crate::tmux::control) fn native_args(
    command: &mut Command,
    args: &[&str],
) -> io::Result<()> {
    if matches!(
        args.first(),
        Some(&("select-pane" | "show-option" | "show-options" | "set-option"))
    ) {
        visit(args, |arg| {
            command.arg(arg);
        })?;
    } else {
        command.args(args);
    }
    Ok(())
}

pub(super) fn encode(args: &[&str]) -> io::Result<String> {
    if args.len() > 128
        || args.iter().any(|s| s.len() > MAX_INPUT || s.contains('\0'))
        || args.iter().map(|s| s.len() + 1).sum::<usize>() > MAX_INPUT
    {
        return Err(invalid("Invalid tmux control argument size or NUL"));
    }
    let mut out = String::new();
    visit(args, |arg| {
        if out.is_empty() {
            out.push_str(arg);
        } else {
            word(&mut out, arg);
        }
    })?;
    out.push('\n');
    Ok(out)
}

fn visit(args: &[&str], mut emit: impl FnMut(&str)) -> io::Result<()> {
    let (&cmd, args) = args
        .split_first()
        .ok_or_else(|| invalid("Missing tmux command"))?;
    let (switches, options) = match cmd {
        "list-panes" => ("as", "tfF"),
        "list-sessions" | "list-clients" => ("", "fF"),
        "capture-pane" => ("paeJNqC", "tSE"),
        "display-message" => ("pl", "tF"),
        "select-pane" => ("", "tT"),
        "show-options" | "show-option" => ("gqvpsw", "t"),
        "set-option" => ("gqupws", "t"),
        _ => return Err(invalid("Unsupported tmux control command")),
    };
    emit(cmd);
    let positional = flags(&mut emit, args, switches, options)?;
    let option_name = matches!(cmd, "set-option" | "show-option" | "show-options");
    match (cmd, positional) {
        ("display-message" | "show-option" | "show-options", [_]) => {}
        ("set-option", [name] | [name, _]) if name.starts_with('@') && name.len() > 1 => {}
        (_, []) if cmd != "set-option" => {}
        _ => return Err(invalid("Unexpected tmux command arguments")),
    }
    if !positional.is_empty() {
        emit("--");
        for (index, value) in positional.iter().enumerate() {
            if option_name && index == 0 {
                emit(&literal_format(value));
            } else {
                emit(value);
            }
        }
    }
    Ok(())
}

fn flags<'a>(
    emit: &mut impl FnMut(&str),
    mut args: &'a [&'a str],
    switches: &str,
    options: &str,
) -> io::Result<&'a [&'a str]> {
    while let Some((&arg, tail)) = args.split_first() {
        if arg == "--" {
            return Ok(tail);
        }
        let Some(group) = arg.strip_prefix('-').filter(|s| !s.is_empty()) else {
            break;
        };
        let at = group.find(|ch| !switches.contains(ch));
        emit(arg);
        args = tail;
        if let Some(option) = at.map(|i| &group[i..]) {
            if option.len() != 1 || !options.contains(option) {
                return Err(invalid("Unsupported tmux command flag"));
            }
            let (&value, rest) = args
                .split_first()
                .ok_or_else(|| invalid("Missing tmux flag argument"))?;
            if matches!(option, "S" | "E") && value != "-" && value.parse::<i32>().is_err() {
                return Err(invalid("Invalid tmux capture line number"));
            }
            // Query formats come from Rust callers. Title data is always literal.
            if option == "T" {
                emit(&literal_format(value));
            } else {
                emit(value);
            }
            args = rest;
        }
    }
    Ok(args)
}
