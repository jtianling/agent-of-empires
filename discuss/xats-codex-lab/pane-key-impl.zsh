#!/usr/bin/env zsh
# Verbatim copy of the two functions from ~/.xats.sh, with exactly ONE change:
# the key root comes from $XATS_KEY_ROOT instead of being hard-coded to
# ~/.config/xats/codex-pane-keys, so the experiment can never read, write or
# rm -rf anything under jt's production key directory.
#
# Invoked INSIDE a lab tmux pane, so `tmux` resolves to that pane's own server
# and $TMUX_PANE is real -- the same context the real function runs in.

_xats-codex-server-id() {
    local id
    id="$(tmux show-options -sv @xats_server_id 2>/dev/null)"
    if [[ -z "$id" ]]; then
        tmux set-option -s -o @xats_server_id "$(uuidgen)" >/dev/null 2>&1
        id="$(tmux show-options -sv @xats_server_id 2>/dev/null)"
    fi
    [[ -n "$id" ]] || return 1
    printf '%s' "$id"
}

_xats-codex-socket-slug() {
    local socket
    socket="$(tmux display-message -p '#{socket_path}' 2>/dev/null)"
    [[ -z "$socket" ]] && socket="${TMUX%%,*}"
    [[ -n "$socket" ]] || return 1
    # Sanitizing alone is not injective (/a_b/c and /a/b_c collapse together),
    # and two live servers sharing a slug would delete each other's keys.
    local digest
    digest="$(printf '%s' "$socket" | shasum | cut -c1-8)" || return 1
    printf '%s' "${socket//\//_}-$digest"
}

_xats-codex-pane-key() {
    local server_id socket_slug
    server_id="$(_xats-codex-server-id)" || return 1
    socket_slug="$(_xats-codex-socket-slug)" || return 1
    local socket_root="${XATS_KEY_ROOT:?experiment must set XATS_KEY_ROOT}/$socket_slug"
    local key_dir="$socket_root/$server_id"
    local key_file="$key_dir/${TMUX_PANE#%}"
    if [[ -f "$key_file" ]]; then
        cat "$key_file"
        return 0
    fi
    mkdir -p "$key_dir" || return 1
    # Only one server can hold a given socket at a time, so any other
    # server_id under this socket belongs to a dead server. Other sockets
    # may well be live -- never touch them.
    local sibling
    for sibling in "$socket_root"/*(/N); do
        [[ "$sibling" != "$key_dir" ]] && rm -rf "$sibling"
    done
    local key
    key="$(uuidgen)" || return 1
    printf '%s' "$key" >| "$key_file"
    chmod 600 "$key_file"
    printf '%s' "$key"
}

case "${1:-key}" in
  key) _xats-codex-pane-key ;;
  server-id) _xats-codex-server-id ;;
  socket-slug) _xats-codex-socket-slug ;;
  # Forces the $TMUX fallback path by making display-message unavailable, to
  # prove the fallback yields the same slug as the primary lookup.
  socket-slug-fallback)
      tmux() { return 1; }
      _xats-codex-socket-slug
      ;;
  *) print -u2 "unknown mode: $1"; exit 2 ;;
esac
