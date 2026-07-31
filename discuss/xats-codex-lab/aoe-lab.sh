#!/usr/bin/env bash
# Run the REAL aoe binary inside the lab, so the left/right two-pane launch path
# (d7250023) can be exercised without touching anything of jt's.
#
# Four things must be redirected, and every one of them is fail-fast here rather
# than left to discipline -- each has a specific way of silently poisoning
# production if it is forgotten:
#
#   CODEX_HOME                 aoe scans $CODEX_HOME/sessions for rollouts
#                              (db/codex_rollout.rs:114-121, defaults to
#                              ~/.codex). Unset, aoe would claim one of jt's
#                              REAL conversations for a lab pane and write that
#                              binding to its DB. Claims are deduped and not
#                              reversible, so this is worse than a wrong port.
#   TMUX_TMPDIR                aoe isolates by socket NAME (-L, tmux/mod.rs:
#                              82-128), so the profile name alone decides the
#                              path. With TMUX_TMPDIR unset, `-L default` is
#                              /tmp/tmux-501/default -- a real socket.
#   CROSS_AGENT_TEAMS_MCP_*    the bootstrap's `npx pre-register-codex-pane`
#                              carries no --port/--token, so the CLI resolves
#                              them from $CROSS_AGENT_TEAMS_MCP_HOME/daemon.pid
#                              and $CROSS_AGENT_TEAMS_MCP_TOKEN (cli.ts:90-103,
#                              157). Unset, the lab pre-registers into jt's
#                              production daemon.
#   HOME                       aoe's own data lives under $HOME; a lab run must
#                              not write into jt's ~/.agent-of-empires.
#
#   CROSS_AGENT_TEAMS_CODEX_WS_URL
#                              since 5652db78 the Codex app-server endpoint is
#                              read from this instead of being hardcoded to
#                              8799, and both the `nc -z` gate and `--remote`
#                              derive from it. Unset, a REAL codex here attaches
#                              to the PRODUCTION app-server -- where, as measured
#                              today, its tools execute and its rollout is
#                              written.
#
# Whether the profile may be `default` depends on the lab root. `default` is the
# socket name the daemon finds through bare `tmux` + TMUX_TMPDIR, so pane-binding
# scenarios need it -- any other name leaves the daemon with an empty candidate
# set and every such assertion passes vacuously. On a SHARED lab root that is
# unsafe here, because this fixture puts $LAB/bin at the front of the tmux
# server's PATH and would shadow other people's real codex. On a private root
# (XATS_LAB_HOME) it is the right choice.

# A stub's infidelities do not announce themselves -- they disguise themselves
# as defects in the system under test, convincingly. The claude stub first
# omitted the hook's stdin JSON (`{"session_id":…,"cwd":…}`), which is where
# `aoe __record-pane` reads the conversation id from (cli/record_pane.rs:35-40),
# NOT argv. The visible symptom was an empty `pane_live` -- i.e. "the extra pane
# never reached a slot", which is exactly the defect this fixture exists to look
# for. Before reporting anything as a defect here, check the stub is faithful on
# the specific surface the assertion reads.

set -euo pipefail

XATS_REPO="${XATS_REPO:-$HOME/workspace/cross-agent-teams-mcp}"
source "$XATS_REPO/lab/lab-env.sh"
lab_guard_isolation

AOE_REPO="${AOE_REPO:-$HOME/workspace/agent-of-empires}"
AOE_BIN="${AOE_BIN:-$AOE_REPO/target/release/aoe}"
LAB_AOE_HOME="$LAB/aoe-home"
LAB_BIN="$LAB/bin"
# Deliberate, documented deviation: the bootstrap runs `npx --no-install
# cross-agent-teams-mcp@latest`, which resolves ONLY against an already
# populated npx cache (instance.rs:267-280 explains why the @latest tag is
# load-bearing). Overriding HOME gives the lab an empty cache, so the bootstrap
# dies with "npx canceled due to missing packages" and the pane silently falls
# back to a shell. Point npm at the real cache: `--no-install` never writes
# packages there, so this stays a read.
REAL_NPM_CACHE="${AOE_LAB_NPM_CACHE:-/Users/jtianling/.npm}"
# Since 5652db78 the Codex app-server endpoint is read from xats's own variable
# instead of being hardcoded, and both the `nc -z` gate and `--remote` derive
# from it -- so pointing this at the lab moves the whole thing, and a real Codex
# no longer has to be replaced by a stub. An invalid value now makes the pane
# print a diagnostic and exit 1 rather than falling back, so this guard has a
# second line of defence behind it.
LAB_WS_URL="ws://127.0.0.1:$LAB_APPSERVER_PORT"
LAB_PROJECT="$LAB/project"
PROFILE="${AOE_LAB_PROFILE:-aoelab}"

die() { echo "aoe-lab: $*" >&2; exit 1; }
note() { echo "aoe-lab: $*"; }

# Every guard below is a refusal, never a warning: a lab run that starts with
# one of these wrong produces results that look valid.
guard() {
  [ -x "$AOE_BIN" ] || die "aoe binary not found at $AOE_BIN (cargo build --release)"

  case "$LAB_CODEX_HOME" in
    "$LAB"/*) ;;
    *) die "CODEX_HOME ($LAB_CODEX_HOME) must live under $LAB" ;;
  esac
  [ -d "$LAB_CODEX_HOME" ] || die "CODEX_HOME $LAB_CODEX_HOME does not exist"

  case "$LAB_TMUX_TMPDIR" in
    "$LAB"/*) ;;
    *) die "TMUX_TMPDIR ($LAB_TMUX_TMPDIR) must live under $LAB" ;;
  esac
  # The socket aoe will actually use, derived the same way aoe derives it.
  AOE_SOCK="$LAB_TMUX_TMPDIR/tmux-$(id -u)/$PROFILE"
  case "$AOE_SOCK" in
    "$LAB"/*) ;;
    *) die "refusing: resolved socket $AOE_SOCK is outside the lab" ;;
  esac

  case "$LAB_HOME_DIR" in
    "$LAB"/*) ;;
    *) die "CROSS_AGENT_TEAMS_MCP_HOME ($LAB_HOME_DIR) must live under $LAB" ;;
  esac
  [ -f "$LAB_HOME_DIR/daemon.pid" ] \
    || die "no daemon.pid in $LAB_HOME_DIR -- the pre-register CLI resolves its port from it"
  local pidport
  pidport="$(sed -n 's/.*"port"[: ]*\([0-9]*\).*/\1/p' "$LAB_HOME_DIR/daemon.pid")"
  [ "$pidport" = "$LAB_PORT" ] \
    || die "daemon.pid says port $pidport, expected the lab port $LAB_PORT"
  case "$LAB_TOKEN" in
    lab-*|xatslab-*) ;;
    *) die "LAB_TOKEN is not a lab token; refusing to run" ;;
  esac

  case "$LAB_AOE_HOME" in
    "$LAB"/*) ;;
    *) die "aoe HOME ($LAB_AOE_HOME) must live under $LAB" ;;
  esac

  case "$LAB_WS_URL" in
    *:8799*) die "CROSS_AGENT_TEAMS_CODEX_WS_URL points at the production app-server" ;;
    ws://*|wss://*) ;;
    *) die "CROSS_AGENT_TEAMS_CODEX_WS_URL ($LAB_WS_URL) is not a ws:// or wss:// URL" ;;
  esac
  lsof -nP -iTCP:"$LAB_APPSERVER_PORT" -sTCP:LISTEN >/dev/null 2>&1 \
    || die "no app-server listening on $LAB_APPSERVER_PORT -- the bootstrap's nc gate would fail the pane"

  if [ -n "${AOE_LAB_REAL_CODEX:-}" ]; then
    [ ! -e "$LAB_BIN/codex" ] || die "AOE_LAB_REAL_CODEX is set but the stub codex is still on the lab PATH"
  else
    [ -x "$LAB_BIN/codex" ] || die "stub codex missing; run: $0 shim"
  fi
  mkdir -p "$LAB_AOE_HOME" "$LAB_PROJECT"
  guard_pane_env
}

lt() { env -u TMUX -u TMUX_PANE TMUX_TMPDIR="$LAB_TMUX_TMPDIR" tmux -S "$AOE_SOCK" "$@"; }

# The guard this fixture turned out to need most.
#
# A pane aoe launches does NOT inherit aoe's environment -- tmux hands a new
# pane the SERVER's environment. So the bootstrap's `npx pre-register-codex-pane`
# resolves its daemon from the SERVER's CROSS_AGENT_TEAMS_MCP_HOME, and execs
# whatever `codex` the SERVER's PATH finds. Measured the hard way: with every
# lab value set correctly on the aoe process and nothing on the server, two lab
# panes pre-registered into jt's PRODUCTION daemon.
#
# Asserted by launching a throwaway pane and reading what it actually got, not
# by reading `show-environment` -- the question is what a pane receives, so ask
# a pane.
guard_pane_env() {
  [ -S "$AOE_SOCK" ] || die "no lab tmux server at $AOE_SOCK; run: $0 server-up"
  local out="$LAB/aoe-lab-paneenv.txt"
  rm -f "$out"
  lt new-session -d -s __probe \
    "printenv > $out 2>/dev/null; command -v codex >> $out" 2>/dev/null || true
  local i=0
  while [ ! -s "$out" ] && [ "$i" -lt 40 ]; do sleep 0.1; i=$((i+1)); done
  lt kill-session -t __probe 2>/dev/null || true
  [ -s "$out" ] || die "probe pane produced no environment; is the lab server healthy?"

  local got
  got="$(sed -n 's/^CROSS_AGENT_TEAMS_MCP_HOME=//p' "$out")"
  [ "$got" = "$LAB_HOME_DIR" ] \
    || die "panes would pre-register into '${got:-<unset, i.e. PRODUCTION>}', not $LAB_HOME_DIR -- restart the lab server: $0 server-up"
  got="$(sed -n 's/^CROSS_AGENT_TEAMS_MCP_TOKEN=//p' "$out")"
  [ "$got" = "$LAB_TOKEN" ] || die "panes carry the wrong xats token -- run: $0 server-up"
  got="$(sed -n 's/^CODEX_HOME=//p' "$out")"
  [ "$got" = "$LAB_CODEX_HOME" ] \
    || die "panes carry CODEX_HOME='${got:-<unset>}', not $LAB_CODEX_HOME -- run: $0 server-up"
  got="$(tail -1 "$out")"
  if [ -n "${AOE_LAB_REAL_CODEX:-}" ]; then
    case "$got" in
      "$LAB_BIN"/*) die "a pane's \`codex\` is still the lab stub while AOE_LAB_REAL_CODEX is set" ;;
      "") die "a pane cannot find codex at all" ;;
    esac
  else
    [ "$got" = "$LAB_BIN/codex" ] \
      || die "a pane's \`codex\` resolves to '${got:-<none>}', not the lab stub -- run: $0 server-up"
  fi
}

# Start the private lab tmux server with the lab environment, so every pane aoe
# creates on it inherits the redirections. The keeper session exists only to own
# the server; aoe's own sessions are created later.
cmd_server_up() {
  AOE_SOCK="$LAB_TMUX_TMPDIR/tmux-$(id -u)/$PROFILE"
  if [ -S "$AOE_SOCK" ] && lt has-session -t __keep 2>/dev/null; then
    note "lab tmux server already up at $AOE_SOCK"
    return 0
  fi
  env -u TMUX -u TMUX_PANE \
    HOME="$LAB_AOE_HOME" \
    TMUX_TMPDIR="$LAB_TMUX_TMPDIR" \
    AGENT_OF_EMPIRES_PROFILE="$PROFILE" \
    CODEX_HOME="$LAB_CODEX_HOME" \
    CROSS_AGENT_TEAMS_MCP_HOME="$LAB_HOME_DIR" \
    CROSS_AGENT_TEAMS_MCP_TOKEN="$LAB_TOKEN" \
    CROSS_AGENT_TEAMS_CODEX_WS_URL="$LAB_WS_URL" \
    npm_config_cache="$REAL_NPM_CACHE" \
    PATH="$LAB_BIN:$PATH" \
    tmux -S "$AOE_SOCK" new-session -d -s __keep 'while :; do sleep 3600; done'
  note "lab tmux server up at $AOE_SOCK"
}

# Only ever the fixture's own private socket, by absolute path.
cmd_server_down() {
  AOE_SOCK="$LAB_TMUX_TMPDIR/tmux-$(id -u)/$PROFILE"
  case "$AOE_SOCK" in
    "$LAB"/*) ;;
    *) die "refusing to kill a server outside $LAB" ;;
  esac
  # `default` is only somebody else's socket on the SHARED lab root; on a
  # private root (XATS_LAB_HOME) it is this fixture's own.
  if [ "$PROFILE" = "default" ] && [ "$LAB" = "$HOME/.xats-lab" ]; then
    die "refusing: 'default' on the shared lab root is not this fixture's socket"
  fi
  [ -S "$AOE_SOCK" ] || { note "no server at $AOE_SOCK"; return 0; }
  lt kill-server 2>/dev/null || true
  note "private lab tmux server at $AOE_SOCK torn down"
}

# Run aoe with the lab environment. `env -u TMUX -u TMUX_PANE` matters twice
# over: aoe passes -L (authoritative over $TMUX), but the pane commands it
# launches inherit this environment, and a leaked $TMUX_PANE is what the
# bootstrap would pre-register.
aoe() {
  guard
  env -u TMUX -u TMUX_PANE \
    HOME="$LAB_AOE_HOME" \
    TMUX_TMPDIR="$LAB_TMUX_TMPDIR" \
    AGENT_OF_EMPIRES_PROFILE="$PROFILE" \
    CODEX_HOME="$LAB_CODEX_HOME" \
    CROSS_AGENT_TEAMS_MCP_HOME="$LAB_HOME_DIR" \
    CROSS_AGENT_TEAMS_MCP_TOKEN="$LAB_TOKEN" \
    CROSS_AGENT_TEAMS_CODEX_WS_URL="$LAB_WS_URL" \
    npm_config_cache="$REAL_NPM_CACHE" \
    PATH="$LAB_BIN:$PATH" \
    "$AOE_BIN" "$@"
}

# A stub that is faithful on exactly the two things aoe's capture path reads:
# it writes a rollout file (the claim reads FILES, not processes), and it stays
# alive with "codex" in its argv (the claim's positive evidence is a live
# process tree matching /codex/, db/codex_rollout.rs:166). Anything else about
# real codex is irrelevant here and deliberately absent.
cmd_shim() {
  mkdir -p "$LAB_BIN"
  cat > "$LAB_BIN/codex" <<'STUB'
#!/bin/sh
# Lab stub codex. Not a simulation of codex -- only of what aoe's rollout claim
# reads. Keep the name `codex`: the claim matches the ps line, not the binary.
cwd=""
prev=""
for a in "$@"; do
  [ "$prev" = "-C" ] && cwd="$a"
  prev="$a"
done
[ -n "$cwd" ] || cwd="$PWD"
: "${CODEX_HOME:?lab stub refuses to run without CODEX_HOME}"
# `codex resume <id>` continues an EXISTING conversation: it keeps that thread
# and writes no new rollout. Getting this wrong would make a resumed pane look
# like it started a fresh conversation, which is exactly the symptom the resume
# scenario asserts against -- the stub would manufacture the defect it is
# supposed to rule out.
# Rollout landing order is controllable so the "delayed flush" hypothesis can be
# tested: real codex creates the conversation at launch but only writes the
# rollout file on the first turn, while the FILE NAME keeps the start time. With
# $LAB/stub-mode = "stagger" the FIRST stub to start delays its write, so the
# later-started pane's rollout lands first -- the order the hypothesis is about.
# Any other value writes immediately (the even shape).
stub_delay=0
if [ "$(cat "$CODEX_HOME/../stub-mode" 2>/dev/null)" = "stagger" ]; then
  if mkdir "$CODEX_HOME/../stub-order-1" 2>/dev/null; then stub_delay=6; fi
fi
resume_id=""
prev=""
for a in "$@"; do
  [ "$prev" = "resume" ] && resume_id="$a"
  prev="$a"
done
if [ -n "$resume_id" ]; then
  uuid="$resume_id"
  roll="(resumed, no new rollout)"
else
  # The name carries the START time even when the write is late -- that is the
  # property the hypothesis turns on, so stamp it before sleeping.
  now="$(date +%Y-%m-%dT%H-%M-%S)"
  [ "$stub_delay" -gt 0 ] && sleep "$stub_delay"
  day="$CODEX_HOME/sessions/$(date +%Y/%m/%d)"
  mkdir -p "$day"
  uuid="$(uuidgen | tr 'A-Z' 'a-z')"
  roll="$day/rollout-$now-$uuid.jsonl"
  printf '{"type":"session_meta","payload":{"id":"%s","cwd":"%s"}}\n' "$uuid" "$cwd" > "$roll"
fi
echo "[lab-stub-codex] pane=${TMUX_PANE:-?} thread=$uuid delay=$stub_delay rollout=$roll"
echo "[lab-stub-codex] key=${XATS_IDENTITY_KEY:+<SET>}${XATS_IDENTITY_KEY:-<unset>}"
echo "[lab-stub-codex] argv: $0 $*"
# Stay alive: a dead pane process makes the claim's positive evidence vanish.
while :; do sleep 3600; done
STUB
  chmod +x "$LAB_BIN/codex"

  # Putting $LAB/bin on the tmux server's PATH is not enough: tmux runs a pane
  # command through a LOGIN shell, and the login sequence rebuilds PATH from
  # scratch -- measured, the lab prefix was gone from the pane while still
  # present in the server's global environment. Since the lab overrides HOME,
  # the lab's own shell startup files are the reliable place to put it back.
  # Both files: .zshenv runs before the system profile, .zprofile after it.
  # npx shim. The bootstrap runs `npx --no-install cross-agent-teams-mcp@latest`,
  # which on this machine resolves to the PUBLISHED 0.7.7 -- and 0.7.7 has no
  # --identity-key-env at all, so it drops the key and still answers ok:true.
  # Forwarding to the repo build tests the post-release target state. The
  # today-state (key never reaches the pre-reg row) is a separate result and is
  # NOT superseded by anything measured through this shim.
  # The npx shim is GONE on purpose. It existed only while the published
  # package was 0.7.7 (no --identity-key-env); the cache now resolves
  # cross-agent-teams-mcp@latest to 0.8.0, so the real bootstrap already gets a
  # CLI that carries the key. Re-adding a shim here would hide the real path.
  # Claude stub: claude's capture is via its hook calling \`aoe __record-pane\`,
  # not via a rollout file, so the stub fires that itself. Nothing else about
  # claude is simulated -- the claude-side check only reads the launch command.
  cat > "$LAB_BIN/claude" <<STUB
#!/bin/sh
echo "[lab-stub-claude] pane=\${TMUX_PANE:-?} key=\${XATS_IDENTITY_KEY:+<SET>}\${XATS_IDENTITY_KEY:-<unset>}"
echo "[lab-stub-claude] argv: \$0 \$*"
# aoe captures claude through its hook shelling out to \`aoe __record-pane\`,
# which takes the conversation id from the hook's stdin JSON -- not from argv.
# Feed it the same shape: reuse --session-id when aoe pre-allocated one (the
# primary pane), otherwise mint one the way a fresh claude conversation would.
sid=""
prev=""
for a in "\$@"; do
  [ "\$prev" = "--session-id" ] && sid="\$a"
  prev="\$a"
done
[ -n "\$sid" ] || sid="\$(uuidgen | tr 'A-Z' 'a-z')"
echo "[lab-stub-claude] session_id=\$sid"
printf '{"session_id":"%s","cwd":"%s"}' "\$sid" "\$PWD" \\
  | "$AOE_BIN" __record-pane --agent claude >/dev/null 2>&1 || true
while :; do sleep 3600; done
STUB
  chmod +x "$LAB_BIN/claude"

  mkdir -p "$LAB_AOE_HOME"
  for rc in .zshenv .zprofile; do
    printf 'export PATH="%s:$PATH"\n' "$LAB_BIN" > "$LAB_AOE_HOME/$rc"
  done
  note "stub codex written to $LAB_BIN/codex (+ PATH prefix in $LAB_AOE_HOME/.zshenv,.zprofile)"
}

cmd_env() {
  guard
  cat <<EOF
aoe binary : $AOE_BIN
aoe HOME   : $LAB_AOE_HOME
profile    : $PROFILE  -> tmux socket $AOE_SOCK
project    : $LAB_PROJECT
CODEX_HOME : $LAB_CODEX_HOME
xats home  : $LAB_HOME_DIR (daemon port $LAB_PORT)
PATH head  : $LAB_BIN
EOF
}

case "${1:-env}" in
  shim) cmd_shim ;;
  server-up) cmd_server_up ;;
  server-down) cmd_server_down ;;
  env) cmd_env ;;
  tmux) shift; AOE_SOCK="$LAB_TMUX_TMPDIR/tmux-$(id -u)/$PROFILE"; lt "$@" ;;
  run) shift; aoe "$@" ;;
  *) die "usage: $0 {shim|server-up|server-down|env|tmux <args>|run <aoe args...>}" ;;
esac
