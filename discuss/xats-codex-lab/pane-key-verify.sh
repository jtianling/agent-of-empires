#!/usr/bin/env bash
# Verify the ~/.xats.sh seat-key implementation (server-id scoped) in the lab.
#
# Covers the four points aoe-main asked for, plus a fifth this reviewer added:
#   1. (a) no cross-generation inheritance / (b) same-server reuse
#   2. concurrent cold start -> one shared server_id, distinct per-pane keys
#   3. sibling cleanup must not delete a live sibling in the SAME server, and
#      must not choke when the root holds only flat files
#   4. legacy flat files are neither read nor deleted
#   5. TWO LIVE tmux servers at once -- the cleanup assumes "any dir that is not
#      mine belongs to a dead server", which is false while a second server is
#      running (jt's machine has several: the shared aoe socket, this lab, ...)
#
# The production key root is never touched: the impl copy takes its root from
# $XATS_KEY_ROOT, pinned below to a path under $LAB.

set -euo pipefail
unset TMUX TMUX_PANE

LAB="${LAB:-$HOME/.xats-lab}"
KEYROOT="$LAB/pane-keys-impl"
PROD_KEYROOT="$HOME/.config/xats/codex-pane-keys"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMPL="$HERE/pane-key-impl.zsh"

SOCK_A="$LAB/tmuxtmp/tmux-$(id -u)/default"
SOCK_B="$LAB/tmuxtmp-b/tmux-$(id -u)/default"

die() { printf 'verify: %s\n' "$1" >&2; exit 1; }
ok() { printf '  %-52s PASS\n' "$1"; }
bad() { printf '  %-52s FAIL  %s\n' "$1" "$2"; FAILED=1; }
FAILED=0

case "$KEYROOT" in
  "$PROD_KEYROOT"|"$PROD_KEYROOT"/*) die "key root collides with production" ;;
  "$LAB"/*) ;;
  *) die "key root must live under $LAB" ;;
esac
[ -f "$IMPL" ] || die "impl copy not found: $IMPL"

TA() { env -u TMUX -u TMUX_PANE TMUX_TMPDIR="$LAB/tmuxtmp" tmux -S "$SOCK_A" "$@"; }
TB() { env -u TMUX -u TMUX_PANE TMUX_TMPDIR="$LAB/tmuxtmp-b" tmux -S "$SOCK_B" "$@"; }

# Mirrors _xats-codex-socket-slug: sanitized path plus a shasum suffix.
slug_of() {
  local s="$1"
  printf '%s-%s' "${s//\//_}" "$(printf '%s' "$s" | shasum | cut -c1-8)"
}

# Runs the impl inside a real pane and returns what it printed.
key_in_pane() { key_in_pane_mode "$1" "$2" key; }

key_in_pane_mode() {
  local which="$1" pane="$2" mode="${3:-key}" out
  out="$LAB/keyout.$$.$RANDOM"
  if [ "$which" = A ]; then
    TA send-keys -t "$pane" "XATS_KEY_ROOT=$KEYROOT zsh $IMPL $mode > $out 2>&1; touch $out.done" Enter
  else
    TB send-keys -t "$pane" "XATS_KEY_ROOT=$KEYROOT zsh $IMPL $mode > $out 2>&1; touch $out.done" Enter
  fi
  local waited=0
  while [ ! -f "$out.done" ]; do
    [ "$waited" -lt 100 ] || { printf '<timeout>'; return 0; }
    sleep 0.1
    waited=$((waited + 1))
  done
  cat "$out"
  rm -f "$out" "$out.done"
}

boot() {
  local which="$1" name="$2"
  if [ "$which" = A ]; then
    mkdir -p "$(dirname "$SOCK_A")"
    TA new-session -d -s "$name" -x 120 -y 40
    TA list-panes -t "$name" -F '#{pane_id}' | head -1
  else
    mkdir -p "$(dirname "$SOCK_B")"
    TB new-session -d -s "$name" -x 120 -y 40
    TB list-panes -t "$name" -F '#{pane_id}' | head -1
  fi
}

cleanup() {
  TA kill-server 2>/dev/null || true
  TB kill-server 2>/dev/null || true
}
trap cleanup EXIT

rm -rf "$KEYROOT"
cleanup
sleep 0.3

echo "== 1. (a) cross-generation inheritance / (b) same-server reuse =="
p1="$(boot A gen1)"
k1="$(key_in_pane A "$p1")"
sid1="$(TA show-options -sv @xats_server_id)"
k1b="$(key_in_pane A "$p1")"
[ "$k1b" = "$k1" ] && ok "(b) same pane, same server reuses the key" \
  || bad "(b) same pane, same server reuses the key" "$k1 vs $k1b"

TA kill-server 2>/dev/null || true
sleep 0.5
p2="$(boot A gen2)"
sid2="$(TA show-options -sv @xats_server_id 2>/dev/null || echo '')"
k2="$(key_in_pane A "$p2")"
[ "$p2" = "$p1" ] || bad "premise: pane id repeats after restart" "$p1 -> $p2"
[ "$k2" != "$k1" ] && ok "(a) new generation does NOT inherit the old key" \
  || bad "(a) new generation does NOT inherit the old key" "both $k1"
sid2="$(TA show-options -sv @xats_server_id)"
[ "$sid2" != "$sid1" ] && ok "server_id changed across restart" \
  || bad "server_id changed across restart" "$sid1"

echo "== 2. concurrent cold start on one server =="
TA kill-server 2>/dev/null || true
rm -rf "$KEYROOT"
sleep 0.3
pc1="$(boot A conc)"
TA split-window -t conc
sleep 0.3
mapfile -t CP < <(TA list-panes -t conc -F '#{pane_id}')
[ "${#CP[@]}" -ge 2 ] || die "need 2 panes for the concurrency check"
outc1="$LAB/conc1.out"; outc2="$LAB/conc2.out"
rm -f "$outc1" "$outc2" "$outc1.done" "$outc2.done"
TA send-keys -t "${CP[0]}" "XATS_KEY_ROOT=$KEYROOT zsh $IMPL key > $outc1 2>&1; touch $outc1.done" Enter
TA send-keys -t "${CP[1]}" "XATS_KEY_ROOT=$KEYROOT zsh $IMPL key > $outc2 2>&1; touch $outc2.done" Enter
w=0; while { [ ! -f "$outc1.done" ] || [ ! -f "$outc2.done" ]; } && [ "$w" -lt 100 ]; do sleep 0.1; w=$((w+1)); done
ck1="$(cat "$outc1" 2>/dev/null)"; ck2="$(cat "$outc2" 2>/dev/null)"
# Layout is now <root>/<socket-slug>/<server_id>/<pane>, so server_id dirs sit
# at depth 2.
sid_dirs="$(find "$KEYROOT" -mindepth 2 -maxdepth 2 -type d | wc -l | tr -d ' ')"
[ "$sid_dirs" = "1" ] && ok "both panes share ONE server_id dir (set-if-unset)" \
  || bad "both panes share ONE server_id dir (set-if-unset)" "dirs=$sid_dirs"
[ -n "$ck1" ] && [ -n "$ck2" ] && [ "$ck1" != "$ck2" ] && ok "each pane got a DISTINCT key" \
  || bad "each pane got a DISTINCT key" "$ck1 / $ck2"

echo "== 3. sibling cleanup does not delete a live sibling in the same server =="
nkeys="$(find "$KEYROOT" -type f | wc -l | tr -d ' ')"
[ "$nkeys" = "2" ] && ok "first pane's key survived the second pane's mint" \
  || bad "first pane's key survived the second pane's mint" "key files=$nkeys"

echo "== 3b. root containing ONLY flat files must not error =="
TA kill-server 2>/dev/null || true
rm -rf "$KEYROOT"; mkdir -p "$KEYROOT"
printf 'legacy-value-68' >"$KEYROOT/68"
printf 'legacy-value-71' >"$KEYROOT/71"
chmod 600 "$KEYROOT/68" "$KEYROOT/71"
sleep 0.3
pf="$(boot A flat)"
kf="$(key_in_pane A "$pf")"
case "$kf" in
  ????????-????-????-????-????????????) ok "mints a key with only flat files present" ;;
  *) bad "mints a key with only flat files present" "got: $kf" ;;
esac

echo "== 4. legacy flat files are neither read nor deleted =="
[ -f "$KEYROOT/68" ] && [ -f "$KEYROOT/71" ] && ok "flat files still present after a mint" \
  || bad "flat files still present after a mint" "one or both were removed"
[ "$(cat "$KEYROOT/68")" = "legacy-value-68" ] && ok "flat file contents untouched" \
  || bad "flat file contents untouched" "modified"
[ "$kf" != "legacy-value-68" ] && [ "$kf" != "legacy-value-71" ] && ok "flat files are NOT read as keys" \
  || bad "flat files are NOT read as keys" "returned a legacy value"

echo "== 5. a SECOND live tmux server on a DIFFERENT socket =="
sid_a="$(TA show-options -sv @xats_server_id)"
slug_a="$(slug_of "$(TA display-message -p '#{socket_path}')")"
pb="$(boot B other)"
kb="$(key_in_pane B "$pb")"
sid_b="$(TB show-options -sv @xats_server_id)"
slug_b="$(slug_of "$(TB display-message -p '#{socket_path}')")"
[ "$sid_a" != "$sid_b" ] && ok "the two live servers have distinct server_ids" \
  || bad "the two live servers have distinct server_ids" "$sid_a"
[ "$slug_a" != "$slug_b" ] && ok "the two live servers have distinct socket slugs" \
  || bad "the two live servers have distinct socket slugs" "$slug_a"
if [ -d "$KEYROOT/$slug_a/$sid_a" ]; then
  ok "server A's dir survived server B minting a key"
else
  bad "server A's dir survived server B minting a key" "A is live but its dir was rm -rf'd by B"
fi
ka_after="$(key_in_pane A "$pf")"
[ "$ka_after" = "$kf" ] && ok "server A's key VALUE is unchanged after B minted" \
  || bad "server A's key VALUE is unchanged after B minted" "$kf -> $ka_after"

echo "== 6. same socket, server restart: old dir cleaned, reuse window still shut =="
old_sid="$(TA show-options -sv @xats_server_id)"
old_key="$ka_after"
TA kill-server 2>/dev/null || true
sleep 0.5
pr="$(boot A regen)"
kr="$(key_in_pane A "$pr")"
new_sid="$(TA show-options -sv @xats_server_id)"
[ "$kr" != "$old_key" ] && ok "restarted server's pane got a FRESH key (window still shut)" \
  || bad "restarted server's pane got a FRESH key (window still shut)" "inherited $old_key"
[ ! -d "$KEYROOT/$slug_a/$old_sid" ] && ok "dead server's dir under the same socket was cleaned" \
  || bad "dead server's dir under the same socket was cleaned" "$old_sid still present"
[ -d "$KEYROOT/$slug_a/$new_sid" ] && ok "new server_id dir exists under the same socket slug" \
  || bad "new server_id dir exists under the same socket slug" "missing"
[ -d "$KEYROOT/$slug_b/$sid_b" ] && ok "the OTHER socket's dir was still not touched" \
  || bad "the OTHER socket's dir was still not touched" "cross-socket deletion happened"

echo "== 7. socket_path lookup and its \$TMUX fallback agree =="
slug_primary="$(key_in_pane_mode A "$pr" socket-slug)"
slug_fallback="$(key_in_pane_mode A "$pr" socket-slug-fallback)"
[ -n "$slug_primary" ] && ok "display-message -p '#{socket_path}' works on tmux $(tmux -V | awk '{print $2}')" \
  || bad "display-message socket_path works" "empty"
[ "$slug_primary" = "$slug_fallback" ] && ok "\$TMUX fallback yields the same slug" \
  || bad "\$TMUX fallback yields the same slug" "primary=$slug_primary fallback=$slug_fallback"

echo "== 8. socket path reached through a SYMLINK (/tmp -> /private/tmp) =="
# The hash suffix turns any string difference between the two lookups into two
# different directories. On macOS /tmp is a symlink to /private/tmp, so a server
# started via /tmp is the sharpest available test of whether the primary lookup
# and the $TMUX fallback agree on the socket string.
SOCK_C="/tmp/xatsslug.sock"
TC() { env -u TMUX -u TMUX_PANE tmux -S "$SOCK_C" "$@"; }
TC kill-server 2>/dev/null || true
rm -f "$SOCK_C"
TC new-session -d -s slugtest -x 100 -y 30
sleep 0.3
pc="$(TC list-panes -t slugtest -F '#{pane_id}' | head -1)"
outc="$LAB/slug.primary.out"; outf="$LAB/slug.fallback.out"
rm -f "$outc" "$outf" "$outc.done" "$outf.done"
TC send-keys -t "$pc" "XATS_KEY_ROOT=$KEYROOT zsh $IMPL socket-slug > $outc 2>&1; touch $outc.done" Enter
TC send-keys -t "$pc" "XATS_KEY_ROOT=$KEYROOT zsh $IMPL socket-slug-fallback > $outf 2>&1; touch $outf.done" Enter
w=0; while { [ ! -f "$outc.done" ] || [ ! -f "$outf.done" ]; } && [ "$w" -lt 100 ]; do sleep 0.1; w=$((w+1)); done
sp="$(cat "$outc" 2>/dev/null)"; sf="$(cat "$outf" 2>/dev/null)"
printf '  primary : %s\n' "$sp"
printf '  fallback: %s\n' "$sf"
[ -n "$sp" ] && [ "$sp" = "$sf" ] && ok "symlinked socket: both lookups agree" \
  || bad "symlinked socket: both lookups agree" "primary and fallback disagree -> two dirs for ONE live server"
TC kill-server 2>/dev/null || true
rm -f "$SOCK_C" "$outc" "$outf" "$outc.done" "$outf.done"

echo
[ "$FAILED" = "0" ] && echo "RESULT: all checks passed" || echo "RESULT: at least one check FAILED"
exit "$FAILED"
