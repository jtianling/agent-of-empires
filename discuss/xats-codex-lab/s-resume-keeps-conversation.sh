#!/usr/bin/env bash
# Independent check of the exception carved out by c2289048: a pane that is
# RESUMING its recorded conversation must not be treated as stale and re-claimed.
#
# Why this does not drive the `R` key: `R` is gated on `is_recoverable`
# (tui/home/input.rs:780-792), i.e. it only acts on a session whose panes are
# already gone, and in this lab that recovery does not complete -- it creates
# the placeholder session, kills a process, and leaves the instance in Error
# (reported separately). Waiting on that machinery would test the recovery path,
# not the exception.
#
# So the state the exception is about is built directly, which is also a
# sharper test of it: respawn the SAME pane id in place, once fresh and once
# resuming. Both give the exception's precondition -- a capture older than the
# pane's current process -- and they differ only in whether the pane is running
# the conversation its slot records.
#
#   phase A  respawn FRESH  -> the slot MUST follow the new conversation
#            (this is the fix; without it phase B proves nothing, because a
#            rule that never re-claims anything also "passes" phase B)
#   phase B  respawn RESUMING the recorded conversation -> the slot MUST NOT
#            move (this is the exception)
#
# A and B are a pair on purpose. Phase B alone is satisfied by a system that
# simply never re-claims -- which is the pre-fix behaviour it is meant to rule
# out.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAB="${XATS_LAB_HOME:-$HOME/.xats-lab-aoe}"
PROFILE="${AOE_LAB_PROFILE:-default}"
AOE_DB="$LAB/aoe-home/.agent-of-empires/profiles/$PROFILE/aoe.db"
TITLE="resume-$(uuidgen | cut -c1-8)"
SNAP="$(mktemp -d)/aoe.db"

fail() { echo "RESUME FAIL: $*" >&2; exit 1; }
note() { echo "RESUME: $*"; }
t() { "$HERE/aoe-lab.sh" tmux "$@"; }

snap() { cp "$AOE_DB" "$SNAP" 2>/dev/null; cp "$AOE_DB-wal" "$SNAP-wal" 2>/dev/null || true
         cp "$AOE_DB-shm" "$SNAP-shm" 2>/dev/null || true; }
q() { snap; sqlite3 "$SNAP" "$1"; }

pane_thread() {
  t capture-pane -p -t "$1" -S -200 -J 2>/dev/null \
    | grep -o 'thread=[0-9a-f-]*' | tail -1 | cut -d= -f2
}

slot_session() { q "SELECT native_session_id FROM agent_slot WHERE instance_id='$ID' AND slot=0;"; }

# Respawn in place: -k keeps the pane id, which is what makes the pane's
# `pane_live` capture older than its process -- the exception's precondition.
respawn_pane() {
  t respawn-pane -k -t "$PANE" \
    "env CODEX_HOME=$LAB/codex-home PATH=$LAB/bin:\$PATH codex $1 -C $LAB/project"
}

wait_settle() { sleep 14; }

[ -x "$LAB/bin/codex" ] || fail "stub codex missing; run: $HERE/aoe-lab.sh shim"

note "creating a single-pane codex session ($TITLE)"
ID="$("$HERE/aoe-lab.sh" run add "$LAB/project" -t "$TITLE" -c codex 2>&1 \
      | sed -n 's/.*ID: *\([0-9a-f]*\).*/\1/p' | head -1)"
[ -n "$ID" ] || fail "could not resolve the new session id"
"$HERE/aoe-lab.sh" run session start "$ID" >/dev/null 2>&1 || fail "session start failed"

for _ in $(seq 1 40); do
  [ "$(q "SELECT COUNT(*) FROM agent_slot WHERE instance_id='$ID';")" -ge 1 ] && break
  sleep 1
done
PANE="$(q "SELECT tmux_pane FROM agent_slot WHERE instance_id='$ID' AND slot=0;")"
[ -n "$PANE" ] || fail "the pane never reached a slot"
BASE="$(slot_session)"
[ "$(pane_thread "$PANE")" = "$BASE" ] \
  || fail "baseline broken: slot records $BASE, pane runs $(pane_thread "$PANE")"
note "baseline: pane=$PANE conversation=$BASE"

echo "--- phase A: respawn FRESH (the slot must follow) ---"
PID_A="$(t display-message -p -t "$PANE" '#{pane_pid}')"
respawn_pane ""
wait_settle
NEW_PID="$(t display-message -p -t "$PANE" '#{pane_pid}')"
[ "$NEW_PID" != "$PID_A" ] || fail "phase A: the pane was not respawned"
FRESH_THREAD="$(pane_thread "$PANE")"
AFTER_A="$(slot_session)"
printf 'RESUME: phase A  pane runs %s | slot records %s\n' "$FRESH_THREAD" "$AFTER_A"
[ "$FRESH_THREAD" != "$BASE" ] || fail "phase A: the respawn did not start a new conversation"
[ "$AFTER_A" = "$FRESH_THREAD" ] \
  || fail "phase A: the slot did not follow the pane's new conversation (still $AFTER_A) -- re-claiming is not happening at all, so phase B would be vacuous"
note "phase A PASS: a stale capture was superseded and the slot followed"

echo "--- phase B: respawn RESUMING $AFTER_A (the slot must NOT move) ---"
PID_B="$(t display-message -p -t "$PANE" '#{pane_pid}')"
respawn_pane "resume $AFTER_A"
wait_settle
NEW_PID="$(t display-message -p -t "$PANE" '#{pane_pid}')"
[ "$NEW_PID" != "$PID_B" ] || fail "phase B: the pane was not respawned"
RESUMED_THREAD="$(pane_thread "$PANE")"
AFTER_B="$(slot_session)"
printf 'RESUME: phase B  pane runs %s | slot records %s\n' "$RESUMED_THREAD" "$AFTER_B"
[ "$RESUMED_THREAD" = "$AFTER_A" ] \
  || fail "phase B: the stub did not actually resume ($RESUMED_THREAD) -- fixture problem, not a product result"
[ "$AFTER_B" = "$AFTER_A" ] \
  || fail "phase B: a resuming pane was re-claimed and its slot moved $AFTER_A -> $AFTER_B"

note "PASS: fresh respawn is followed, resuming respawn is left alone"
echo "RESUME PASS"
