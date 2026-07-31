#!/usr/bin/env bash
# xats codex lab: faithful replica of the aoe managed-pane bootstrap.
#
# Source of truth: src/session/instance.rs::codex_xats_bootstrap_command (read
# directly, not from the prose summary). Step order, the two uuid format checks,
# the `pre_register_failed=` initialisation and the bare-call downgrade are all
# reproduced verbatim in shape.
#
# Deliberate lab deviations (everything else must stay identical):
#   1. CLI: `npx --no-install cross-agent-teams-mcp@latest` -> `node $XATS_REPO/dist/cli.js`
#      (production hits the published 0.7.7, the lab must exercise repo code)
#   2. pre-register always carries explicit --port/--token, so it can never fall
#      back to pid-file discovery and hit the production daemon
#   3. reachability probe: production probes the aoe app-server on 8799. The lab
#      must probe the LAB app-server (8899) to preserve that semantics. Until
#      that app-server exists the probe temporarily points at the lab daemon
#      (9199) -- set LAB_PROBE_PORT=8899 to switch back once it is up. Probing
#      the daemon is NOT equivalent: it hides "app-server is down" failures.
#   4. exec drops --remote for the same reason
#
# Usage:
#   ./lab-bootstrap.sh render [opts]      print the sh script that would run
#   ./lab-bootstrap.sh check-key-leak     assert the identity key VALUE never reaches argv
#   ./lab-bootstrap.sh dry-run [opts]     run against a fake pre-register CLI + fake codex
#   ./lab-bootstrap.sh run [opts]         real run (needs the lab daemon on 9199)
#
# Options:
#   --ttl N        pre-register TTL in seconds (default 600; use 5-10 for S1 expiry races)
#   --pane ID      tmux pane id (default $TMUX_PANE)
#   --no-key       run the branch where XATS_IDENTITY_KEY is empty (S1b shape)
#   --legacy-cli   fake CLI rejects --identity-key-env, exercising the downgrade path

set -euo pipefail

LAB="${LAB:-$HOME/.xats-lab}"
XATS_REPO="${XATS_REPO:-$HOME/workspace/cross-agent-teams-mcp}"
if [ -z "${LAB_TOKEN:-}" ] && [ -f "$LAB/token" ]; then
  LAB_TOKEN="$(cat "$LAB/token")"
fi
LAB_TOKEN="${LAB_TOKEN:-xatslab-token}"
LAB_PORT=9199
# Production probes the aoe app-server (8799) for reachability. The lab must
# probe its OWN app-server (8899) to keep that semantics; 9199 is a TEMPORARY
# stand-in until the lab app-server exists, tracked with aoe-main. Probing the
# daemon instead of the app-server would mask "app-server is down" failures.
LAB_APPSERVER_PORT="${XATS_LAB_APPSERVER_PORT:-8899}"
PROBE_PORT="${LAB_PROBE_PORT:-$LAB_APPSERVER_PORT}"
PROD_PORT=9100
PROD_PACKAGE="cross-agent-teams-mcp@latest"

TTL=600
# Inheriting $TMUX_PANE blindly is wrong: this script normally runs from a pane
# on the PRODUCTION tmux server, and that pane id would then be pre-registered
# into the lab DB, binding a lab identity to one of jt's real panes. Only accept
# the ambient pane when $TMUX actually points at the lab server.
lab_pane_default() {
  case "${TMUX:-}" in
    "$LAB/tmuxtmp"/*) printf '%s' "${TMUX_PANE:-}" ;;
    *) printf '' ;;
  esac
}
PANE="$(lab_pane_default)"
USE_KEY="yes"
LEGACY_CLI="no"
FAKE="no"

CALL_LOG="$LAB/prereg-calls.log"
FAKE_CLI="$LAB/bin/fake-prereg-cli.js"

die() {
  printf 'lab-bootstrap: %s\n' "$1" >&2
  exit 1
}

parse_opts() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --ttl)
        TTL="${2:-}"
        shift 2
        case "$TTL" in
          ''|*[!0-9]*) die "--ttl needs a positive integer" ;;
        esac
        ;;
      --pane)
        PANE="${2:-}"
        shift 2
        ;;
      --no-key)
        USE_KEY="no"
        shift
        ;;
      --legacy-cli)
        LEGACY_CLI="yes"
        shift
        ;;
      *) die "unknown option: $1" ;;
    esac
  done
}

guard() {
  local home_script
  home_script="$(dirname "${BASH_SOURCE[0]}")/codex-home.sh"
  [ -x "$home_script" ] || die "codex-home.sh not found next to this script"
  "$home_script" check >/dev/null ||
    die "codex-home guard rejected the lab; refusing to build a bootstrap command"
  case "$LAB_TOKEN" in
    lab-*|xatslab-*) ;;
    *) die "LAB_TOKEN is not a lab token; refusing to build a bootstrap command" ;;
  esac
}

cli_invocation() {
  if [ "$FAKE" = "yes" ]; then
    printf 'node %s' "$FAKE_CLI"
  else
    [ -f "$XATS_REPO/dist/cli.js" ] || die "no CLI at $XATS_REPO/dist/cli.js"
    printf 'node %s/dist/cli.js' "$XATS_REPO"
  fi
}

codex_invocation() {
  if [ "$FAKE" = "yes" ]; then
    # Records the argv it was launched with instead of starting a real codex.
    printf 'node %s/bin/fake-codex.js -C %s -c "xats.agent_id=\\"$xats_agent_id\\""' "$LAB" "$LAB"
  else
    # CODEX_HOME must be carried onto the exec line itself: without it the real
    # codex started here reads and writes the PRODUCTION ~/.codex (auth,
    # sessions, rollouts), which is exactly the isolation the lab exists for.
    # --remote is NOT optional, despite the lab running its own codex CLI: the
    # daemon identifies a codex carrier by matching /codex\s+.*--remote/ against
    # the pane's argv (auto-bind-codex-pane.ts:135). Dropping it makes the
    # process invisible to pane claiming -- measured: matches=0. It points at
    # the LAB app-server (8899), never the production one.
    #
    # --dangerously-bypass-approvals-and-sandbox is OPT-IN (LAB_BYPASS=yes) and
    # off by default, because production panes do not carry it and it is not a
    # neutral convenience: with it set, `shell_environment_policy` stops being
    # applied at all -- measured, `inherit = "core"` and `inherit = "none"` both
    # handed the tool the full environment. A scenario that inherits it silently
    # runs in a WIDER environment than production, so any conclusion of the form
    # "the model can/cannot see X" taken under it does not transfer.
    printf 'env CODEX_HOME=%s CROSS_AGENT_TEAMS_MCP_TOKEN=%s codex --remote ws://127.0.0.1:%s -C %s -c "xats.agent_id=\\"$xats_agent_id\\""%s' \
      "$(printf '%q' "$LAB/codex-home")" \
      "$(printf '%q' "$LAB_TOKEN")" \
      "$LAB_APPSERVER_PORT" \
      "$LAB" \
      "$([ "${LAB_BYPASS:-no}" = "yes" ] && printf ' --dangerously-bypass-approvals-and-sandbox')"
  fi
}

# Mirrors codex_xats_bootstrap_command step for step. The identity key is passed
# by VARIABLE NAME only (--identity-key-env XATS_IDENTITY_KEY); its value never
# appears in any argv, which is visible to every process on the machine.
render_script() {
  local cli probe_port
  cli="$(cli_invocation)"
  probe_port="$PROBE_PORT"

  cat <<EOF
if [ -z "\${TMUX_PANE:-}" ]; then
    printf '%s\\n' '[xats] TMUX_PANE is empty; refusing to bootstrap.' >&2
    exit 1
fi
if ! command -v uuidgen >/dev/null 2>&1; then
    printf '%s\\n' '[xats] uuidgen is unavailable.' >&2
    exit 1
fi
if ! command -v nc >/dev/null 2>&1; then
    printf '%s\\n' '[xats] nc is unavailable.' >&2
    exit 1
fi
xats_agent_id="\$(uuidgen)" || {
    printf '%s\\n' '[xats] Failed to generate a Codex agent UUID.' >&2
    exit 1
}
case "\$xats_agent_id" in
    ????????-????-????-????-????????????) ;;
    *)
        printf '%s\\n' '[xats] Generated an invalid Codex agent UUID.' >&2
        exit 1
        ;;
esac
case "\$xats_agent_id" in
    *[!0-9A-Fa-f-]*)
        printf '%s\\n' '[xats] Generated an invalid Codex agent UUID.' >&2
        exit 1
        ;;
    *) ;;
esac
if ! nc -z 127.0.0.1 $probe_port >/dev/null 2>&1; then
    printf '%s\\n' '[xats] Lab daemon on $probe_port is unreachable.' >&2
    exit 1
fi
pre_register_failed=;
if [ -n "\${XATS_IDENTITY_KEY:-}" ]; then
    $cli pre-register-codex-pane \\
        --pane "\$TMUX_PANE" --agent-id "\$xats_agent_id" \\
        --identity-key-env XATS_IDENTITY_KEY --ttl $TTL \\
        --port $LAB_PORT --token $LAB_TOKEN \\
        || pre_register_failed=1
else
    $cli pre-register-codex-pane \\
        --pane "\$TMUX_PANE" --agent-id "\$xats_agent_id" \\
        --ttl $TTL \\
        --port $LAB_PORT --token $LAB_TOKEN \\
        || pre_register_failed=1
fi
if [ -n "\${pre_register_failed:-}" ]; then
    if ! $cli pre-register-codex-pane \\
        --pane "\$TMUX_PANE" --agent-id "\$xats_agent_id" \\
        --port $LAB_PORT --token $LAB_TOKEN; then
        printf '%s\\n' '[xats] Failed to pre-register the Codex pane.' >&2
        exit 1
    fi
fi
exec $(codex_invocation)
EOF
}

install_fakes() {
  mkdir -p "$LAB/bin"
  cat >"$FAKE_CLI" <<'EOF'
const fs = require("fs");
const log = process.env.LAB_CALL_LOG;
const argv = process.argv.slice(2);
fs.appendFileSync(log, JSON.stringify({ argv, env_key_set: !!process.env.XATS_IDENTITY_KEY }) + "\n");
if (process.env.LAB_LEGACY_CLI === "1" && argv.includes("--identity-key-env")) {
  process.stderr.write("error: unknown option '--identity-key-env'\n");
  process.exit(2);
}
process.exit(0);
EOF
  cat >"$LAB/bin/fake-codex.js" <<'EOF'
const fs = require("fs");
fs.appendFileSync(process.env.LAB_CALL_LOG,
  JSON.stringify({ codex_argv: process.argv.slice(2) }) + "\n");
setInterval(function () {}, 1 << 30);
EOF
}

cmd_render() {
  parse_opts "$@"
  guard
  render_script
}

# The one security property that must hold no matter what: the key VALUE stays
# out of argv. Checked against the rendered script and against the argv the CLI
# actually observed during a fake run.
cmd_check_key_leak() {
  parse_opts "$@"
  guard
  # This gate only inspects argv, so it must not depend on the app-server being
  # reachable -- probe the daemon instead. Otherwise a stopped app-server makes
  # the gate unprovable and blocks every scenario that (correctly) refuses to
  # run without it.
  PROBE_PORT="$LAB_PORT"
  local sentinel="SENTINEL-KEY-VALUE-DO-NOT-LEAK"
  local script
  script="$(render_script)"

  if printf '%s' "$script" | grep -q "$sentinel"; then
    die "sentinel leaked into the rendered script"
  fi
  printf 'rendered_script_contains_key_value\tno\n'

  FAKE=yes
  install_fakes
  : >"$CALL_LOG"
  script="$(render_script)"
  TMUX_PANE="${PANE:-%99}" \
    XATS_IDENTITY_KEY="$sentinel" \
    LAB_CALL_LOG="$CALL_LOG" \
    sh -c "$script" >/dev/null 2>&1 &
  local pid=$!
  local waited=0
  while [ ! -s "$CALL_LOG" ] && [ "$waited" -lt 50 ]; do
    sleep 0.1
    waited=$((waited + 1))
  done
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  pkill -f 'fake-codex.js' 2>/dev/null || true

  [ -s "$CALL_LOG" ] || die "fake CLI was never invoked; cannot prove anything"
  if grep -q "$sentinel" "$CALL_LOG"; then
    die "sentinel leaked into the observed argv: $CALL_LOG"
  fi
  printf 'observed_argv_contains_key_value\tno\n'
  printf 'observed_argv_carries_env_var_name\t%s\n' \
    "$(grep -q -- '--identity-key-env' "$CALL_LOG" && echo yes || echo no)"
  printf 'cli_saw_key_in_env\t%s\n' \
    "$(grep -q '"env_key_set":true' "$CALL_LOG" && echo yes || echo no)"
  printf 'key_leak\tnone\n'
}

cmd_dry_run() {
  parse_opts "$@"
  guard
  FAKE=yes
  install_fakes
  : >"$CALL_LOG"

  local script legacy=0
  [ "$LEGACY_CLI" = "yes" ] && legacy=1
  script="$(render_script)"

  local key_env=()
  if [ "$USE_KEY" = "yes" ]; then
    key_env=(XATS_IDENTITY_KEY="lab-identity-key-$$")
  else
    key_env=(XATS_IDENTITY_KEY=)
  fi

  set +e
  env "${key_env[@]}" \
    TMUX_PANE="${PANE:-%99}" \
    LAB_CALL_LOG="$CALL_LOG" \
    LAB_LEGACY_CLI="$legacy" \
    sh -c "$script" >"$LAB/dry-run.out" 2>&1 &
  local pid=$!
  set -e
  local waited=0
  while [ "$waited" -lt 50 ]; do
    grep -q 'codex_argv' "$CALL_LOG" 2>/dev/null && break
    sleep 0.1
    waited=$((waited + 1))
  done
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  pkill -f 'fake-codex.js' 2>/dev/null || true

  printf 'ttl\t%s\n' "$TTL"
  printf 'key_branch\t%s\n' "$([ "$USE_KEY" = yes ] && echo with-key || echo no-key)"
  printf 'legacy_cli\t%s\n' "$LEGACY_CLI"
  printf 'prereg_calls\t%s\n' "$(grep -c '"argv"' "$CALL_LOG" 2>/dev/null || echo 0)"
  printf 'reached_codex\t%s\n' "$(grep -q 'codex_argv' "$CALL_LOG" && echo yes || echo no)"
  printf 'call_log\t%s\n' "$CALL_LOG"
}

cmd_run() {
  parse_opts "$@"
  guard
  [ -n "$PANE" ] || die "no pane id: run inside a lab tmux pane or pass --pane"
  local script
  script="$(render_script)"
  TMUX_PANE="$PANE" exec sh -c "$script"
}

main() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    render) cmd_render "$@" ;;
    check-key-leak) cmd_check_key_leak "$@" ;;
    dry-run) cmd_dry_run "$@" ;;
    run) cmd_run "$@" ;;
    *)
      printf 'usage: %s {render|check-key-leak|dry-run|run} [--ttl N] [--pane ID] [--no-key] [--legacy-cli]\n' \
        "$(basename "$0")" >&2
      exit 2
      ;;
  esac
}

main "$@"
