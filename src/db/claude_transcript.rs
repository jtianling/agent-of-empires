//! Read the model a Claude pane is actually running from its transcript.
//!
//! `/model` is pure session state inside Claude: it is written to no settings
//! file, no `~/.claude.json`, and no session record, and `claude --resume` does
//! not restore it. The one place it lands on disk is the `message.model` field
//! of the assistant entries Claude appends to
//!
//! ```text
//! ~/.claude/projects/<project-dir>/<session-uuid>.jsonl
//! ```
//!
//! where `<project-dir>` is the conversation's absolute working directory with
//! every `/` replaced by `-`.
//!
//! Transcripts grow to tens of megabytes with single lines past 256 KiB, so the
//! read is a bounded tail window rather than a scan, in the shape
//! [`super::codex_rollout`] already uses. Two kinds of entry in that window are
//! not the pane's own model and are skipped: subagent entries (`isSidechain`)
//! and the placeholder Claude writes for entries it synthesised itself.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Tail window read from a transcript. Sized by the largest single entry
/// observed in practice (268 KiB): a 256 KiB window slices through the middle of
/// exactly the kind of large assistant message that holds the answer.
const TAIL_WINDOW_BYTES: u64 = 1024 * 1024;

/// The model Claude records for entries it synthesised rather than received
/// from a model (an interrupted turn, for instance).
const SYNTHETIC_MODEL: &str = "<synthetic>";

/// Longest model identifier accepted. Real ids are well under this; the bound
/// exists so a corrupt transcript cannot put an unbounded string on a command
/// line.
const MAX_MODEL_LEN: usize = 128;

/// Whether a model identifier is safe to interpolate into a shell command.
///
/// The value ends up in `claude --model <id>`, which `tmux respawn-pane` runs
/// through a shell, so it is validated the same way a persisted resume token is
/// before it can reach a command line.
pub fn is_safe_model_id(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= MAX_MODEL_LEN
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// Claude's project directory name for a working directory: the absolute path
/// with every `/` replaced by `-`.
fn project_dir_name(cwd: &str) -> String {
    let canonical = std::fs::canonicalize(cwd).unwrap_or_else(|_| PathBuf::from(cwd));
    canonical.to_string_lossy().replace('/', "-")
}

/// Where a pane's transcript lives, given the directory its conversation runs in
/// and the conversation's id.
pub fn transcript_path(cwd: &str, native_session_id: &str) -> Option<PathBuf> {
    if cwd.is_empty() || native_session_id.is_empty() {
        return None;
    }
    Some(
        dirs::home_dir()?
            .join(".claude")
            .join("projects")
            .join(project_dir_name(cwd))
            .join(format!("{native_session_id}.jsonl")),
    )
}

/// The model of the last valid assistant entry in the transcript's tail window,
/// or `None` when the file is absent, unreadable, or holds no such entry.
///
/// Never returns an error: a probe is a side channel, and every failure has to
/// leave the caller's own work (a restart, a reconcile pass) untouched.
pub fn detect_model(path: &Path) -> Option<String> {
    let window = read_tail_window(path)?;
    let text = String::from_utf8_lossy(&window.bytes);
    let mut lines = text.lines();
    // The window starts mid-file, so its first line is almost certainly the tail
    // of an entry rather than an entry.
    if window.truncated_head {
        lines.next();
    }
    let lines: Vec<&str> = lines.collect();
    lines.iter().rev().find_map(|line| model_from_line(line))
}

struct TailWindow {
    bytes: Vec<u8>,
    truncated_head: bool,
}

fn read_tail_window(path: &Path) -> Option<TailWindow> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) => {
            tracing::debug!(
                "claude transcript: cannot open {}: {}",
                path.display(),
                error
            );
            return None;
        }
    };
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_WINDOW_BYTES);
    if start > 0 {
        file.seek(SeekFrom::Start(start)).ok()?;
    }
    let mut bytes = Vec::new();
    if let Err(error) = file.take(TAIL_WINDOW_BYTES).read_to_end(&mut bytes) {
        tracing::debug!(
            "claude transcript: cannot read {}: {}",
            path.display(),
            error
        );
        return None;
    }
    Some(TailWindow {
        bytes,
        truncated_head: start > 0,
    })
}

/// The model named by one transcript line, if that line is an assistant entry
/// belonging to this conversation and carrying a real model.
fn model_from_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;
    if entry.get("type").and_then(serde_json::Value::as_str) != Some("assistant") {
        return None;
    }
    if entry
        .get("isSidechain")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return None;
    }
    let model = entry
        .get("message")?
        .get("model")
        .and_then(serde_json::Value::as_str)?;
    if model == SYNTHETIC_MODEL || !is_safe_model_id(model) {
        return None;
    }
    Some(model.to_string())
}

/// Cheap identity of a transcript file: which file, at what modification time,
/// at what length. Two probes that compute the same fingerprint would read the
/// same bytes, so the second one can be skipped.
///
/// Modification time is taken to the second, the resolution every filesystem
/// and every timestamp-copying tool agrees on; finer digits are present on some
/// and silently dropped by others, so a key that included them would report
/// "changed" for files nothing touched. The cost is that a rewrite landing in
/// the same second at exactly the same length is missed, and then only until
/// the next append moves either.
///
/// Returned as a string because it is persisted on the slot rather than held in
/// memory: reconcile runs in more than one AoE process (the home-view poller and
/// the notification monitor), and a per-process cache would let whichever
/// process probed a file first do all the skipping while the other re-read it
/// every tick.
pub fn fingerprint(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    Some(format!("{}|{}|{}", path.display(), modified, meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPUS: &str = "claude-opus-5";
    const FABLE: &str = "claude-fable-5";

    fn assistant(model: &str, sidechain: bool, text: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "isSidechain": sidechain,
            "message": { "role": "assistant", "model": model, "content": text },
        })
        .to_string()
    }

    fn user(text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "isSidechain": false,
            "message": { "role": "user", "content": text },
        })
        .to_string()
    }

    fn write(lines: &[String]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut body = String::new();
        for line in lines {
            body.push_str(line);
            body.push('\n');
        }
        std::fs::write(&path, body).unwrap();
        (dir, path)
    }

    #[test]
    fn takes_the_last_valid_assistant_entry() {
        let (_dir, path) = write(&[
            user("switch me"),
            assistant(OPUS, false, "on opus"),
            user("again"),
            assistant(FABLE, false, "on fable"),
        ]);
        assert_eq!(detect_model(&path).as_deref(), Some(FABLE));
    }

    #[test]
    fn skips_sidechain_entries() {
        let (_dir, path) = write(&[
            assistant(FABLE, false, "main agent"),
            assistant(OPUS, true, "subagent"),
        ]);
        assert_eq!(detect_model(&path).as_deref(), Some(FABLE));
    }

    #[test]
    fn skips_synthetic_entries() {
        let (_dir, path) = write(&[
            assistant(OPUS, false, "real answer"),
            assistant(SYNTHETIC_MODEL, false, "interrupted"),
        ]);
        assert_eq!(detect_model(&path).as_deref(), Some(OPUS));
    }

    /// A 256 KiB window would cut through the middle of this entry.
    #[test]
    fn an_entry_larger_than_256_kib_is_still_read() {
        let huge = assistant(FABLE, false, &"x".repeat(300 * 1024));
        assert!(huge.len() > 256 * 1024);
        let (_dir, path) = write(&[assistant(OPUS, false, "small"), huge]);
        assert_eq!(detect_model(&path).as_deref(), Some(FABLE));
    }

    /// A window that starts mid-file drops its first line, which is a fragment
    /// rather than an entry. Only entries fully inside the window are read.
    #[test]
    fn a_partial_leading_line_is_discarded() {
        let filler = assistant(OPUS, false, &"y".repeat(TAIL_WINDOW_BYTES as usize));
        let (_dir, path) = write(&[filler, assistant(FABLE, false, "in the window")]);
        assert_eq!(detect_model(&path).as_deref(), Some(FABLE));
    }

    #[test]
    fn a_transcript_without_assistant_entries_yields_nothing() {
        let (_dir, path) = write(&[user("no reply yet")]);
        assert_eq!(detect_model(&path), None);
    }

    #[test]
    fn a_missing_file_yields_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_model(&dir.path().join("absent.jsonl")), None);
    }

    #[test]
    fn a_malformed_tail_yields_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "{not json at all\n{\"type\":\n").unwrap();
        assert_eq!(detect_model(&path), None);
    }

    /// The value reaches a shell through `respawn-pane`, so a transcript that
    /// names something other than a model identifier is not a model.
    #[test]
    fn an_unsafe_model_identifier_is_rejected() {
        let (_dir, path) = write(&[
            assistant(OPUS, false, "safe"),
            assistant("x; rm -rf /", false, "unsafe"),
        ]);
        assert_eq!(detect_model(&path).as_deref(), Some(OPUS));
        assert!(!is_safe_model_id("x; rm -rf /"));
        assert!(!is_safe_model_id(""));
        assert!(is_safe_model_id("claude-opus-4-5-20260514"));
    }

    #[test]
    fn the_project_directory_name_replaces_every_separator() {
        assert_eq!(project_dir_name("/no/such/dir/here"), "-no-such-dir-here");
    }

    #[test]
    fn a_transcript_path_needs_both_a_directory_and_a_conversation() {
        assert_eq!(transcript_path("", "sess"), None);
        assert_eq!(transcript_path("/tmp", ""), None);
    }

    /// Rewriting a file in place and restoring its modification time leaves the
    /// fingerprint identical, so a probe can tell there is nothing new to read
    /// without opening the file at all.
    #[test]
    fn an_unchanged_transcript_keeps_its_fingerprint() {
        let (_dir, path) = write(&[assistant(OPUS, false, "first")]);
        let before = fingerprint(&path).unwrap();

        // The shorter text pays for the longer model name, so the byte count is
        // unchanged; only the content differs.
        let original = std::fs::read_to_string(&path).unwrap();
        let swapped = format!("{}\n", assistant(FABLE, false, "firs"));
        assert_eq!(swapped.len(), original.len());
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, &swapped).unwrap();
        set_mtime(&path, mtime);

        assert_eq!(fingerprint(&path).as_deref(), Some(before.as_str()));

        // Timestamp-copying tools drop the sub-second part, so the same mtime
        // comes back with different fractional digits. Still the same mtime.
        std::fs::write(&path, &swapped).unwrap();
        set_mtime(
            &path,
            SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(
                    mtime
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                ),
        );

        assert_eq!(fingerprint(&path).as_deref(), Some(before.as_str()));
    }

    #[test]
    fn a_changed_transcript_gets_a_new_fingerprint() {
        let (_dir, path) = write(&[assistant(OPUS, false, "first")]);
        let before = fingerprint(&path).unwrap();

        std::fs::write(
            &path,
            format!("{}\n", assistant(FABLE, false, "a longer second turn")),
        )
        .unwrap();

        assert_ne!(fingerprint(&path).as_deref(), Some(before.as_str()));
        assert_eq!(detect_model(&path).as_deref(), Some(FABLE));
    }

    /// Two transcripts in the same state are still two transcripts: the
    /// fingerprint names the file, so a fresh conversation is never mistaken for
    /// the one whose model was last recorded.
    #[test]
    fn two_files_never_share_a_fingerprint() {
        let (dir, first) = write(&[assistant(OPUS, false, "first")]);
        let second = dir.path().join("other.jsonl");
        std::fs::copy(&first, &second).unwrap();
        set_mtime(
            &second,
            std::fs::metadata(&first).unwrap().modified().unwrap(),
        );

        assert_ne!(fingerprint(&first), fingerprint(&second));
    }

    #[test]
    fn a_missing_file_has_no_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(fingerprint(&dir.path().join("absent.jsonl")), None);
    }

    fn set_mtime(path: &Path, mtime: SystemTime) {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
    }
}
