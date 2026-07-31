#!/usr/bin/env bash
# Where exactly does identity_key_contradiction stop protecting us?
#
# xats-main's claim: the guard reads the key already on the caller's agents row,
# not the register_agent argument, and a keyless re-registration cannot wash that
# key off (COALESCE upsert). Therefore the guard only fails to fire for an
# identity whose row has NEVER been given a key -- i.e. first registration.
#
# Verified here in three steps, on facts rather than on the claim:
#   1. seed a key the production way: a pre-reg row carrying the key is CONSUMED
#      by the identity (not passed as a register_agent argument)
#   2. re-register that identity WITHOUT a key; the row's key must survive
#   3. replay the S6-expiry shape on the now-seeded identity: its own row is gone
#      and a neighbour holds a valid row. Does the guard fire and stop the theft?
#
# Stub carriers throughout: this tests daemon branch selection, which does not
# depend on real-codex process shape (S6 already established that separately).

set -euo pipefail

XATS_REPO="${XATS_REPO:-$HOME/workspace/cross-agent-teams-mcp}"
source "$XATS_REPO/lab/lab-env.sh"
lab_guard_isolation

SESSION="s6seed"
KEY_S="5EEDEDEE-0000-4000-8000-00000000000S"
KEY_B="B0B0B0B0-0000-4000-8000-00000000000B"
UUID_S="55555555-1111-4111-8111-11111111111S"
UUID_B="55555555-2222-4222-8222-22222222222B"
TEAM="lab"
NAME="seeded-x"

fail() { echo "BOUNDARY FAIL: $*" >&2; exit 1; }
note() { echo "BOUNDARY: $*"; }
db="file:$(lab_db)?mode=ro"
q() { sqlite3 "$db" "$1"; }

curl -fsS "http://127.0.0.1:$LAB_PORT/health" >/dev/null 2>&1 \
  || fail "lab daemon is not up (start it with --fresh)"

cleanup() {
  [ -n "${HOLD:-}" ] && kill "$HOLD" 2>/dev/null || true
  lab_tmux kill-session -t "$SESSION" 2>/dev/null || true
}
trap cleanup EXIT

carrier() {
  printf "exec node -e 'setInterval(()=>{},1e9)' -- codex --remote ws://127.0.0.1:%s -C %s -c 'xats.agent_id=\"%s\"'" \
    "$LAB_APPSERVER_PORT" "$LAB" "$1"
}

lab_tmux kill-session -t "$SESSION" 2>/dev/null || true
lab_tmux new-session -d -s "$SESSION" -x 200 -y 50 "$(carrier "$UUID_S")"
lab_tmux split-window -t "$SESSION" -d "$(carrier "$UUID_B")"
sleep 1
mapfile -t PANES < <(lab_tmux list-panes -t "$SESSION" -F '#{pane_id}')
[ "${#PANES[@]}" -ge 2 ] || fail "expected 2 panes"
PANE_S="${PANES[0]}"
PANE_B="${PANES[1]}"
note "panes: S=$PANE_S (to be seeded) B=$PANE_B (neighbour)"

# --- step 1: seed the key through a real pre-reg consumption -----------------
XATS_IDENTITY_KEY="$KEY_S" node "$XATS_REPO/dist/cli.js" pre-register-codex-pane \
  --pane "$PANE_S" --agent-id "$UUID_S" --identity-key-env XATS_IDENTITY_KEY \
  --ttl 600 --port "$LAB_PORT" --token "$LAB_TOKEN" >/dev/null \
  || fail "pre-register for the seed pane failed"

node "$XATS_REPO/lab/lab-mcp.mjs" --hold register_agent \
  "{\"agent_type\":\"codex\",\"name\":\"$NAME\",\"team\":\"$TEAM\",\"delivery\":{\"kind\":\"none\"}}" \
  >/dev/null 2>&1 &
HOLD=$!
sleep 3

seeded_key="$(q "SELECT COALESCE(identity_key,'-') FROM agents WHERE team='$TEAM' AND name='$NAME';")"
seeded_pane="$(q "SELECT COALESCE(tmux_pane_id,'-') FROM agents WHERE team='$TEAM' AND name='$NAME';")"
row_s_left="$(q "SELECT COUNT(*) FROM codex_pane_pre_registrations WHERE pane_id='$PANE_S';")"
[ "$seeded_key" = "$KEY_S" ] \
  || fail "step 1: key was not attached by consuming the row (got: $seeded_key)"
[ "$row_s_left" = "0" ] || fail "step 1: the seed row was not consumed"
note "step 1 PASS: key seeded via row consumption (key=$seeded_key pane=$seeded_pane, row consumed)"

# --- step 2: keyless re-registration must not wash the key off ---------------
kill "$HOLD" 2>/dev/null || true
sleep 0.5
node "$XATS_REPO/lab/lab-mcp.mjs" --hold register_agent \
  "{\"agent_type\":\"codex\",\"name\":\"$NAME\",\"team\":\"$TEAM\",\"delivery\":{\"kind\":\"none\"}}" \
  >/dev/null 2>&1 &
HOLD=$!
sleep 3
after_key="$(q "SELECT COALESCE(identity_key,'-') FROM agents WHERE team='$TEAM' AND name='$NAME';")"
[ "$after_key" = "$KEY_S" ] \
  || fail "step 2: keyless re-registration washed the key off (now: $after_key)"
note "step 2 PASS: keyless re-registration preserved the key (COALESCE holds)"

# --- step 3: S6-expiry shape on the SEEDED identity -------------------------
# Its own row is already gone (consumed in step 1) -- the same state an expired
# row leaves behind. The neighbour now gets a valid row with a different key.
XATS_IDENTITY_KEY="$KEY_B" node "$XATS_REPO/dist/cli.js" pre-register-codex-pane \
  --pane "$PANE_B" --agent-id "$UUID_B" --identity-key-env XATS_IDENTITY_KEY \
  --ttl 600 --port "$LAB_PORT" --token "$LAB_TOKEN" >/dev/null \
  || fail "pre-register for the neighbour failed"
note "step 3: neighbour B holds a valid row; seeded identity has no row of its own"

log_before="$(grep -c 'identity_key_contradiction' "$LAB_DAEMON_LOG" 2>/dev/null || true)"
kill "$HOLD" 2>/dev/null || true
sleep 0.5
node "$XATS_REPO/lab/lab-mcp.mjs" --hold register_agent \
  "{\"agent_type\":\"codex\",\"name\":\"$NAME\",\"team\":\"$TEAM\",\"delivery\":{\"kind\":\"none\"}}" \
  >/dev/null 2>&1 &
HOLD=$!
sleep 4

final_pane="$(q "SELECT COALESCE(tmux_pane_id,'-') FROM agents WHERE team='$TEAM' AND name='$NAME';")"
final_key="$(q "SELECT COALESCE(identity_key,'-') FROM agents WHERE team='$TEAM' AND name='$NAME';")"
row_b_after="$(q "SELECT COALESCE(identity_key,'-') FROM codex_pane_pre_registrations WHERE pane_id='$PANE_B';")"
log_after="$(grep -c 'identity_key_contradiction' "$LAB_DAEMON_LOG" 2>/dev/null || true)"

printf 'BOUNDARY: final: pane=%s key=%s | B row=%s | contradiction log lines %s -> %s\n' \
  "$final_pane" "$final_key" "$row_b_after" "$log_before" "$log_after"

[ "$row_b_after" = "$KEY_B" ] || fail "step 3: the seeded identity CONSUMED B's row (key: $row_b_after)"
[ "$final_pane" != "$PANE_B" ] || fail "step 3: the seeded identity claimed B's pane"
[ "$final_key" = "$KEY_S" ] || fail "step 3: the seeded identity's key changed to $final_key"
# LIVENESS PAIR (positive half). s6-prereg-validity.sh --expiry asserts this same
# string must NOT appear on the zero-evidence path. A negative assertion whose
# needle has been renamed or deleted is true forever, so it needs a partner that
# fails loudly when the string stops being emitted -- that partner is this line.
# Keep both sides referencing the identical literal.
[ "$log_after" -gt "$log_before" ] \
  || fail "step 3: theft was blocked but identity_key_contradiction was never logged -- either the guard fired for another reason, or the string was renamed (check src/ and update BOTH halves of the pair)"
note "step 3 PASS: seeded identity did not take B's seat, via identity_key_contradiction"
echo "SEEDED-BOUNDARY PASS"
