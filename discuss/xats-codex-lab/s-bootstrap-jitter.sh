#!/usr/bin/env bash
# Does the xats bootstrap, by itself, stagger rollout landing enough to cross
# two panes' conversations?
#
# Purpose is RULING OUT a second path, not confirming an explanation:
#   ~1-in-5 crossings   -> consistent with "bootstrap jitter staggers landing",
#                          but NOT proof of it (other causes give that too)
#   0 / n               -> the earlier observation had some other cause and we
#                          are still missing one
#   near 7/7            -> something else is amplifying the stagger
#
# Only ONE variable differs from s-rollout-order-cross.sh's `even` cell (0/7):
# cross_agent_team is ON, so each pane runs the full bootstrap (`nc` gate ->
# `npx --no-install pre-register-codex-pane` -> exec codex) before the stub
# starts. The stub still writes its rollout AT LAUNCH, so any stagger measured
# here comes from the bootstrap, not from the stub.
#
# cross_agent_team is set by patching the stored instance rather than by driving
# the new-session dialog: `aoe add` has no flag for it, and driving the TUI adds
# a large surface that has nothing to do with the variable under test.
#
# Also records the landing gap for every round. That distribution is the input
# for choosing any age/timeout threshold later -- picking one without it is
# guesswork.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAB="${XATS_LAB_HOME:-$HOME/.xats-lab-aoe}"
PROFILE="${AOE_LAB_PROFILE:-default}"
AOE_DB="$LAB/aoe-home/.agent-of-empires/profiles/$PROFILE/aoe.db"
SESSIONS="$LAB/aoe-home/.agent-of-empires/profiles/$PROFILE/sessions.json"
SNAP="$(mktemp -d)/aoe.db"
ROUNDS="${ROUNDS:-12}"

note() { echo "JITTER: $*"; }
fail() { echo "JITTER FAIL: $*" >&2; exit 1; }
t() { "$HERE/aoe-lab.sh" tmux "$@"; }
snap() { cp "$AOE_DB" "$SNAP" 2>/dev/null; cp "$AOE_DB-wal" "$SNAP-wal" 2>/dev/null || true
         cp "$AOE_DB-shm" "$SNAP-shm" 2>/dev/null || true; }
q() { snap; sqlite3 "$SNAP" "$1"; }
pane_thread() {
  t capture-pane -p -t "$1" -S -200 -J 2>/dev/null \
    | grep -o 'thread=[0-9a-f-]*' | tail -1 | cut -d= -f2
}

enable_xats() {  # <instance id>
  python3 - "$SESSIONS" "$1" <<'PY'
import json,sys
p,iid=sys.argv[1],sys.argv[2]
d=json.load(open(p))
items=d if isinstance(d,list) else d.get('instances',d.get('sessions',[]))
for i in items:
    if i.get('id')==iid:
        i['cross_agent_team']=True
        i['cross_agent_team_channel']='cross-agent-teams-channel'
json.dump(d,open(p,'w'))
PY
}

[ -x "$LAB/bin/codex" ] || fail "stub codex missing; run: $HERE/aoe-lab.sh shim"
printf 'even' > "$LAB/stub-mode"   # the stub must NOT add stagger of its own

cross=0; n=0; gaps=""
for r in $(seq 1 "$ROUNDS"); do
  "$HERE/aoe-lab.sh" server-up >/dev/null 2>&1 || true
  label="jit-$(uuidgen | cut -c1-6)"
  mark="$(date +%s)"
  id="$("$HERE/aoe-lab.sh" run add "$LAB/project" -t "$label" -c codex 2>&1 \
        | sed -n 's/.*ID: *\([0-9a-f]*\).*/\1/p' | sed -n 1p)"
  [ -n "$id" ] || fail "$label: could not create the session"
  enable_xats "$id"
  "$HERE/aoe-lab.sh" run session start "$id" >/dev/null 2>&1 || fail "$label: start failed"
  "$HERE/aoe-lab.sh" run session add-agent-pane "$id" >/dev/null 2>&1 || fail "$label: add-agent-pane failed"

  i=0
  while [ "$(q "SELECT COUNT(*) FROM agent_slot WHERE instance_id='$id';")" -lt 2 ] && [ $i -lt 60 ]; do
    sleep 1; i=$((i+1))
  done
  if [ "$(q "SELECT COUNT(*) FROM agent_slot WHERE instance_id='$id';")" -lt 2 ]; then
    note "round $r ($label): never reached two slots in ${i}s -- inconclusive, not counted"
    "$HERE/aoe-lab.sh" run remove "$label" --force >/dev/null 2>&1
    continue
  fi
  sleep 2
  n=$((n+1))

  verdict=ok
  while read -r slot pane recorded; do
    [ -n "$slot" ] || continue
    actual="$(pane_thread "$pane")"
    [ "$actual" = "$recorded" ] || { verdict=cross; echo "  round $r slot $slot pane $pane: recorded=$recorded actual=${actual:-<none>}"; }
  done <<< "$(q "SELECT slot||' '||tmux_pane||' '||native_session_id FROM agent_slot WHERE instance_id='$id' ORDER BY slot;")"

  # Landing gap: mtimes of the two rollouts this round produced.
  # BSD find rejects `-newermt @epoch`, and under `set -e` that killed the round
  # before it could report. Take the two newest rollouts instead -- this round
  # just produced them.
  mts="$(ls -t "$LAB/codex-home/sessions/$(date +%Y/%m/%d)"/*.jsonl 2>/dev/null \
         | sed -n '1,2p' | xargs -I{} stat -f '%m' {} 2>/dev/null | sort -n || true)"
  first="$(echo "$mts" | sed -n 1p)"; last="$(echo "$mts" | tail -1)"
  gap="?"
  if [ -n "$first" ] && [ -n "$last" ] && [ "$first" != "$last" ]; then gap=$((last-first));
  elif [ -n "$first" ]; then gap=0; fi
  gaps="$gaps $gap"
  echo "round $r ($label): $verdict, landing gap ${gap}s"
  [ "$verdict" = cross ] && cross=$((cross+1)) || true

  "$HERE/aoe-lab.sh" run remove "$label" --force >/dev/null 2>&1
  t kill-session -t "$(t list-sessions -F '#{session_name}' | grep "$label" | sed -n 1p)" 2>/dev/null || true
done

echo
echo "crossed: $cross / $n rounds (bootstrap ON, stub writes at launch)"
echo "landing gaps (s):$gaps"
