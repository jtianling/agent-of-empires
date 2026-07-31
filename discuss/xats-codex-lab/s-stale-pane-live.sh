#!/usr/bin/env bash
# RED assertion for: a Fresh restart leaves the pane's `pane_live` row behind,
# so the pane's NEW conversation is never claimed and its slot keeps pointing at
# the pre-restart one (and, with a sibling pane in play, that unclaimed
# conversation can be taken by the SIBLING's slot instead).
#
# The assertion is the invariant, not the symptom:
#
#     for every agent_slot row: slot.native_session_id MUST equal the
#     conversation actually running in that slot's pane
#
# Stated that way it covers both halves at once -- "slot points at the old
# conversation" (single pane) and "slot points at the sibling's conversation"
# (two panes) are the same violation.
#
# Run order matters and is the point:
#   phase 1  assert BEFORE any restart -- MUST PASS. Without this the phase-3
#            failure proves nothing: an assertion that never held is not
#            evidence of a regression, it is a broken assertion.
#   phase 2  Shift+C (RestartMode::Fresh) driven through the TUI.
#
#            Do NOT "simplify" this to `aoe session restart`, however much more
#            scriptable that looks. That command goes through
#            `restart_with_size`, which does not reuse pane ids -- and a fresh
#            pane id has no `pane_live` row, so the claim runs normally and the
#            invariant holds. The rewrite does not make the test flaky; it makes
#            it PERMANENTLY GREEN while appearing to cover the same thing.
#            Reusing the pane id is the precondition of the whole defect, and
#            only `recover_from_slots` (Shift+C) does that.
#   phase 3  assert again -- RED today. When the fix lands this turns GREEN and
#            phase 1 keeps it honest.
#
# Stub codex is enough and is faithful on the surface that matters: the claim
# reads ROLLOUT FILES plus a live `codex` process tree, and the stub produces a
# real rollout per launch. Nothing here depends on real-codex behaviour.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAB="${XATS_LAB_HOME:-$HOME/.xats-lab-aoe}"
PROFILE="${AOE_LAB_PROFILE:-default}"
AOE_DB="$LAB/aoe-home/.agent-of-empires/profiles/$PROFILE/aoe.db"
TITLE="stale-$(uuidgen | cut -c1-8)"
SNAP="$(mktemp -d)/aoe.db"

fail() { echo "STALE-PANE-LIVE FAIL: $*" >&2; exit 1; }
note() { echo "STALE-PANE-LIVE: $*"; }
t() { "$HERE/aoe-lab.sh" tmux "$@"; }

# The aoe db is WAL and opened by a live TUI; query a snapshot, never the file.
snap() { cp "$AOE_DB" "$SNAP" 2>/dev/null; cp "$AOE_DB-wal" "$SNAP-wal" 2>/dev/null || true
         cp "$AOE_DB-shm" "$SNAP-shm" 2>/dev/null || true; }
q() { snap; sqlite3 "$SNAP" "$1"; }

# What is this pane ACTUALLY running right now? The stub reports the rollout it
# created on its own launch line; take the LAST one, so a respawned pane is read
# as its current conversation and not as whatever the scrollback still shows.
pane_thread() {
  t capture-pane -p -t "$1" -S -200 -J 2>/dev/null \
    | grep -o 'thread=[0-9a-f-]*' | tail -1 | cut -d= -f2
}

assert_slots_match_panes() {
  local phase="$1" rows bad=0
  rows="$(q "SELECT slot || ' ' || tmux_pane || ' ' || native_session_id FROM agent_slot WHERE instance_id='$INSTANCE' ORDER BY slot;")"
  [ -n "$rows" ] || fail "$phase: no agent_slot rows for $INSTANCE -- nothing to assert on"
  while read -r slot pane recorded; do
    [ -n "$slot" ] || continue
    local actual
    actual="$(pane_thread "$pane")"
    if [ -z "$actual" ]; then
      echo "  slot $slot pane $pane: recorded=$recorded actual=<pane reports none>"
      bad=1
      continue
    fi
    if [ "$actual" = "$recorded" ]; then
      echo "  slot $slot pane $pane: OK ($recorded)"
    else
      echo "  slot $slot pane $pane: MISMATCH recorded=$recorded actual=$actual"
      bad=1
    fi
  done <<< "$rows"
  return $bad
}

[ -x "$LAB/bin/codex" ] || fail "stub codex missing; run: $HERE/aoe-lab.sh shim"
sqlite3 --version >/dev/null || fail "sqlite3 required"

note "creating a two-codex-pane session ($TITLE)"
# No `-l`: it ends in an attach, which blocks forever without a tty here.
ID="$("$HERE/aoe-lab.sh" run add "$LAB/project" -t "$TITLE" -c codex 2>&1 \
      | sed -n 's/.*ID: *\([0-9a-f]*\).*/\1/p' | head -1)"
[ -n "$ID" ] || fail "could not resolve the new session id"
"$HERE/aoe-lab.sh" run session start "$ID" >/dev/null 2>&1 || fail "session start failed"
sleep 3
# Pin the instance to the one just created. Selecting "any instance with two
# slots" would silently latch onto a leftover instance from an earlier run --
# it did, and phase 1 caught it.
INSTANCE="$ID"
"$HERE/aoe-lab.sh" run session add-agent-pane "$ID" >/dev/null 2>&1 || fail "add-agent-pane failed"

note "waiting for both panes of $INSTANCE to reach slots"
for _ in $(seq 1 40); do
  n="$(q "SELECT COUNT(*) FROM agent_slot WHERE instance_id='$INSTANCE';")"
  [ "$n" -ge 2 ] && break
  sleep 1
done
[ "$(q "SELECT COUNT(*) FROM agent_slot WHERE instance_id='$INSTANCE';")" -ge 2 ] \
  || fail "the two panes never both reached a slot (claim latency? see events)"
note "instance=$INSTANCE"

echo "--- phase 1: BEFORE any restart (must PASS) ---"
if ! assert_slots_match_panes "phase 1"; then
  fail "phase 1: the invariant does not hold even before a restart -- the assertion or the fixture is wrong, and a phase-3 failure would prove nothing"
fi
note "phase 1 PASS: every slot records the conversation its own pane is running"

echo "--- phase 2: Shift+C (RestartMode::Fresh) ---"
t send-keys -t drv C-b d 2>/dev/null || true
sleep 2
t send-keys -t drv Up; sleep 0.4; t send-keys -t drv Up; sleep 0.6
t send-keys -t drv "C"
sleep 18

echo "--- phase 3: AFTER the restart (RED today) ---"
if assert_slots_match_panes "phase 3"; then
  note "phase 3 PASS -- the invariant survived a Fresh restart"
  echo "STALE-PANE-LIVE PASS (the defect is fixed)"
  exit 0
fi
echo
echo "STALE-PANE-LIVE RED (expected today): a Fresh restart left the pane_live row"
echo "  behind, so the restarted pane's new conversation was never claimed and its"
echo "  slot still records the pre-restart one. Root cause: codex_rollout.rs:62-65"
echo "  returns early when read_pane_live(pane) is Some, and nothing deletes that"
echo "  row on a restart that keeps the pane id."
exit 1
