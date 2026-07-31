#!/usr/bin/env bash
# Where does a codex tool call actually execute -- in the pane's own codex CLI
# process, or inside the shared app-server?
#
# The question matters because it decides whether the missing edge ("the model
# cannot see its own $XATS_IDENTITY_KEY") can be closed cheaply by relaxing the
# environment policy. If tools run pane-side, the pane's env is right there. If
# they run in the app-server, no environment policy can help -- the app-server's
# env is shared and belongs to nobody's pane.
#
# Measured on the process table, not on the environment: `ps` output is immune
# to whatever shell_environment_policy filtering codex applies, so a single run
# settles the mechanism instead of leaving "maybe there is a third filter".
#
# The chain is walked FROM INSIDE the tool call, in one command. Printing $$ and
# walking it from outside cannot work: that shell is gone by the time anything
# outside reads it, and after pid reuse the walk would produce a plausible chain
# belonging to an unrelated process.
#
# Three panes, because two remote panes alone cannot separate "codex always
# executes tools out-of-process" from "--remote specifically moves them":
#   A, B -- real codex WITH --remote (the shape under test)
#   C    -- real codex WITHOUT --remote (control)
# The sharpest single fact is whether A and B land on the SAME pid.

set -euo pipefail

# Anything that poisons the app-server's environment MUST run on its own port
# with its own CODEX_HOME. The lab's 8899 app-server and $LAB/codex-home config
# are shared with the xats-side scenarios, and a stray XATS_IDENTITY_KEY in the
# app-server env is visible to every session running through it -- measured, and
# it did corrupt a co-worker's run once.
XATS_REPO="${XATS_REPO:-$HOME/workspace/cross-agent-teams-mcp}"
source "$XATS_REPO/lab/lab-env.sh"
lab_guard_isolation

SESSION="anc"
OUT="$LAB/anc"

note() { echo "ANC: $*"; }
fail() { echo "ANC FAIL: $*" >&2; exit 1; }

lsof -nP -iTCP:"$LAB_APPSERVER_PORT" -sTCP:LISTEN >/dev/null 2>&1 \
  || fail "lab app-server is not listening on $LAB_APPSERVER_PORT"

mkdir -p "$OUT"

# One command, run by the model's own shell tool, that walks its ancestry to
# the top and records it atomically. Written to a file rather than scraped off
# the TUI: a 200-column pane wraps and truncates long argv, and argv is exactly
# what identifies the processes here.
chain_cmd() {
  local tag="$1"
  cat <<EOF
p=\$\$; f=$OUT/$tag.txt; : > \$f; echo "tool shell pid=\$\$" >> \$f; n=0; while [ -n "\$p" ] && [ "\$p" != "0" ] && [ "\$n" -lt 12 ]; do ps -o pid=,ppid=,command= -p "\$p" >> \$f 2>/dev/null || break; p=\$(ps -o ppid= -p "\$p" 2>/dev/null | tr -d ' '); n=\$((n+1)); done; echo "--- done ---" >> \$f
EOF
}

launch_remote() {
  printf 'env CODEX_HOME=%s CROSS_AGENT_TEAMS_MCP_TOKEN=%s codex --remote ws://127.0.0.1:%s -C %s -c "xats.agent_id=\\"%s\\"" --dangerously-bypass-approvals-and-sandbox' \
    "$(printf '%q' "$LAB_CODEX_HOME")" "$(printf '%q' "$LAB_TOKEN")" \
    "$LAB_APPSERVER_PORT" "$(printf '%q' "$LAB")" "$1"
}

launch_local() {
  printf 'env CODEX_HOME=%s CROSS_AGENT_TEAMS_MCP_TOKEN=%s codex -C %s -c "xats.agent_id=\\"%s\\"" --dangerously-bypass-approvals-and-sandbox' \
    "$(printf '%q' "$LAB_CODEX_HOME")" "$(printf '%q' "$LAB_TOKEN")" \
    "$(printf '%q' "$LAB")" "$1"
}

case "${1:-up}" in
  up)
    lab_tmux kill-session -t "$SESSION" 2>/dev/null || true
    rm -f "$OUT"/*.txt
    lab_tmux new-session -d -s "$SESSION" -x 200 -y 50 "$(launch_remote "$(uuidgen)")"
    lab_tmux split-window -t "$SESSION" -d "$(launch_remote "$(uuidgen)")"
    lab_tmux split-window -t "$SESSION" -d "$(launch_local "$(uuidgen)")"
    sleep 1
    lab_tmux list-panes -t "$SESSION" -F 'pane=#{pane_id} pid=#{pane_pid} tty=#{pane_tty}'
    ;;
  ask)
    # ask <pane_id> <tag>
    pane="$2"; tag="$3"
    # The prompt must contain no '$' and no '@': typing either into the codex
    # composer opens a completion popup, and the Enter that should submit gets
    # eaten accepting a suggestion instead -- measured, it silently mangled the
    # command mid-string. So the command lives in a file and the prompt only
    # names it.
    prompt="Run this shell command and then reply DONE only: sh $OUT/walk.sh $tag"
    lab_tmux send-keys -t "$pane" -l "$prompt"
    sleep 0.5
    lab_tmux send-keys -t "$pane" Enter
    note "asked $pane -> $OUT/$tag.txt"
    ;;
  screen)
    lab_tmux capture-pane -p -t "$2" | tail -"${3:-25}"
    ;;
  panes)
    lab_tmux list-panes -t "$SESSION" -F 'pane=#{pane_id} pid=#{pane_pid} tty=#{pane_tty} cmd=#{pane_current_command}'
    ;;
  down)
    lab_tmux kill-session -t "$SESSION" 2>/dev/null || true
    note "session $SESSION killed (lab socket only)"
    ;;
  *)
    fail "usage: $0 {up|ask <pane> <tag>|screen <pane> [n]|panes|down}"
    ;;
esac
