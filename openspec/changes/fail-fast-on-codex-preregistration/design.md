## Context

The bootstrap's pre-registration section currently reads, in shape:

```sh
pre_register_failed=
if [ -n "${XATS_IDENTITY_KEY:-}" ]; then
    npx ... --identity-key-env XATS_IDENTITY_KEY --ttl 600 || pre_register_failed=1
else
    npx ... --ttl 600 || pre_register_failed=1
fi
if [ -n "${pre_register_failed:-}" ]; then
    if ! npx ... ; then          # no key, no ttl
        printf '[xats] Failed to pre-register the Codex pane.' >&2
        exit 1
    fi
fi
```

The two branches at the top are not the retry: they are the keyed and unkeyed forms of the *first* attempt, and the unkeyed one is legitimate -- a pane with no identity key yet (an extra pane on its first launch) has nothing to pass. The retry is the inner call, which drops the key even when the pane had one.

## Goals / Non-Goals

**Goals:**

- A pre-registration that fails stops the pane, with its diagnostic, rather than producing a keyless registration.

**Non-Goals:**

- Changing the diagnostic's text. It is accurate for the new behavior, and naming the likely causes (a stale `npx --no-install` cache, a daemon refusal) is a separate change with its own reasoning.
- Changing the keyed/unkeyed split at the top. A pane legitimately without a key still pre-registers without one.
- Adding a retry of any other shape. A retry that repeated the same call would fail the same way; see the proposal.

## Decisions

### 1. Delete the retry rather than make it keyed

A keyed retry would repeat the call that just failed, with the same arguments, against the same daemon. Nothing about the failure modes observed -- npx cannot resolve the package, or the daemon refuses the write -- is transient in a way a second immediate attempt would clear.

The next reconcile is not a recovery path here either: pre-registration happens once, in the pane's own shell, before `exec codex`. There is no later actor to retry it.

### 2. The unkeyed first attempt stays

Dropping it would refuse to start any pane that has no identity key yet, which is every extra pane on its first launch -- a documented, converging state, not an error. The change is about not *discarding* a key that exists, not about requiring one.

## Risks / Trade-offs

- **A pane that would previously have started now does not.** -> Intended. The alternative is a pane that starts and is silently outside Cross Agent Team forever; the user cannot see the difference at launch, and the symptom appears much later as "it never re-registers".
- **A transient daemon hiccup during pre-registration now stops the pane.** -> Accepted. The user can restart the pane, which is a visible, recoverable action, unlike the keyless state it replaces. If such hiccups turn out to be common in practice, the answer is a bounded retry of the *same* keyed call, which is a different change from the one being removed here.
