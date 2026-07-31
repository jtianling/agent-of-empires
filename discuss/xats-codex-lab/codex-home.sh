#!/usr/bin/env bash
# xats codex lab: CODEX_HOME isolation + wrong-daemon guard.
#
# The guard is the point of this script. If a lab codex ever registers against
# the production daemon it silently pollutes jt's real agent table, which is
# harder to notice than a killed tmux session. Every misconnect condition below
# is a hard exit before any codex process starts.
#
# Usage:
#   ./codex-home.sh prep [--with-xats]   create $LAB/codex-home, copy auth, write config.toml
#   ./codex-home.sh check                run the misconnect guard on env + config.toml
#   ./codex-home.sh env                  print the env assignments a lab codex must run with
#   ./codex-home.sh verify-rollout       run one minimal real codex, assert rollout lands in the lab
#   ./codex-home.sh clean                remove $LAB/codex-home
#
# Env:
#   LAB        lab root, default /tmp/xats-lab
#   LAB_TOKEN  lab daemon token, default xatslab-token (must differ from prod)

set -euo pipefail

# Lab root is shared with the xats-side fixture (lab/lab-env.sh defaults to the
# same path); keeping one root avoids two divergent codex-home trees.
LAB="${LAB:-$HOME/.xats-lab}"
XATS_REPO="${XATS_REPO:-$HOME/workspace/cross-agent-teams-mcp}"
CODEX_LAB_HOME="$LAB/codex-home"
LAB_CONFIG="$CODEX_LAB_HOME/config.toml"
MCP_TEMPLATE="$XATS_REPO/lab/codex-config.toml.template"

# Fixed by spec (discuss/xats-codex-lab.md): the lab daemon owns 9199 and nothing
# else is a legal target. Not overridable on purpose.
LAB_PORT=9199
PROD_PORT=9100
PROD_XATS_HOME="$HOME/.cross-agent-teams-mcp"
PROD_CODEX_HOME="$HOME/.codex"

# The lab daemon mints its own token into $LAB/token; read that rather than
# inventing one, or every lab MCP call answers 401.
if [ -z "${LAB_TOKEN:-}" ] && [ -f "$LAB/token" ]; then
  LAB_TOKEN="$(cat "$LAB/token")"
fi
LAB_TOKEN="${LAB_TOKEN:-xatslab-token}"

# Whether a token is one the lab itself minted or defaulted to. Asserted
# positively on purpose: a check that merely excludes the production value has
# to name that value (so it lands in the repo), and it silently stops
# protecting anything the day production rotates its token.
is_lab_token() {
  case "$1" in
    lab-*|xatslab-*) return 0 ;;
    *) return 1 ;;
  esac
}
LAB_DEVICE="jtlab"
MCP_SERVER_NAME="cross-agent-teams-mcp"
TOKEN_ENV_VAR="CROSS_AGENT_TEAMS_MCP_TOKEN"

# This script normally runs from a shell that is itself part of the production
# xats setup, so it inherits production credentials. Capture what came in for
# diagnostics, then overwrite before anything else looks at the environment --
# the same shape as the tmux fixture unsetting $TMUX up front.
INHERITED_TOKEN="${CROSS_AGENT_TEAMS_MCP_TOKEN:-}"
INHERITED_XATS_HOME="${CROSS_AGENT_TEAMS_MCP_HOME:-}"
INHERITED_CODEX_HOME="${CODEX_HOME:-}"
export CODEX_HOME="$CODEX_LAB_HOME"
export CROSS_AGENT_TEAMS_MCP_TOKEN="$LAB_TOKEN"
export CROSS_AGENT_TEAMS_MCP_HOME="$LAB/xats-home"

inherited_flag() {
  case "$1" in
    "$2") echo "yes (overridden)" ;;
    '') echo "unset" ;;
    *) echo "no" ;;
  esac
}

die() {
  printf 'codex-home: %s\n' "$1" >&2
  exit 1
}

lab_paths_ok() {
  case "$LAB" in
    /*) ;;
    *) die "LAB must be an absolute path, got: $LAB" ;;
  esac
  case "$LAB" in
    */) die "LAB must not end with a slash: $LAB" ;;
  esac

  local repo
  repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  case "$LAB/" in
    "$repo"/*) die "LAB must live outside the aoe repo ($repo), got: $LAB" ;;
  esac

  [ "$CODEX_LAB_HOME" != "$PROD_CODEX_HOME" ] ||
    die "lab CODEX_HOME resolves to the production one: $CODEX_LAB_HOME"
}

# Hard-fail guard. Any single hit means a lab codex could reach the production
# daemon or write into jt's real codex home, so nothing gets started.
#
# Liveness of these checks: they compare runtime values against the PROD_*
# constants below, not against strings emitted by the code under test, so they
# cannot be silently voided by a rename upstream. The residual risk is config
# drift -- if production ever moves off port 9100 or off the token value here,
# these constants go stale and the guard passes vacuously. Evaluated and
# deliberately not automated: a production port/token change is loud on its own,
# and a self-test would have to hard-code the same constants it is checking. The
# negative cases were exercised by hand (LAB_TOKEN=xats and a 9100-bearing
# config were both rejected). Revisit if production values start moving.
cmd_check() {
  lab_paths_ok

  [ "$LAB_PORT" = "9199" ] ||
    die "lab port must be 9199, got: $LAB_PORT"
  [ "$LAB_PORT" != "$PROD_PORT" ] ||
    die "lab port collides with the production daemon port $PROD_PORT"

  [ -n "$LAB_TOKEN" ] ||
    die "LAB_TOKEN is empty"
  is_lab_token "$LAB_TOKEN" ||
    die "LAB_TOKEN is not a lab token (expected the lab daemon's own, from $LAB/token, or the xatslab-* default); refusing to start anything"

  # These assert the effective values a lab codex would be started with, after
  # the overwrite above -- that is what actually reaches the process.
  case "$CROSS_AGENT_TEAMS_MCP_HOME" in
    "$PROD_XATS_HOME"|"$PROD_XATS_HOME"/*)
      die "CROSS_AGENT_TEAMS_MCP_HOME points at the production daemon home: $CROSS_AGENT_TEAMS_MCP_HOME"
      ;;
    "$LAB"/*) ;;
    *) die "CROSS_AGENT_TEAMS_MCP_HOME must live under $LAB, got: $CROSS_AGENT_TEAMS_MCP_HOME" ;;
  esac

  [ "$CODEX_HOME" = "$CODEX_LAB_HOME" ] ||
    die "CODEX_HOME is $CODEX_HOME, expected $CODEX_LAB_HOME"

  # Stronger than "is not the production value": the effective token must be
  # the lab's own, so anything this shell inherited is rejected whatever it was.
  [ "$CROSS_AGENT_TEAMS_MCP_TOKEN" = "$LAB_TOKEN" ] ||
    die "$TOKEN_ENV_VAR is not the lab token; the overwrite above did not take"

  # Only effective lines count: the upstream template documents the forbidden
  # port inside a comment ("NEVER 9100"), and a comment cannot point codex
  # anywhere. Grepping the raw file would hard-fail on prose.
  if [ -f "$LAB_CONFIG" ]; then
    local effective
    effective="$(grep -v '^[[:space:]]*#' "$LAB_CONFIG" || true)"
    case "$effective" in
      *"$PROD_PORT"*)
        die "lab config.toml has an effective line referencing the production port $PROD_PORT: $LAB_CONFIG"
        ;;
    esac
    # An unsubstituted placeholder fails silently: codex would trust a directory
    # literally named __LAB_DIR__ and stop on the trust prompt instead.
    case "$effective" in
      *__LAB_*__*) die "lab config.toml still has an unsubstituted placeholder: $LAB_CONFIG" ;;
    esac
    case "$effective" in
      *"mcp_servers.$MCP_SERVER_NAME"*)
        case "$effective" in
          *"127.0.0.1:$LAB_PORT/mcp"*) ;;
          *) die "lab config.toml declares the xats MCP server but not against 127.0.0.1:$LAB_PORT" ;;
        esac
        ;;
    esac
  fi

  printf 'guard\tpass\n'
  printf 'inherited_foreign_token\t%s\n' \
    "$(if [ -z "$INHERITED_TOKEN" ]; then echo "unset"; \
       elif is_lab_token "$INHERITED_TOKEN"; then echo "no"; \
       else echo "yes (overridden)"; fi)"
  printf 'inherited_prod_xats_home\t%s\n' "$(inherited_flag "$INHERITED_XATS_HOME" "$PROD_XATS_HOME")"
  printf 'inherited_prod_codex_home\t%s\n' "$(inherited_flag "$INHERITED_CODEX_HOME" "$PROD_CODEX_HOME")"
  printf 'lab_port\t%s\n' "$LAB_PORT"
  printf 'codex_home\t%s\n' "$CODEX_LAB_HOME"
  printf 'config\t%s\n' "$([ -f "$LAB_CONFIG" ] && echo "$LAB_CONFIG" || echo "absent")"
  printf 'xats_mcp_declared\t%s\n' \
    "$([ -f "$LAB_CONFIG" ] && grep -q "mcp_servers\.$MCP_SERVER_NAME" "$LAB_CONFIG" && echo yes || echo no)"
}

# The xats MCP block is opt-in: until the lab daemon on 9199 is up, declaring it
# would make every lab codex start against a dead endpoint.
#
# The MCP block comes from the xats-side template rather than being hand-rolled
# here: it carries two requirements that fail SILENTLY when missing --
# top-level experimental_use_rmcp_client (without it codex never loads
# streamable-http MCP servers) and type = "streamable-http" (the legacy headers
# form is ignored since codex 0.130 and the daemon answers 401).
write_config() {
  local with_xats="$1"
  {
    printf '# xats codex lab config -- see discuss/xats-codex-lab.md\n'
    printf '# Generated by codex-home.sh. Never reference the production daemon port\n'
    printf '# here; the guard greps this file for it and hard-fails.\n\n'
    if [ "$with_xats" = "yes" ]; then
      # The template already carries its own trust entry, so do not add a
      # second one -- duplicate [projects."..."] tables are a toml error.
      [ -f "$MCP_TEMPLATE" ] || die "xats MCP template not found: $MCP_TEMPLATE"
      sed -e "s/__LAB_PORT__/$LAB_PORT/g" -e "s|__LAB_DIR__|$LAB|g" "$MCP_TEMPLATE"
    else
      # Without this, every real codex stops on the first-run directory-trust
      # prompt and no scenario can proceed unattended.
      printf '[projects."%s"]\n' "$LAB"
      printf 'trust_level = "trusted"\n'
    fi
  } >"$LAB_CONFIG"
}

cmd_prep() {
  local with_xats="no"
  case "${1:-}" in
    --with-xats) with_xats="yes" ;;
    '') ;;
    *) die "unknown flag: $1" ;;
  esac

  lab_paths_ok
  mkdir -p "$CODEX_LAB_HOME"
  chmod 700 "$CODEX_LAB_HOME"

  # Whole-file copy only. The credential contents are never read, logged, or
  # passed through a variable.
  [ -f "$PROD_CODEX_HOME/auth.json" ] ||
    die "no auth.json at $PROD_CODEX_HOME/auth.json; a lab codex cannot authenticate"
  cp "$PROD_CODEX_HOME/auth.json" "$CODEX_LAB_HOME/auth.json"
  chmod 600 "$CODEX_LAB_HOME/auth.json"

  write_config "$with_xats"
  cmd_check
  printf 'auth_json\tcopied (contents not read)\n'
}

cmd_env() {
  cmd_check >/dev/null
  printf 'CODEX_HOME=%s\n' "$CODEX_LAB_HOME"
  printf '%s=%s\n' "$TOKEN_ENV_VAR" "$LAB_TOKEN"
  printf 'CROSS_AGENT_TEAMS_MCP_HOME=%s\n' "$LAB/xats-home"
  printf 'XATS_LAB_PORT=%s\n' "$LAB_PORT"
  printf 'XATS_LAB_DEVICE=%s\n' "$LAB_DEVICE"
}

count_rollouts() {
  find "$1" -type f -name 'rollout-*.jsonl' 2>/dev/null | wc -l | tr -d ' '
}

# Burns a small amount of model quota on purpose: rollout landing can only be
# proven by a real codex run, and rollout files are what aoe uses to bind a pane
# to a conversation.
cmd_verify_rollout() {
  cmd_check >/dev/null
  [ -f "$CODEX_LAB_HOME/auth.json" ] || die "run prep first"

  local prod_before prod_after lab_after
  prod_before="$(count_rollouts "$PROD_CODEX_HOME/sessions")"

  local runner=()
  if command -v timeout >/dev/null; then
    runner=(timeout 120)
  elif command -v gtimeout >/dev/null; then
    runner=(gtimeout 120)
  fi

  set +e
  # Deliberately no --dangerously-bypass-approvals-and-sandbox here: this run only
  # has to prove where the rollout lands. Scenario runs copy the production shape.
  CODEX_HOME="$CODEX_LAB_HOME" "${runner[@]}" codex exec \
    --skip-git-repo-check \
    -C "$LAB" \
    'reply with exactly: lab-ok' >"$LAB/verify-rollout.out" 2>&1
  local rc=$?
  set -e

  prod_after="$(count_rollouts "$PROD_CODEX_HOME/sessions")"
  lab_after="$(count_rollouts "$CODEX_LAB_HOME/sessions")"

  printf 'codex_exit\t%s\n' "$rc"
  printf 'prod_rollouts_before\t%s\n' "$prod_before"
  printf 'prod_rollouts_after\t%s\n' "$prod_after"
  printf 'lab_rollouts\t%s\n' "$lab_after"
  printf 'transcript\t%s\n' "$LAB/verify-rollout.out"

  [ "$prod_before" = "$prod_after" ] ||
    die "production rollout count changed ($prod_before -> $prod_after); CODEX_HOME isolation leaked"
  [ "$lab_after" -ge 1 ] ||
    die "no rollout landed under $CODEX_LAB_HOME/sessions"
  [ "$rc" -eq 0 ] || die "codex exec failed with exit $rc; see $LAB/verify-rollout.out"

  printf 'isolation\tpass\n'
}

cmd_clean() {
  lab_paths_ok
  [ -d "$CODEX_LAB_HOME" ] || {
    printf 'codex-home: nothing at %s\n' "$CODEX_LAB_HOME"
    return 0
  }
  rm -rf "$CODEX_LAB_HOME"
  printf 'codex-home: removed %s\n' "$CODEX_LAB_HOME"
}

main() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    prep) cmd_prep "$@" ;;
    check) cmd_check "$@" ;;
    env) cmd_env "$@" ;;
    verify-rollout) cmd_verify_rollout "$@" ;;
    clean) cmd_clean "$@" ;;
    *)
      printf 'usage: %s {prep [--with-xats]|check|env|verify-rollout|clean}\n' "$(basename "$0")" >&2
      exit 2
      ;;
  esac
}

main "$@"
