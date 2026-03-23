## REMOVED Requirements

### Requirement: aoe tmux refresh-bindings subcommand
**Reason**: The `refresh-bindings` subcommand was only called by the nested mode `client-session-changed` hook to dynamically rebind keys when the user switched between managed and non-managed sessions. With nested mode removed, the hook no longer exists and this subcommand has no callers.
**Migration**: No user action needed. This was an internal subcommand not intended for direct user use.
