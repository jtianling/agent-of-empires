#!/usr/bin/env bash
# xats codex lab: private tmux fixture + stub codex panes.
#
# Safety contract (see discuss/xats-codex-lab.md):
#   - every tmux call goes through t(), which runs `env -u TMUX -u TMUX_PANE tmux -S $SOCK`
#   - teardown only ever targets $SOCK (private absolute path), never a bare kill-server
#   - preflight() must pass before any tmux command is issued
#
# Usage:
#   ./tmux-fixture.sh doctor            isolation self-check only, touches no tmux server
#   ./tmux-fixture.sh up [N]            start private server with N stub codex panes (default 2)
#   ./tmux-fixture.sh up --real [N]     same, but each pane runs a real codex under the lab CODEX_HOME
#   ./tmux-fixture.sh ls [--json]       machine-readable pane/pid/tty/uuid inventory
#   ./tmux-fixture.sh down              kill the private server, remove the socket
#
# Env:
#   LAB   lab root, default /tmp/xats-lab (keep it short: sun_path <= 104)

set -euo pipefail

unset TMUX TMUX_PANE

LAB="${LAB:-$HOME/.xats-lab}"
# Socket path and TMUX_TMPDIR must match lab/lab-env.sh exactly. The lab daemon
# shells out to BARE `tmux` (no -S available), so it only finds panes on the
# server that TMUX_TMPDIR resolves to. A fixture on its own socket would host
# panes the daemon can never see, and every pane-claim assertion would be moot.
LAB_TMUX_TMPDIR="$LAB/tmuxtmp"
SOCK="$LAB_TMUX_TMPDIR/tmux-$(id -u)/default"
SESSION="${LAB_SESSION:-xatslab}"
MARKER_PREFIX='xats.agent_id='
PREFLIGHT_OK=""
REAL_CODEX="no"

STUB_JS='process.stdout.write("[stub-codex] ready " + process.argv.slice(1).join(" ") + "\n"); setInterval(function () {}, 1 << 30);'
# Real codex is reached through an npm shim: `node <prefix>/bin/codex <flags>`.
# `node -e <code> -c ...` is impossible (node reads -c as --check, which conflicts
# with --eval), so the stub ships the idle loop as a file to keep the same argv
# skeleton: node + script path + flags, with the marker behind `-c`.
STUB_BIN_REL="bin/codex"

die() {
  printf 'fixture: %s\n' "$1" >&2
  exit 1
}

# Refuse to touch tmux unless the process is provably detached from the caller's
# server and the socket path is a private absolute path under $LAB.
preflight() {
  [ -z "${TMUX:-}" ] || die "TMUX is still set ($TMUX); refusing to run any tmux command"
  [ -z "${TMUX_PANE:-}" ] || die "TMUX_PANE is still set; refusing to run any tmux command"

  case "$LAB" in
    /*) ;;
    *) die "LAB must be an absolute path, got: $LAB" ;;
  esac
  case "$LAB" in
    */) die "LAB must not end with a slash: $LAB" ;;
  esac

  [ "${#SOCK}" -le 104 ] || die "socket path is ${#SOCK} bytes, exceeds sun_path limit of 104: $SOCK"

  case "$SOCK" in
    /private/tmp/tmux-*|/tmp/tmux-*)
      die "socket path collides with the shared default tmux socket dir: $SOCK"
      ;;
    *auto-coworkspace*)
      die "socket path collides with the aoe-managed socket: $SOCK"
      ;;
  esac

  local repo
  repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  case "$LAB/" in
    "$repo"/*) die "LAB must live outside the aoe repo ($repo), got: $LAB" ;;
  esac

  command -v tmux >/dev/null || die "tmux not found on PATH"
  command -v node >/dev/null || die "node not found on PATH"
  command -v uuidgen >/dev/null || die "uuidgen not found on PATH"

  PREFLIGHT_OK=1
}

t() {
  [ -n "$PREFLIGHT_OK" ] || die "internal: tmux called before preflight"
  env -u TMUX -u TMUX_PANE TMUX_TMPDIR="$LAB_TMUX_TMPDIR" tmux -S "$SOCK" "$@"
}

server_running() {
  t has-session -t "$SESSION" 2>/dev/null
}

# Real codex emits an uppercase uuid; keep the case so any case-sensitive
# comparison on the daemon side fails here the same way it would in production.
new_uuid() {
  uuidgen
}

install_stub_bin() {
  mkdir -p "$LAB/$(dirname "$STUB_BIN_REL")"
  printf '%s\n' "$STUB_JS" >"$LAB/$STUB_BIN_REL"
  chmod 755 "$LAB/$STUB_BIN_REL"
}

# tmux runs the pane command through a shell; exec keeps pane_pid == node pid.
# The marker rides behind `-c`, matching how the real codex CLI receives it as a
# config override: `codex ... -c xats.agent_id="<UUID>" ...`.
stub_command() {
  local uuid="$1"
  printf 'exec node %s --remote ws://127.0.0.1:9199 -C %s -c %s --dangerously-bypass-approvals-and-sandbox' \
    "$(printf '%q' "$LAB/$STUB_BIN_REL")" \
    "$(printf '%q' "$LAB")" \
    "$(printf '%q' "${MARKER_PREFIX}\"${uuid}\"")"
}

cmd_doctor() {
  preflight
  printf 'lab_root\t%s\n' "$LAB"
  printf 'socket\t%s\n' "$SOCK"
  printf 'socket_bytes\t%s\n' "${#SOCK}"
  printf 'tmux_env_cleared\tyes\n'
  printf 'socket_exists\t%s\n' "$([ -S "$SOCK" ] && echo yes || echo no)"
  printf 'session_up\t%s\n' "$(server_running && echo yes || echo no)"
}

# Real codex panes reuse the production launch shape from
# src/session/instance.rs::codex_xats_bootstrap_command, minus --remote (the lab
# runs the CLI directly). The misconnect guard must pass before any real codex
# starts, so a misconfigured lab can never register against the production daemon.
real_command() {
  local uuid="$1"
  local home_script token codex_home
  home_script="$(dirname "${BASH_SOURCE[0]}")/codex-home.sh"
  [ -x "$home_script" ] || die "codex-home.sh not found next to this script"

  local env_out
  env_out="$("$home_script" env)" || die "codex-home guard rejected the lab; refusing to start real codex"
  codex_home="$(printf '%s\n' "$env_out" | awk -F= '$1 == "CODEX_HOME" { print $2 }')"
  token="$(printf '%s\n' "$env_out" | awk -F= '$1 == "CROSS_AGENT_TEAMS_MCP_TOKEN" { print $2 }')"
  [ -n "$codex_home" ] || die "codex-home.sh env did not report CODEX_HOME"

  # --dangerously-bypass-approvals-and-sandbox matches the production argv and
  # also clears the first-run directory-trust prompt, which would otherwise block
  # every scenario behind an interactive keypress.
  printf 'exec env CODEX_HOME=%s CROSS_AGENT_TEAMS_MCP_TOKEN=%s codex -C %s -c %s --dangerously-bypass-approvals-and-sandbox' \
    "$(printf '%q' "$codex_home")" \
    "$(printf '%q' "$token")" \
    "$(printf '%q' "$LAB")" \
    "$(printf '%q' "${MARKER_PREFIX}\"${uuid}\"")"
}

pane_command() {
  if [ "$REAL_CODEX" = "yes" ]; then
    real_command "$1"
  else
    stub_command "$1"
  fi
}

cmd_up() {
  if [ "${1:-}" = "--real" ]; then
    REAL_CODEX=yes
    shift
  fi
  local count="${1:-2}"
  case "$count" in
    ''|*[!0-9]*) die "pane count must be a positive integer, got: $count" ;;
  esac
  [ "$count" -ge 1 ] || die "pane count must be >= 1"

  preflight
  mkdir -p "$LAB" "$(dirname "$SOCK")"
  chmod 700 "$LAB" "$LAB_TMUX_TMPDIR" "$(dirname "$SOCK")"

  if server_running; then
    die "session '$SESSION' already exists on $SOCK; run 'down' first"
  fi

  local wait_limit=50
  if [ "$REAL_CODEX" = "yes" ]; then
    wait_limit=300
  else
    install_stub_bin
  fi

  local uuid
  uuid="$(new_uuid)"
  t new-session -d -s "$SESSION" -x 200 -y 200 "$(pane_command "$uuid")"

  local i
  for ((i = 1; i < count; i++)); do
    uuid="$(new_uuid)"
    t split-window -t "$SESSION" -d "$(pane_command "$uuid")"
    t select-layout -t "$SESSION" tiled >/dev/null
  done

  # Panes need a beat before the process has execed and ps can see the marker argv.
  local waited=0 up_now
  while :; do
    up_now="$(inventory_rows | wc -l | tr -d ' ')"
    [ "$up_now" -lt "$count" ] || break
    if [ "$waited" -ge "$wait_limit" ]; then
      cmd_down >/dev/null 2>&1 || true
      die "only $up_now/$count panes came up; tore the private server back down"
    fi
    sleep 0.1
    waited=$((waited + 1))
  done

  cmd_ls
}

# Emits: pane_id \t pane_pid \t tty \t marker_pids \t marker_count \t uuid
# The uuid comes from the process argv, not from tmux state: identifying a codex
# process must go through the argv marker plus the pane process tree, never the
# process name. marker_count is reported because a real codex is a wrapper+child
# pair (comm=node plus comm=codex) carrying the SAME agent_id, so a plain argv
# substring match legitimately hits two pids and the caller has to disambiguate.
inventory_rows() {
  local pane_id pane_pid pane_tty tty_short marker uuid line pid args
  local marker_pids marker_count
  while IFS=$'\t' read -r pane_id pane_pid pane_tty; do
    [ -n "$pane_id" ] || continue
    tty_short="${pane_tty#/dev/}"
    marker=""
    marker_pids=""
    marker_count=0
    while IFS= read -r line; do
      pid="${line%% *}"
      args="${line#* }"
      case "$args" in
        *"$MARKER_PREFIX"*) ;;
        *) continue ;;
      esac
      [ -z "$marker_pids" ] && marker_pids="$pid" || marker_pids="$marker_pids,$pid"
      marker_count=$((marker_count + 1))
      [ -n "$marker" ] || marker="$args"
    done < <(ps -Awwo pid=,tty=,command= |
      awk -v tty="$tty_short" '$2 == tty { pid = $1; $1 = ""; $2 = ""; sub(/^ +/, ""); print pid, $0 }')
    [ "$marker_count" -gt 0 ] || continue
    uuid="${marker#*${MARKER_PREFIX}\"}"
    uuid="${uuid%%\"*}"
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$pane_id" "$pane_pid" "$pane_tty" "$marker_pids" "$marker_count" "$uuid"
  done < <(t list-panes -t "$SESSION" -F '#{pane_id}	#{pane_pid}	#{pane_tty}' 2>/dev/null || true)
}

cmd_ls() {
  preflight
  if [ "${1:-}" = "--json" ]; then
    printf '{"socket":"%s","session":"%s","panes":[' "$SOCK" "$SESSION"
    local first=1 pane_id pane_pid pane_tty marker_pids marker_count uuid
    while IFS=$'\t' read -r pane_id pane_pid pane_tty marker_pids marker_count uuid; do
      [ "$first" -eq 1 ] || printf ','
      first=0
      printf '{"pane_id":"%s","pane_pid":%s,"tty":"%s","marker_pids":[%s],"marker_count":%s,"agent_id":"%s"}' \
        "$pane_id" "$pane_pid" "$pane_tty" "$marker_pids" "$marker_count" "$uuid"
    done < <(inventory_rows)
    printf ']}\n'
  else
    printf 'pane_id\tpane_pid\ttty\tmarker_pids\tmarker_count\tagent_id\n'
    inventory_rows
  fi
}

# Default teardown kills ONLY this fixture's own session by exact name: the lab
# server is now shared with the xats-side scenarios, so killing the server would
# take out a scenario mid-run. --server is the opt-in full teardown, and even
# that is scoped to the lab's private socket path.
cmd_down() {
  preflight
  if [ ! -S "$SOCK" ]; then
    printf 'fixture: no socket at %s, nothing to tear down\n' "$SOCK"
    return 0
  fi
  if [ "${1:-}" = "--server" ]; then
    t kill-server 2>/dev/null || true
    rm -f "$SOCK"
    printf 'fixture: tore down the whole lab tmux server at %s\n' "$SOCK"
    return 0
  fi
  if server_running; then
    t kill-session -t "$SESSION"
    printf 'fixture: killed session %s (lab server left running)\n' "$SESSION"
  else
    printf 'fixture: session %s not present\n' "$SESSION"
  fi
}

main() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    doctor) cmd_doctor "$@" ;;
    up) cmd_up "$@" ;;
    ls) cmd_ls "$@" ;;
    down) cmd_down "$@" ;;
    *)
      printf 'usage: %s {doctor|up [N]|ls [--json]|down}\n' "$(basename "$0")" >&2
      exit 2
      ;;
  esac
}

main "$@"
