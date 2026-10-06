---
paths:
  - "backend/**"
  - "crates/**"
  - "desktop/**"
---

# Secure coding

Loads when you read or edit backend, core-crate, or desktop files. The permission model (explicit
`segments.action` strings, national or ARTCC-scoped grants, deny beats allow, the
`RequirePermission<P>` extractor, `SERVER_ADMIN`), the auth model (VATSIM Connect only, API keys
capped to the owner's live access, service accounts, the HMAC-authenticated roster webhook), and
the two origin lists are in `AGENTS.md` § Permissions, § Conventions & gotchas, and § Environment
variables. This file is the review checklist on top of them.

Sources: ported from the general sections of the house secure-coding rule; OIS lessons from #346,
#531, #537, and #577.

## Access control

- **Every handler that mutates state declares `RequirePermission<P>`.** A missing extractor is a
  visible gap in the signature, so look for it in review.
- **The extractor is the floor, not the whole check.** Ownership, "is this request still open",
  and ARTCC scope (`access_repo::permission_scope(...).allows(...)`,
  `backend/src/repos/access.rs:867` and `:1002`) are checked in the handler on top of it.
- **Scope first, then authorize.** Resolve a child record through its parent in the same query
  (`where artcc = $1 and id = $2`), so a foreign id is simply not found. An authorization check
  on the parent passes for an actor who legitimately holds the parent, and the write to someone
  else's child still lands. This is the standard IDOR shape.
- **Widening a read widens every write that resolves through it.** When you relax a lookup to make
  more rows visible, list every handler that resolves a record through the same lookup and
  re-apply the constraint at each write.
- **A background job acting for a user is still that user.** A pass that publishes or mutates on a
  user's behalf checks the acting principal's permission and scope like the handler would (#537).
- **Both halves of an AND need their own test.** When a check combines a role or membership with a
  scope, give each half a fixture where the other half passes (#577); see `test-quality.md`.
- **A rule that preserves access must check the access is still live.** A "keep what they already
  hold" branch that compares only stored fields can re-grant something that was deliberately
  revoked. Condition it on the grant being effective now, and test it against revoked and denied
  rows, not only the default fixture.

## Injection

- **Bind every value.** sqlx queries take values through `.bind(...)`. A `format!` into SQL is
  allowed only for trusted code constants, and the code says so; `backend/src/repos/audit.rs:61-71`
  is the model (an internal column name interpolated, the values bound).
- **No shell-outs built from input.** If a `std::process::Command` is genuinely needed, pass
  arguments as a list, never through a shell string.
- **Allowlist enums and statuses** against the values the newest migration permits; see
  `database-postgres.md`.

## Secrets and tokens

- **Tokens are hashed at rest.** Bearer tokens are looked up by SHA-256
  (`backend/src/repos/access.rs:17` and `:129`). Never store or compare a raw token.
- **Compare MACs in constant time.** The roster webhook verifies with `mac.verify_slice(...)`
  (`backend/src/handlers/webhooks.rs:124-126`). Never `==` on a signature or token.
- **Encrypt stored third-party secrets** with `backend/src/secrets.rs` under `OIS_SECRET_KEY`;
  without the key the feature that needs it is off, not degraded to plaintext.
- **Secrets live in the environment.** Add the variable to `.env.example` with a dev-safe value,
  never a real one. Never commit `.env` or `web/.env.local`.
- **Never log a token, key, webhook secret, or OAuth code.** `tracing` fields included. Log the
  principal id instead.
- **A credential shown once stays shown once.** If an acceptance criterion says nothing persists a
  secret client-side, pin that with a source-scan guard; a grep is not enough (#531, see
  `test-quality.md` § Absence needs a guard).

## Errors

Users see the stable error envelope (`AGENTS.md` § Conventions & gotchas), never a SQL error, a
stack trace, or an internal type name.

## Desktop

- **Grant the narrowest capability set.** Pop-out windows already get a narrower set
  (`desktop/src-tauri/capabilities/popout.json`) than the main window (`default.json`). A new
  permission goes in the narrowest file that needs it, with the reason in the file's description.
- **The CSP is security config.** Adding an origin to `connect-src`, `img-src`, or `script-src`
  in `desktop/src-tauri/tauri.conf.json:27` widens what the webview can reach. Justify each one.
- **The session token is stored only where `desktop/src-tauri/src/auth.rs` stores it**: the OS
  credential store on Windows and Linux, and on macOS, instead, an owner-only (0600) session file
  whose trade-off that module documents. Never in web storage, a URL, or a log.
- **The sign-in loopback origin is return-to only.** `http://127.0.0.1:8765` belongs in
  `OAUTH_RETURN_TO_ORIGINS`, never in `CORS_ALLOWED_ORIGINS` (#346; `AGENTS.md` § Environment
  variables).

## Dependencies

`pnpm audit --audit-level=high` and `cargo deny` run in CI (`.github/workflows/ci.yml:96` and
`:120`, configured by `deny.toml`), not in `just ci`. A new dependency needs a reason in the PR. A high advisory against
an unchanged lockfile is repo-wide: file it once instead of fixing it on an unrelated branch.

## Security checklist for every PR

1. Every new mutating route has `RequirePermission<P>` and its data-dependent scope check.
2. Every child record is resolved through its parent.
3. No `format!` into SQL except a commented trusted constant.
4. No token, key, or secret in logs, responses, fixtures, or `.env.example`.
5. Signatures and tokens compared in constant time, tokens stored hashed.
6. A new permission is in all three places (`AGENTS.md` § Permissions).
7. Desktop: capabilities narrowest, CSP changes justified.
