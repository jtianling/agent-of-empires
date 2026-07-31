#!/usr/bin/env bash
# S6 -- a managed pane's pre-registration row must still be valid when ITS OWN
# codex registers.
#
# STATUS: SKELETON. Written but not yet run: the assertions below need the lab
# codex app-server on 8899 (a codex registration makes the daemon resume the
# thread, and without a lab app-server that fails as codex_appserver_unreachable
# before the interesting state is reached). Every TODO(8899) marks a step that
# stays stubbed until then.
#
# Why this scenario exists: the 2026-07-31 incident was triggered by the
# CALLER'S OWN row having expired, not by anything about the victim's row. S1
# covers "must not steal someone else's row"; S6 covers the precondition that
# produced the theft attempt in the first place.
#
#   ./s6-prereg-validity.sh --expiry    TTL runs out before codex registers
#   ./s6-prereg-validity.sh --restart   pane restarts and re-pre-registers mid-flight
#
# Assertions land only on lab-facts.sh's three fact classes (agents row /
# pre-reg row / decision log), never on "looked right on screen".

set -euo pipefail

XATS_REPO="${XATS_REPO:-$HOME/workspace/cross-agent-teams-mcp}"
source "$XATS_REPO/lab/lab-env.sh"
lab_guard_isolation

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOOTSTRAP="$HERE/lab-bootstrap.sh"
FIXTURE="$HERE/tmux-fixture.sh"

MODE="${1:---expiry}"
SESSION="s6"
SHORT_TTL=6
KEY_A="AAAA6666-0000-4000-8000-00000000000A"
KEY_B="BBBB6666-0000-4000-8000-00000000000B"
UUID_A="66666666-1111-4111-8111-11111111111A"
UUID_B="66666666-2222-4222-8222-22222222222B"

lab_tmux_pane_list() { lab_tmux list-panes -t "$SESSION" -F '#{pane_id}'; }

fail() { echo "S6 FAIL: $*" >&2; exit 1; }
note() { echo "S6: $*"; }

# The key-leak gate runs FIRST on every scenario: the property it protects (the
# identity key value never reaching argv) is the one most easily broken by an
# unrelated refactor, and a scenario that ran with a leaking bootstrap would
# produce security conclusions that are simply false.
"$BOOTSTRAP" check-key-leak >/dev/null || fail "key-leak gate failed; refusing to run"
note "key-leak gate passed"

curl -fsS "http://127.0.0.1:$LAB_PORT/health" >/dev/null 2>&1 \
  || fail "lab daemon is not up; run lab/start-lab-daemon.sh --fresh first"

cleanup() {
  "$FIXTURE" down >/dev/null 2>&1 || true
}
trap cleanup EXIT

case "$MODE" in
  --expiry)
    # Shape: pane A's own row expires before its codex registers, while pane B
    # holds a valid row. This is S1's precondition reproduced through the REAL
    # expiry path (S1 fakes it by never creating A's row at all) and with a real
    # codex as the caller.
    #
    # A is the real codex because it is the acting party whose process shape and
    # registration timing are under test. B is a stub carrier: it only has to
    # occupy a seat with a valid row, and stubs cost no model quota. Per the
    # joint spec the scenario's real-codex confirmation is A.
    note "mode=expiry ttl=${SHORT_TTL}s"

    curl -fsS "http://127.0.0.1:$LAB_APPSERVER_PORT/" >/dev/null 2>&1 || \
      nc -z 127.0.0.1 "$LAB_APPSERVER_PORT" >/dev/null 2>&1 || \
      fail "lab app-server not listening on $LAB_APPSERVER_PORT; run lab/start-lab-appserver.sh"

    # Pane A stays an ordinary shell so the bootstrap replica can be launched
    # into it later. Killing a stub with C-c would close its pane outright
    # (remain-on-exit is off), which is why A is not seeded with a carrier.
    lab_tmux kill-session -t "$SESSION" 2>/dev/null || true
    lab_tmux new-session -d -s "$SESSION" -x 200 -y 50
    lab_tmux split-window -t "$SESSION" -d \
      "exec node -e 'setInterval(()=>{},1e9)' -- codex --remote ws://127.0.0.1:$LAB_APPSERVER_PORT -C $LAB -c 'xats.agent_id=\"$UUID_B\"'"
    sleep 1
    mapfile -t PANES < <(lab_tmux list-panes -t "$SESSION" -F '#{pane_id}')
    [ "${#PANES[@]}" -ge 2 ] || fail "expected 2 lab panes"
    PANE_A="${PANES[0]}"
    PANE_B="${PANES[1]}"
    note "panes: A=$PANE_A (shell -> real codex) B=$PANE_B (stub carrier)"

    # B's seat: a valid row with its own key, pre-registered normally.
    XATS_IDENTITY_KEY="$KEY_B" node "$XATS_REPO/dist/cli.js" pre-register-codex-pane \
      --pane "$PANE_B" --agent-id "$UUID_B" --identity-key-env XATS_IDENTITY_KEY \
      --ttl 600 --port "$LAB_PORT" --token "$LAB_TOKEN" >/dev/null \
      || fail "pre-register for pane B failed"
    note "B pre-registered with a valid row"

    # A's seat is pre-registered by the bootstrap replica itself (that is the
    # production path), with a TTL short enough to lapse before its codex
    # finishes coming up and registering.
    db="file:$(lab_db)?mode=ro"
    lab_tmux send-keys -t "$PANE_A" \
      "exec env XATS_IDENTITY_KEY=$KEY_A $BOOTSTRAP run --ttl $SHORT_TTL" Enter
    note "real codex launching in pane A via the bootstrap replica (ttl=${SHORT_TTL}s)"

    waited=0
    while [ "$(sqlite3 "$db" "SELECT COUNT(*) FROM codex_pane_pre_registrations WHERE pane_id='$PANE_A';")" = "0" ]; do
      [ "$waited" -lt 60 ] || fail "bootstrap never pre-registered pane A"
      sleep 0.5
      waited=$((waited + 1))
    done
    note "A's row landed"

    # Expiry here is LAZY: the row is not deleted when it lapses, so asserting on
    # row count would silently never be true. Compare expires_at against now.
    sleep "$((SHORT_TTL + 3))"
    a_expired="$(sqlite3 "$db" "SELECT julianday(expires_at) < julianday('now') FROM codex_pane_pre_registrations WHERE pane_id='$PANE_A';")"
    b_row="$(sqlite3 "$db" "SELECT COALESCE(identity_key,'-') FROM codex_pane_pre_registrations WHERE pane_id='$PANE_B';")"
    [ "$b_row" = "$KEY_B" ] || fail "B's row went missing before the act (got: $b_row)"
    [ "$a_expired" = "1" ] || fail "A's row has not lapsed yet (expires_at still in the future)"
    note "A's row has lapsed (row still present -- expiry is lazy, not a delete); B's row intact"
    lab_tmux send-keys -t "$PANE_A" 'Register to xats with name real-expiry and team lab. Do only that, nothing else.'
    sleep 1
    lab_tmux send-keys -t "$PANE_A" Enter

    for _ in $(seq 1 30); do
      n="$(sqlite3 "$db" "SELECT COUNT(*) FROM agents WHERE team='lab' AND name='real-expiry';" 2>/dev/null || echo 0)"
      [ "$n" -ge 1 ] && break
      sleep 5
    done

    a_pane="$(sqlite3 "$db" "SELECT COALESCE(tmux_pane_id,'-') FROM agents WHERE team='lab' AND name='real-expiry';")"
    a_key="$(sqlite3 "$db" "SELECT COALESCE(identity_key,'-') FROM agents WHERE team='lab' AND name='real-expiry';")"
    b_row_after="$(sqlite3 "$db" "SELECT COALESCE(identity_key,'-') FROM codex_pane_pre_registrations WHERE pane_id='$PANE_B';")"
    a_rows="$(sqlite3 "$db" "SELECT COUNT(*) FROM agents WHERE team='lab' AND name='real-expiry';")"

    [ "$a_rows" = "1" ] || fail "the real codex never registered (would false-green every theft assertion)"
    [ "$b_row_after" = "$KEY_B" ] || fail "B's row was consumed or altered by the expired caller (key: $b_row_after)"
    [ "$a_pane" != "$PANE_B" ] || fail "expired caller claimed B's pane $PANE_B"

    # LIVENESS PAIR (negative half). The caller here has never held a key, so the
    # contradiction guard has no evidence and must stay silent. On its own this
    # would be vacuous the day the string is renamed -- s6-seeded-boundary.sh
    # asserts the same literal MUST appear on the seeded path, and goes red if it
    # ever stops being emitted. Keep both halves on the identical literal.
    contra="$(grep -c 'identity_key_contradiction' "$LAB_DAEMON_LOG" 2>/dev/null || true)"
    [ "${contra:-0}" = "0" ] \
      || fail "identity_key_contradiction fired on the zero-evidence path; the caller was not keyless as assumed"
    note "no identity_key_contradiction on this path, as expected (paired with s6-seeded-boundary.sh)"
    note "PASS: expired caller did not take B's seat (its pane=$a_pane key=$a_key)"
    echo "S6-expiry PASS"
    ;;

  --renewed)
    # Same layout as --expiry, except A's row is STILL VALID when its codex
    # registers -- i.e. the state aoe-main's renewal mitigation is meant to
    # guarantee. Two questions in one run:
    #   Q1 (mitigation) does a valid own-row stop A from taking B's seat, even
    #      though A still passes no identity_key?
    #   Q2 (exposure)   how wide is the window between a row landing and its
    #      codex actually registering? That is how long a victim sits with a
    #      valid row and no live key holder.
    note "mode=renewed ttl=600s (models a renewed/never-lapsed row)"

    nc -z 127.0.0.1 "$LAB_APPSERVER_PORT" >/dev/null 2>&1 \
      || fail "lab app-server not listening on $LAB_APPSERVER_PORT"

    lab_tmux kill-session -t "$SESSION" 2>/dev/null || true
    lab_tmux new-session -d -s "$SESSION" -x 200 -y 50
    lab_tmux split-window -t "$SESSION" -d \
      "exec node -e 'setInterval(()=>{},1e9)' -- codex --remote ws://127.0.0.1:$LAB_APPSERVER_PORT -C $LAB -c 'xats.agent_id=\"$UUID_B\"'"
    sleep 1
    mapfile -t PANES < <(lab_tmux list-panes -t "$SESSION" -F '#{pane_id}')
    [ "${#PANES[@]}" -ge 2 ] || fail "expected 2 lab panes"
    PANE_A="${PANES[0]}"
    PANE_B="${PANES[1]}"
    note "panes: A=$PANE_A (shell -> real codex) B=$PANE_B (stub carrier)"

    XATS_IDENTITY_KEY="$KEY_B" node "$XATS_REPO/dist/cli.js" pre-register-codex-pane \
      --pane "$PANE_B" --agent-id "$UUID_B" --identity-key-env XATS_IDENTITY_KEY \
      --ttl 600 --port "$LAB_PORT" --token "$LAB_TOKEN" >/dev/null \
      || fail "pre-register for pane B failed"
    note "B pre-registered with a valid row"

    db="file:$(lab_db)?mode=ro"
    lab_tmux send-keys -t "$PANE_A" \
      "exec env XATS_IDENTITY_KEY=$KEY_A $BOOTSTRAP run --ttl 600" Enter

    waited=0
    while [ "$(sqlite3 "$db" "SELECT COUNT(*) FROM codex_pane_pre_registrations WHERE pane_id='$PANE_A';")" = "0" ]; do
      [ "$waited" -lt 120 ] || fail "bootstrap never pre-registered pane A"
      sleep 0.5
      waited=$((waited + 1))
    done
    t_row="$(date +%s)"
    note "A's row landed at t=0 (valid, ttl=600)"

    sleep 7
    lab_tmux send-keys -t "$PANE_A" 'Register to xats with name real-renewed and team lab. Do only that, nothing else.'
    sleep 1
    lab_tmux send-keys -t "$PANE_A" Enter
    t_prompt="$(date +%s)"

    for _ in $(seq 1 40); do
      n="$(sqlite3 "$db" "SELECT COUNT(*) FROM agents WHERE team='lab' AND name='real-renewed';" 2>/dev/null || echo 0)"
      [ "$n" -ge 1 ] && break
      sleep 2
    done
    t_reg="$(date +%s)"

    a_pane="$(sqlite3 "$db" "SELECT COALESCE(tmux_pane_id,'-') FROM agents WHERE team='lab' AND name='real-renewed';")"
    a_key="$(sqlite3 "$db" "SELECT COALESCE(identity_key,'-') FROM agents WHERE team='lab' AND name='real-renewed';")"
    a_rows="$(sqlite3 "$db" "SELECT COUNT(*) FROM agents WHERE team='lab' AND name='real-renewed';")"
    b_row_after="$(sqlite3 "$db" "SELECT COALESCE(identity_key,'-') FROM codex_pane_pre_registrations WHERE pane_id='$PANE_B';")"

    printf 'S6: exposure window: row->registered %ss (of which %ss was this script waiting before prompting)\n' \
      "$((t_reg - t_row))" "$((t_prompt - t_row))"

    [ "$a_rows" = "1" ] || fail "the real codex never registered"
    if [ "$a_pane" = "$PANE_A" ] && [ "$b_row_after" = "$KEY_B" ]; then
      note "Q1 mitigation WORKS: A bound its OWN pane $a_pane (key=$a_key); B's row untouched"
      echo "S6-renewed PASS"
    else
      note "Q1 mitigation INSUFFICIENT: A bound $a_pane (key=$a_key); B's row now '$b_row_after'"
      echo "S6-renewed FAIL"
      exit 1
    fi
    ;;

  --restart)
    # Shape: pane A pre-registers, then the pane restarts (new codex process,
    # new agent uuid) and pre-registers AGAIN with the same identity key before
    # the first registration completes. Two rows for one seat, one key.
    #
    # This is the race the recovery handle depends on: if the late registration
    # binds against the stale row, the recovery poke is delivered to a pane that
    # no longer hosts that identity.
    note "mode=restart ttl=${SHORT_TTL}s"

    # TODO(8899): drive the two pre-registrations through lab-bootstrap.sh with
    #   the SAME XATS_IDENTITY_KEY but different agent uuids, then register.
    # Assertions once runnable:
    #   1. exactly one pre-reg row survives for the pane (the newer one)
    #   2. the agents row's identity_key matches the key, and its tmux_pane_id
    #      is the pane that actually hosts the live process
    #   3. decision log shows stale_registration_bind for the late binder
    #      rather than a silent overwrite
    fail "TODO(8899): restart race not runnable until the lab app-server exists"
    ;;

  *)
    fail "unknown mode: $MODE (expected --expiry or --restart)"
    ;;
esac
