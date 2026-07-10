<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Security Skill

Use when:

* touching authentication, tokens, passphrases, or crypto code
* implementing or modifying SSH agent forwarding
* handling input from users, agents, or API callers
* adding logging, tracing, or debug output anywhere near credentials

The canonical rules are in `AGENTS.md` under "Non-negotiable rules" — always true, regardless of task. This skill gives the concrete checklist for applying them.

## Required

* Passphrases are encrypted at rest using AES-256-GCM. Never store, log, or transmit passphrases in plaintext — at any layer: database, logs, WebSocket messages, error responses.
* Agent tokens must be cryptographically random (32+ bytes), generated with a CSPRNG.
* Never log sensitive data (passphrases, tokens, SSH keys). Use `[REDACTED]` placeholders in debug output. Check new `tracing::debug!`/`Debug` derives for accidental leakage of secret-bearing structs.
* All user-facing input must be validated. Never trust input from agents or API callers without validation.
* SSH agent forwarding relays the server's SSH agent socket to `borg` on agent machines so no SSH private keys need to be distributed to agent machines. The architecture (WebSocket relay at `/ws/ssh-agent/:hostname`, temporary Unix domain sockets, token verification, `SSH_AUTH_SOCK` injection into the borg subprocess) is documented in `docs/ssh-agent-forwarding.md` — read it before modifying the relay endpoint, the temp-socket handling, or the token-verification path.

## Validation checklist

* [ ] No secret value appears in a `Debug`/`Display` impl, log line, span field, or error message
* [ ] New tokens/keys use a CSPRNG, sized 32+ bytes
* [ ] New external input (from users, agents, or API callers) is validated before use
* [ ] Passphrase handling still round-trips through AES-256-GCM at rest
* [ ] If SSH agent forwarding code changed, `docs/ssh-agent-forwarding.md` still accurately describes the behavior — update it if not (see `skills/documentation/SKILL.md`)

## Reference

* `docs/security.md` — authentication mechanisms (session cookies, API tokens, agent tokens), session security properties
* `docs/ssh-agent-forwarding.md` — SSH relay architecture, server/agent setup, borg repository authorization, failure modes, troubleshooting
* `skills/review/SKILL.md` — PRs touching the areas above (or adding a new suppression) are auto-labeled `needs human review` by `.github/workflows/pr-status-labels.yml`; only a human may clear that label
