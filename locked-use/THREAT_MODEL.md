# Locked-use threat model

Status: design only. This directory is not an installable macOS authorization
component and does not unlock a Mac.

## Assets

- the macOS login session and local console;
- credentials, keychains and signed-in application state;
- screen contents and accessibility trees visible after unlock;
- the integrity of the Authorization Plug-in and its update path;
- the trust relationship between a remote Codex/Computer Use turn and this Mac.

## Adversaries

- a local process that tries to reuse the locked path as a general unlock API;
- a remote attacker who steals or replays a Computer Use turn identifier;
- malicious or corrupted software running as the logged-in user;
- a modified or substituted authorization bundle;
- a person physically present at the Mac before or during use;
- a crash, update or process restart while the display is unlocked.

## Required controls

1. A stable Developer ID identity, hardened runtime, notarization and a signed
   installer. Quarantine removal is not a trust mechanism.
2. A short-lived grant created only for an active trusted remote turn. The grant
   is bound to turn id, process identity, Team ID, bundle id and bundle path.
3. At most 30 seconds between trusted-turn issue and Authorization Plug-in use.
4. Every display covered before authorization begins.
5. Local keyboard or pointer input immediately relocks and pauses automatic
   unlock.
6. No password, credential, private key or general-purpose `unlock()` API is
   accepted by the library.
7. Any missing evidence, parse failure, timeout or backend error is a denial.
8. The privileged process must relock on crash, disconnect, cancel, signature
   change or update.
9. Audit only non-secret metadata: turn id hash, decision, reason, timestamps,
   identities and process generation.

## Explicit non-goals

- remote password entry;
- administrator authentication;
- unlocking after a signature, Team ID or bundle-path change;
- keeping the Mac unlocked while no trusted turn is active;
- bypassing macOS lock-screen or privacy protections.

See [README.md](README.md) and [src/lib.rs](src/lib.rs) for the fail-closed
decision logic and integration boundary.

