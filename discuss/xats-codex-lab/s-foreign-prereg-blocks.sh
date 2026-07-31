#!/usr/bin/env bash
# Does an UNRELATED session's pending pre-registration change what happens to
# mine?
#
#     Invariant: whether a session binds (or re-binds) MUST NOT depend on
#     whether some other session on this machine has a pending pre-reg row.
#
# Written as a differential on purpose. "Two panes of one session both fail" is
# the symptom that started this, but it is the narrow reading: measured
# 2026-08-01, a completely unrelated SINGLE-pane session failed to re-bind
# purely because another session's rows were still pending
# (`candidate_count candidates=2 pending=3`). With TTL at 600s, any codex launch
# anywhere on the machine inside ten minutes is enough, and the failure is
# silent -- the user sees "it did not come back" with nothing pointing at the
# other session.
#
# A differential also survives not knowing how the fix will work: it asserts
# same-inputs-same-outcome, not a particular branch or log string.
#
#   cell 1  fresh registration    A: no foreign row  B: foreign row present
#   cell 2  re-registration after the carrier is replaced (the recovery shape)
#
# Both cells: outcome(B) MUST equal outcome(A). RED today; A binds, B does not.
#
# No aoe and no TUI here: the subject is the daemon's scan, and driving the TUI
# would add a large surface that has nothing to do with the invariant. Panes are
# stub codex carriers shaped exactly the way the scan reads them -- argv with
# `codex ... --remote` and `xats.agent_id="<uuid>"`, foreground on their own tty.

# STATUS 2026-08-01: DOES NOT REPRODUCE THE PRODUCTION CONDITION. Kept for the
# record, NOT usable as M9's acceptance test. Measured: with a lab-mcp caller the
# daemon takes entirely different branches -- `identity_key_live_holder_conflict`
# and `no_match matches=0`, and `seat-follow ... candidates=0` -- and never
# reaches `candidate_count` at all. Cell 2 even PASSES here while the equivalent
# REAL-codex case failed the same day. The difference is the caller shape: real
# codex registers with a `thread_id`, which sends the daemon down a correlation
# path a lab-mcp caller never enters.
#
# So this script would go GREEN without the defect being fixed. M9's acceptance
# has to be driven by a real codex (twice today it produced
# `candidate_count candidates=2 pending=2` deterministically), or be a
# daemon-side test where the caller shape can be constructed exactly.

set -euo pipefail

XATS_REPO="${XATS_REPO:-$HOME/workspace/cross-agent-teams-mcp}"
LAB="${XATS_LAB_HOME:-$HOME/.xats-lab-aoe}"
PORT="${XATS_LAB_PORT:-9198}"
APPPORT="${XATS_LAB_APPSERVER_PORT:-8898}"
SOCK="$LAB/tmuxtmp/tmux-$(id -u)/default"
TOKEN="$(cat "$LAB/token")"
DB="file:$LAB/xats-home/data.db?mode=ro"
RUN="$(uuidgen | cut -c1-8)"

fail() { echo "FOREIGN-PREREG FAIL: $*" >&2; cleanup; exit 1; }
note() { echo "FOREIGN-PREREG: $*"; }
t() { env -u TMUX -u TMUX_PANE TMUX_TMPDIR="$LAB/tmuxtmp" tmux -S "$SOCK" "$@"; }
q() { sqlite3 "$DB" "$1"; }

cleanup() {
  for s in subj-$RUN foreign-$RUN; do t kill-session -t "$s" 2>/dev/null || true; done
}
trap cleanup EXIT

carrier_cmd() {
  printf 'env CODEX_HOME=%s codex --remote ws://127.0.0.1:%s -C %s -c "xats.agent_id=\\"%s\\""' \
    "$LAB/codex-home" "$APPPORT" "$LAB/project" "$1"
}

spawn_carrier() {  # <session> <uuid>
  t kill-session -t "$1" 2>/dev/null || true
  t new-session -d -s "$1" -x 200 -y 50 "$(carrier_cmd "$2")"
  sleep 1
  t list-panes -t "$1" -F '#{pane_id}' | head -1
}

prereg() {  # <pane> <uuid> [key]
  if [ -n "${3:-}" ]; then
    XATS_IDENTITY_KEY="$3" node "$XATS_REPO/dist/cli.js" pre-register-codex-pane \
      --pane "$1" --agent-id "$2" --identity-key-env XATS_IDENTITY_KEY --ttl 600 \
      --port "$PORT" --token "$TOKEN" >/dev/null
  else
    node "$XATS_REPO/dist/cli.js" pre-register-codex-pane \
      --pane "$1" --agent-id "$2" --ttl 600 --port "$PORT" --token "$TOKEN" >/dev/null
  fi
}

# Register as <name> and report the pane it ended up bound to ('-' if none).
register_and_report() {  # <name>
  CROSS_AGENT_TEAMS_MCP_HOME="$LAB/xats-home" CROSS_AGENT_TEAMS_MCP_TOKEN="$TOKEN" \
    timeout 30 node "$XATS_REPO/lab/lab-mcp.mjs" register_agent \
    "{\"agent_type\":\"codex\",\"name\":\"$1\",\"team\":\"labteam-$RUN\",\"delivery\":{\"kind\":\"none\"}}" \
    >/dev/null 2>&1 || true
  sleep 3
  q "SELECT COALESCE(tmux_pane_id,'-') FROM agents WHERE team='labteam-$RUN' AND name='$1';"
}

lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1 || fail "lab daemon is not listening on $PORT"
[ -x "$LAB/bin/codex" ] || fail "stub codex missing; run: aoe-lab.sh shim"

# A leftover pending row from an earlier run is itself a foreign row and would
# make cell 1A behave like 1B -- i.e. the control would fail for the very reason
# under test, and the differential would read "no difference".
LEFT="$(q "SELECT COUNT(*) FROM codex_pane_pre_registrations;")"
[ "$LEFT" = "0" ] || note "warning: $LEFT pre-reg row(s) already pending; cell 1A may not be a clean control"

SUBJ_UUID="$(uuidgen)"
FOREIGN_UUID="$(uuidgen)"
KEY="AAAAAAAA-0000-4000-8000-$(uuidgen | cut -c1-12)"

echo "=== cell 1: fresh registration ==="
SUBJ="$(spawn_carrier "subj-$RUN" "$SUBJ_UUID")"
[ -n "$SUBJ" ] || fail "subject pane did not come up"
prereg "$SUBJ" "$SUBJ_UUID" "$KEY"
note "1A: subject pane $SUBJ, its own row only"
A1="$(register_and_report "one-$RUN")"
note "1A outcome: bound pane = $A1"
[ "$A1" = "$SUBJ" ] \
  || fail "1A: the control did not bind (got '$A1') -- with no foreign row this must work, so either the lab or the daemon is broken and the differential below would be meaningless"

FOREIGN="$(spawn_carrier "foreign-$RUN" "$FOREIGN_UUID")"
[ -n "$FOREIGN" ] || fail "foreign pane did not come up"
prereg "$FOREIGN" "$FOREIGN_UUID"
prereg "$SUBJ" "$SUBJ_UUID" "$KEY"     # the subject announces itself again
note "1B: same subject, but foreign pane $FOREIGN now also has a pending row"
B1="$(register_and_report "two-$RUN")"
note "1B outcome: bound pane = $B1"

echo "=== cell 2: re-registration after the carrier is replaced ==="
# Same shape a Shift+C leaves behind: same pane id, new carrier, a fresh row
# carrying the SAME identity key.
t kill-session -t "foreign-$RUN" 2>/dev/null || true
sleep 2
t respawn-pane -k -t "$SUBJ" "$(carrier_cmd "$(uuidgen)")" 2>/dev/null || true
sleep 2
NEW_UUID="$(t list-panes -a -F '#{pane_id}' | grep -x "$SUBJ" >/dev/null && echo ok)"
[ -n "$NEW_UUID" ] || fail "subject pane vanished on respawn"
SUBJ_UUID2="$(uuidgen)"
t respawn-pane -k -t "$SUBJ" "$(carrier_cmd "$SUBJ_UUID2")"
sleep 2
prereg "$SUBJ" "$SUBJ_UUID2" "$KEY"
note "2A: carrier replaced, fresh row with the same key, no foreign row"
A2="$(register_and_report "three-$RUN")"
note "2A outcome: bound pane = $A2"
[ "$A2" = "$SUBJ" ] \
  || fail "2A: the control did not re-bind (got '$A2') -- the differential below would be meaningless"

# The carrier's uuid and its row's uuid MUST be the same value: the scan finds
# a candidate by looking for `xats.agent_id="<row uuid>"` on that pane's tty.
# Two independent uuidgen calls here is what produced `no_match matches=0` --
# the row never entered the candidate set, so the state under test never existed.
FOREIGN2_UUID="$(uuidgen)"
FOREIGN2="$(spawn_carrier "foreign-$RUN" "$FOREIGN2_UUID")"
prereg "$FOREIGN2" "$FOREIGN2_UUID"
SUBJ_UUID3="$(uuidgen)"
t respawn-pane -k -t "$SUBJ" "$(carrier_cmd "$SUBJ_UUID3")"
sleep 2
prereg "$SUBJ" "$SUBJ_UUID3" "$KEY"
note "2B: same again, but foreign pane $FOREIGN2 has a pending row"
B2="$(register_and_report "four-$RUN")"
note "2B outcome: bound pane = $B2"

echo
echo "cell 1: without foreign row = $A1   with foreign row = $B1"
echo "cell 2: without foreign row = $A2   with foreign row = $B2"
rc=0
[ "$B1" = "$A1" ] || { echo "RED cell 1: a foreign pending row changed a fresh registration ($A1 -> $B1)"; rc=1; }
[ "$B2" = "$A2" ] || { echo "RED cell 2: a foreign pending row changed a re-registration ($A2 -> $B2)"; rc=1; }
if [ "$rc" = 0 ]; then
  echo "FOREIGN-PREREG PASS: an unrelated session's pending row changes nothing"
else
  echo "FOREIGN-PREREG RED (expected today): binding depends on other sessions' pending rows"
  echo "  daemon reason to look for: auto-bind skip (debug): reason=candidate_count"
fi
exit $rc
