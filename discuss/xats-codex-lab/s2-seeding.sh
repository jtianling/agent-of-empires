#!/usr/bin/env bash
# S2 -- first-time seeding: a brand-new identity consumes its OWN pre-reg row,
# gets the key cleanly attached, binds its own pane, and does not touch anyone
# else's seat.
#
# STATUS: WRITTEN, NOT YET RUN. Two pending xats-side rulings change what the
# expected outcome is, so running it now would bake in an answer that is about
# to move:
#   - fallback consuming the caller's OWN pending row instead of refusing on
#     pane_has_pending_prereg (would turn S6-renewed green)
#   - pre-reg validity keyed on "pane exists + carrier alive" instead of a wall
#     clock ttl
# Both are tracked with aoe-main. When they land, run this first: it is the
# narrowest scenario and everything else builds on it.
#
# Why this scenario matters more than its number suggests: after the boundary
# result (s6-seeded-boundary.sh), first registration is the ONLY window where
# identity_key_contradiction cannot protect anything -- the caller's row has
# never held a key, so there is no evidence to reason with. S2 is that window.
#
# Fresh name per run, always: a name that has already been seeded carries a key
# on its agents row, which sends the very next run down the contradiction branch
# instead of the zero-evidence branch. The scenario would still "pass" while
# silently testing something else.

set -euo pipefail

XATS_REPO="${XATS_REPO:-$HOME/workspace/cross-agent-teams-mcp}"
source "$XATS_REPO/lab/lab-env.sh"
lab_guard_isolation

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOOTSTRAP="$HERE/lab-bootstrap.sh"

SESSION="s2"
TEAM="lab"
# One-shot suffix: never reuse an identity across runs (see header).
RUN_ID="$(uuidgen | cut -c1-8)"
NAME="seed-$RUN_ID"
KEY_A="A2A2A2A2-0000-4000-8000-${RUN_ID}0000"
KEY_N="42424242-0000-4000-8000-${RUN_ID}0000"
UUID_A="$(uuidgen)"
UUID_N="$(uuidgen)"

fail() { echo "S2 FAIL: $*" >&2; exit 1; }
note() { echo "S2: $*"; }
db="file:$(lab_db)?mode=ro"
q() { sqlite3 "$db" "$1"; }

"$BOOTSTRAP" check-key-leak >/dev/null || fail "key-leak gate failed; refusing to run"
note "key-leak gate passed"

curl -fsS "http://127.0.0.1:$LAB_PORT/health" >/dev/null 2>&1 \
  || fail "lab daemon is not up; start it with --fresh"

# A name that already exists means a previous run leaked into this one, which
# would silently move the test onto the contradiction branch.
[ "$(q "SELECT COUNT(*) FROM agents WHERE team='$TEAM' AND name='$NAME';")" = "0" ] \
  || fail "name $NAME already exists; refusing to run a seeding test on a used identity"

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
lab_tmux new-session -d -s "$SESSION" -x 200 -y 50 "$(carrier "$UUID_A")"
lab_tmux split-window -t "$SESSION" -d "$(carrier "$UUID_N")"
sleep 1
mapfile -t PANES < <(lab_tmux list-panes -t "$SESSION" -F '#{pane_id}')
[ "${#PANES[@]}" -ge 2 ] || fail "expected 2 lab panes"
PANE_A="${PANES[0]}"
PANE_N="${PANES[1]}"
note "panes: A=$PANE_A (seeding) N=$PANE_N (neighbour, must stay untouched)"

# The neighbour carries a live codex-shaped process but deliberately has NO row.
#
# Giving it a valid row too would recreate S6-renewed exactly: two legitimate
# candidates in the scan, fail-closed, nobody bound. That is a real finding, but
# it is S6's finding -- reproducing it here would mean S2 never exercises the
# seeding path it exists to cover. The neighbour's job in S2 is only to be
# something the seeding must not touch.
XATS_IDENTITY_KEY="$KEY_A" node "$XATS_REPO/dist/cli.js" pre-register-codex-pane \
  --pane "$PANE_A" --agent-id "$UUID_A" --identity-key-env XATS_IDENTITY_KEY \
  --ttl 600 --port "$LAB_PORT" --token "$LAB_TOKEN" >/dev/null \
  || fail "pre-register for the seeding pane failed"
note "single valid row: A=$PANE_A (key A); neighbour $PANE_N has a carrier but no row"

# Make the seeding pane the active one. Per the daemon fact recorded in
# discuss/xats-codex-lab.md, the fallback's pane "identification" is a score in
# which the only discriminator between two identical codex panes is +3 for being
# active -- i.e. which pane the user happens to be looking at. Pinning it keeps
# this run deterministic; the assertions below still must not depend on it.
lab_tmux select-pane -t "$PANE_A"

# Register WITHOUT passing a key: production's first registration never does,
# and the whole point of S2 is the zero-prior-key path.
node "$XATS_REPO/lab/lab-mcp.mjs" --hold register_agent \
  "{\"agent_type\":\"codex\",\"name\":\"$NAME\",\"team\":\"$TEAM\",\"delivery\":{\"kind\":\"none\"}}" \
  >/dev/null 2>&1 &
HOLD=$!
sleep 4

rows="$(q "SELECT COUNT(*) FROM agents WHERE team='$TEAM' AND name='$NAME';")"
got_key="$(q "SELECT COALESCE(identity_key,'-') FROM agents WHERE team='$TEAM' AND name='$NAME';")"
got_pane="$(q "SELECT COALESCE(tmux_pane_id,'-') FROM agents WHERE team='$TEAM' AND name='$NAME';")"
got_pid="$(q "SELECT COALESCE(runtime_ui_pid,'-') FROM agents WHERE team='$TEAM' AND name='$NAME';")"
got_verif="$(q "SELECT COALESCE(runtime_verification_mode,'-') FROM agents WHERE team='$TEAM' AND name='$NAME';")"
row_a_left="$(q "SELECT COUNT(*) FROM codex_pane_pre_registrations WHERE pane_id='$PANE_A';")"
rows_n_created="$(q "SELECT COUNT(*) FROM codex_pane_pre_registrations WHERE pane_id='$PANE_N';")"
# Do NOT compare the bound pid against #{pane_pid}. A lab stub execs in place so
# ui_pid == pane_pid, while in production the carrier is a CHILD of the pane
# shell (%71: pane_pid=75561, carrier=83254) -- that assertion would be green in
# the lab and wrong in production. Judge shape-independently instead: the bound
# pid must live on this pane's tty AND carry this pane's marker uuid.
pane_a_tty="$(lab_tmux list-panes -t "$SESSION" -F '#{pane_id} #{pane_tty}' | awk -v p="$PANE_A" '$1==p{print $2}')"
got_pid_tty="$(ps -p "$got_pid" -o tty= 2>/dev/null | tr -d ' ' || true)"
got_pid_argv="$(ps -p "$got_pid" -o command= 2>/dev/null || true)"

printf 'S2: result: key=%s pane=%s pid=%s (tty=%s) verification=%s | own row left=%s | neighbour rows=%s\n' \
  "$got_key" "$got_pane" "$got_pid" "$got_pid_tty" "$got_verif" "$row_a_left" "$rows_n_created"
# verification is RECORDED, never asserted: production shows it stays at
# verified_pid_tty_pane on rows whose pane is empty and whose pid is long dead
# (aoe-codex-r2 / aoe-codex-shell both pinned to the dead pid 11754). It is a
# snapshot taken at bind time, not a live health signal.

# 1. it actually registered -- without this every "did not steal" check below
#    would pass for the wrong reason
[ "$rows" = "1" ] || fail "the identity never registered"
# 2. the key came from its OWN row, cleanly
[ "$got_key" = "$KEY_A" ] || fail "key not attached from its own row (got: $got_key)"
# 3. bound its own pane and the pid that actually hosts it
[ "$got_pane" = "$PANE_A" ] || fail "did not bind its own pane (got: $got_pane)"
[ "$got_pid_tty" = "${pane_a_tty#/dev/}" ] \
  || fail "bound pid $got_pid lives on tty '$got_pid_tty', not on $PANE_A's tty '${pane_a_tty#/dev/}'"
case "$got_pid_argv" in
  *"xats.agent_id=\"$UUID_A\""*) ;;
  *) fail "bound pid $got_pid does not carry $PANE_A's marker uuid $UUID_A" ;;
esac
# 4. its own row was consumed
[ "$row_a_left" = "0" ] || fail "its own row was not consumed"
# 5. the neighbour is untouched: not bound, and no row invented for it
[ "$got_pane" != "$PANE_N" ] || fail "seeding bound the neighbour's pane"
[ "$rows_n_created" = "0" ] || fail "a row appeared for the neighbour's pane ($rows_n_created)"

note "PASS: clean seeding -- own key, own pane, own pid, own row consumed, neighbour untouched"
echo "S2 PASS"
