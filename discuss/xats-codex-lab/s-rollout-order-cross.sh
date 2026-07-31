#!/usr/bin/env bash
# Does the ORDER in which rollout files land decide whether two panes end up
# holding each other's conversations?
#
# The hypothesis under test (not a fix -- an explanation): real codex creates a
# conversation when the pane starts but only writes the rollout file on the
# first turn, while the FILE NAME keeps the start time. So if B's file lands
# before A's, a reconcile tick in between finds exactly one unclaimed rollout --
# B's -- and hands it to whichever pane it is processing.
#
# Known tension, stated up front rather than designed around: the crossing was
# first observed with stubs that wrote their rollout AT LAUNCH, i.e. with both
# files already present. The hypothesis therefore does not explain that
# observation. Either there are two paths, or the hypothesis is wrong. Cell 2
# exists to keep that question open instead of quietly assuming the answer.
#
#   cell 1  stagger: the LATER-started pane's rollout lands FIRST
#   cell 2  even:    both rollouts land before any claim (the original shape),
#                    repeated, to measure how often it crosses at all
#
# Assertion in both: every agent_slot row must record the conversation its own
# pane is actually running. Same invariant as s-stale-pane-live.sh, on purpose:
# "slot holds the sibling's conversation" and "slot holds a stale conversation"
# are the same violation.
#
# Zero quota: the claim reads rollout FILES plus a live process tree, and the
# stub produces a real rollout per launch. No xats bootstrap is needed either --
# claiming does not depend on cross-agent-team being on -- so sessions are built
# with the CLI instead of driving the TUI.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAB="${XATS_LAB_HOME:-$HOME/.xats-lab-aoe}"
PROFILE="${AOE_LAB_PROFILE:-default}"
AOE_DB="$LAB/aoe-home/.agent-of-empires/profiles/$PROFILE/aoe.db"
SNAP="$(mktemp -d)/aoe.db"
ROUNDS="${ROUNDS:-4}"

note() { echo "ROLLOUT-ORDER: $*"; }
fail() { echo "ROLLOUT-ORDER FAIL: $*" >&2; exit 1; }
t() { "$HERE/aoe-lab.sh" tmux "$@"; }
snap() { cp "$AOE_DB" "$SNAP" 2>/dev/null; cp "$AOE_DB-wal" "$SNAP-wal" 2>/dev/null || true
         cp "$AOE_DB-shm" "$SNAP-shm" 2>/dev/null || true; }
q() { snap; sqlite3 "$SNAP" "$1"; }

pane_thread() {
  t capture-pane -p -t "$1" -S -200 -J 2>/dev/null \
    | grep -o 'thread=[0-9a-f-]*' | tail -1 | cut -d= -f2
}

# One round. Prints VERDICT=cross|ok and the raw rows it judged on.
round() {  # <mode> <label>
  local mode="$1" label="$2" id pane slot recorded actual verdict=ok
  # Removing the last session makes tmux exit, and the next tmux call would
  # start a server WITHOUT the lab environment -- the guard catches that, but
  # only after wasting a round. Keep the keeper session alive.
  "$HERE/aoe-lab.sh" server-up >/dev/null 2>&1 || true
  rm -rf "$LAB/stub-order-1"
  printf '%s' "$mode" > "$LAB/stub-mode"
  id="$("$HERE/aoe-lab.sh" run add "$LAB/project" -t "$label" -c codex 2>&1 \
        | sed -n 's/.*ID: *\([0-9a-f]*\).*/\1/p' | sed -n 1p)"
  [ -n "$id" ] || fail "$label: could not create the session"
  "$HERE/aoe-lab.sh" run session start "$id" >/dev/null 2>&1 || fail "$label: session start failed"
  # Both panes must exist before either rollout can be claimed, so add the
  # second one immediately -- the window this is about is the one where two
  # panes are live and the files are not both there yet.
  "$HERE/aoe-lab.sh" run session add-agent-pane "$id" >/dev/null 2>&1 || fail "$label: add-agent-pane failed"

  local i=0
  while [ "$(q "SELECT COUNT(*) FROM agent_slot WHERE instance_id='$id';")" -lt 2 ] && [ $i -lt 60 ]; do
    sleep 1; i=$((i+1))
  done
  [ "$(q "SELECT COUNT(*) FROM agent_slot WHERE instance_id='$id';")" -ge 2 ] \
    || { note "$label: only $(q "SELECT COUNT(*) FROM agent_slot WHERE instance_id='$id';") slot(s) after ${i}s -- recording as inconclusive"; \
         "$HERE/aoe-lab.sh" run remove "$label" --force >/dev/null 2>&1; echo "VERDICT=inconclusive"; return 0; }
  sleep 3

  echo "--- $label ($mode) rows as judged ---"
  q "SELECT '  agent_slot slot='||slot||' pane='||tmux_pane||' recorded='||native_session_id FROM agent_slot WHERE instance_id='$id' ORDER BY slot;"
  q "SELECT '  pane_live   pane='||tmux_pane||' captured='||native_session_id FROM pane_live;"
  while read -r slot pane recorded; do
    [ -n "$slot" ] || continue
    actual="$(pane_thread "$pane")"
    echo "  slot $slot pane $pane: recorded=$recorded actual=${actual:-<none>}"
    [ "$actual" = "$recorded" ] || verdict=cross
  done <<< "$(q "SELECT slot||' '||tmux_pane||' '||native_session_id FROM agent_slot WHERE instance_id='$id' ORDER BY slot;")"

  echo "  rollout files (name carries START time, mtime is when it LANDED):"
  ls -lT "$LAB/codex-home/sessions/$(date +%Y/%m/%d)"/*.jsonl 2>/dev/null | tail -2 \
    | awk '{print "   ", $6, $7, $8, $NF}' | sed 's|.*/||'
  "$HERE/aoe-lab.sh" run remove "$label" --force >/dev/null 2>&1
  t kill-session -t "$(t list-sessions -F '#{session_name}' | grep "$label" | sed -n 1p)" 2>/dev/null || true
  echo "VERDICT=$verdict"
}

[ -x "$LAB/bin/codex" ] || fail "stub codex missing; run: $HERE/aoe-lab.sh shim"

echo "=========== cell 1: stagger (later-started pane's rollout lands first) ==========="
c1_cross=0; c1_n=0
for r in $(seq 1 "$ROUNDS"); do
  out="$(round stagger "stag-$(uuidgen | cut -c1-6)")"
  echo "$out" | grep -v '^VERDICT='
  v="$(echo "$out" | sed -n 's/^VERDICT=//p')"
  [ "$v" = "inconclusive" ] || c1_n=$((c1_n+1))
  [ "$v" = "cross" ] && c1_cross=$((c1_cross+1)) || true
done

echo
echo "=========== cell 2: even (both rollouts land at launch) ==========="
c2_cross=0; c2_n=0
for r in $(seq 1 "$ROUNDS"); do
  out="$(round even "even-$(uuidgen | cut -c1-6)")"
  echo "$out" | grep -v '^VERDICT='
  v="$(echo "$out" | sed -n 's/^VERDICT=//p')"
  [ "$v" = "inconclusive" ] || c2_n=$((c2_n+1))
  [ "$v" = "cross" ] && c2_cross=$((c2_cross+1)) || true
done

echo
echo "cell 1 (stagger): $c1_cross / $c1_n rounds crossed"
echo "cell 2 (even):    $c2_cross / $c2_n rounds crossed"
