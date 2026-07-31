#!/usr/bin/env bash
# Does scoping the seat-key path by tmux server identity close the
# wrong-inheritance window, without breaking key reuse?
#
# Two variants of the key function are compared side by side:
#   current   $KEYROOT/current/<pane>              (~/.xats.sh today)
#   proposed  $KEYROOT/proposed/<server_pid>/<pane>
#
# Claims under test:
#   (a) after a tmux server restart, the new %0 must NOT receive the previous
#       generation's %0 key
#   (b) within one server lifetime, a codex restart in the same pane MUST still
#       get the same key back (recovery must not regress)
#
# Everything happens on the lab's private tmux server and under $LAB. The
# production key dir (~/.config/xats/codex-pane-keys) is never read or written.

set -euo pipefail

unset TMUX TMUX_PANE

LAB="${LAB:-$HOME/.xats-lab}"
LAB_TMUX_TMPDIR="$LAB/tmuxtmp"
SOCK="$LAB_TMUX_TMPDIR/tmux-$(id -u)/default"
SESSION="pkexp"
KEYROOT="$LAB/pane-keys-exp"
PROD_KEYROOT="$HOME/.config/xats/codex-pane-keys"

die() { printf 'pkexp: %s\n' "$1" >&2; exit 1; }

[ -z "${TMUX:-}" ] || die "TMUX still set"
case "$KEYROOT" in
  "$PROD_KEYROOT"|"$PROD_KEYROOT"/*) die "experiment key root collides with production" ;;
  "$LAB"/*) ;;
  *) die "experiment key root must live under $LAB" ;;
esac

t() { env -u TMUX -u TMUX_PANE TMUX_TMPDIR="$LAB_TMUX_TMPDIR" tmux -S "$SOCK" "$@"; }

server_pid() { t display -p '#{pid}'; }

# Mirrors _xats-codex-pane-key from ~/.xats.sh: reuse if present, else mint.
key_for() {
  local variant="$1" pane="$2" dir file
  case "$variant" in
    current) dir="$KEYROOT/current" ;;
    proposed) dir="$KEYROOT/proposed/$(server_pid)" ;;
    *) die "unknown variant: $variant" ;;
  esac
  file="$dir/${pane#%}"
  if [ -f "$file" ]; then
    cat "$file"
    return 0
  fi
  mkdir -p "$dir"
  local key
  key="$(uuidgen)"
  printf '%s' "$key" >|"$file"
  chmod 600 "$file"
  printf '%s' "$key"
}

boot_session() {
  t new-session -d -s "$SESSION" -x 120 -y 40 "exec sh -c 'while :; do sleep 30; done'"
  local waited=0
  while ! t list-panes -t "$SESSION" -F '#{pane_id}' >/dev/null 2>&1; do
    [ "$waited" -lt 30 ] || die "session did not come up"
    sleep 0.1
    waited=$((waited + 1))
  done
  t list-panes -t "$SESSION" -F '#{pane_id}' | head -1
}

rm -rf "$KEYROOT"
t kill-server 2>/dev/null || true
sleep 0.3

# --- generation 1 -----------------------------------------------------------
pane1="$(boot_session)"
sp1="$(server_pid)"
cur1="$(key_for current "$pane1")"
pro1="$(key_for proposed "$pane1")"
printf 'gen1\tserver_pid=%s\tpane=%s\n' "$sp1" "$pane1"

# (b) same pane, same server: a codex restart re-invokes the function.
cur1b="$(key_for current "$pane1")"
pro1b="$(key_for proposed "$pane1")"
[ "$cur1b" = "$cur1" ] || die "(b) current variant did not reuse the key within one server"
[ "$pro1b" = "$pro1" ] || die "(b) proposed variant did not reuse the key within one server"
printf 'claim_b_same_server_reuse\tPASS\n'

# --- restart the server (the dangerous window) ------------------------------
t kill-server 2>/dev/null || true
sleep 0.5
pane2="$(boot_session)"
sp2="$(server_pid)"
printf 'gen2\tserver_pid=%s\tpane=%s\n' "$sp2" "$pane2"

[ "$pane2" = "$pane1" ] || die "pane id did not repeat ($pane1 -> $pane2); experiment premise broken"
[ "$sp2" != "$sp1" ] || die "server pid did not change across restart; premise broken"
printf 'premise_pane_id_repeated\t%s\n' "$pane2"

cur2="$(key_for current "$pane2")"
pro2="$(key_for proposed "$pane2")"

# (a) the whole point: current inherits, proposed must not.
if [ "$cur2" = "$cur1" ]; then
  printf 'current_variant\tINHERITED the previous generation key (bug reproduced)\n'
else
  printf 'current_variant\tunexpectedly minted a fresh key\n'
fi

if [ "$pro2" = "$pro1" ]; then
  printf 'claim_a_no_cross_generation_inheritance\tFAIL (proposed also inherited)\n'
  t kill-server 2>/dev/null || true
  exit 1
fi
printf 'claim_a_no_cross_generation_inheritance\tPASS\n'

printf 'gen1_dirs\t%s\n' "$(find "$KEYROOT" -type f | wc -l | tr -d ' ') key files total"
find "$KEYROOT" -type f | sed "s|$KEYROOT/||" | sort | sed 's/^/  /'

t kill-server 2>/dev/null || true
printf 'teardown\tlab tmux server stopped\n'
