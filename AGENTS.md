# computer-use

Computer Use as a stdio MCP server for any agent on macOS: the running apps, an
app's accessibility tree, and the actions that drive it.

## Layout

- `src/main.rs`: the CLI (`mcp`, `call`, `tools`, `doctor`, `install`, `apps`).
- `src/mcp.rs`: newline-delimited JSON-RPC over stdio; echoes the client's
  protocol revision when it is one of the three we answer.
- `src/tools.rs`: the single source of the tool list. A tool's schema, its
  dispatch arm and its row in the README table are the same tool; adding one
  means changing all three, and `tools::tests` counts them.
- `src/install.rs`: writes each client's config file. JSON is merged with
  `serde_json` and Codex's TOML is edited in place with `toml_edit`, so a user's
  other settings and comments survive.
- `src/permissions.rs`: permission status in the MCP process, observed launching
  app bundle, and user-only setup guidance.
- `src/policy.rs`: the app allow/deny list, enforced before any accessibility
  call. A deny rule beats an allow list.
- `src/runtime.rs`: picks the backend. macOS is real; every other platform
  compiles to a stub whose methods return an explicit error, so the tool list
  stays identical.
- `src/macos/`: `ax` (elements, actions, settability, text ranges), `tree`
  (snapshot rendering, per-node window binding), `apps` (window server),
  `input` (paced synthetic events from a private HID source), `screen`
  (screenshot), `cf` (Core Foundation bridging).
- `tests/cli.rs`: the binary as an MCP client sees it.
- `install.sh`: the `curl | sh` installer. It reads `computer-use-manifest` from
  the latest release (or a pinned `COMPUTER_USE_VERSION`), verifies the
  archive's SHA-256, and installs into `~/.local/bin`.
- `scripts/set-version.sh`: stamps a release tag into `Cargo.toml`; the release
  workflow calls it and never commits the result.
- `.github/workflows/`: `ci.yml` for the branch, `release-drafter.yml` for
  `main` and pull requests (it keeps one draft release and applies type labels),
  and `release.yml` for the `v*` tag that publishing the draft creates: check,
  build both macOS targets, then attach the archives, checksums, the manifest and
  `install.sh` to that release. `.github/release-drafter.yml` holds the notes
  categories, the version rules and the autolabeler.
- `docs/`: `install.md`, `tools.md` (the tool reference a test checks against
  `specs()`), `architecture.md`, `configuration.md`, `clients.md`,
  `troubleshooting.md`, `development.md`, `comparison.md`, and `issues.md` (audit
  against the open-computer-use tracker). A behaviour change updates the relevant
  page in the same commit; a new or renamed tool updates `docs/tools.md` and the
  README table, which `tools::tests` enforces.

## Contracts to keep

- Element indexes come from the newest snapshot, are per process, are bound to
  the `AXWindow` they were captured in, and are refused when the app, window or
  range does not match. Never act on a stale index by guessing.
- An action reports the path it took (accessibility action versus pointer
  event), because the two behave differently for a background app.
- A write is verified: `set_value` refuses a non-settable element and reads the
  value back. Nothing reports success it did not observe.
- Policy first, permission second. `apps deny` answers before the Accessibility
  check, so a refusal is about the app, not about setup.
- Observations are cheap by default: no screenshot unless asked, an output cap
  that says it truncated, and `query` for large trees.
- Tool failures are `isError` results with a message an agent can act on, not
  JSON-RPC errors. Protocol errors are reserved for bad JSON and unknown
  methods.
- Accessibility and Screen Recording are user permissions. Ask, explain, and
  fail with instructions; never loop on a permission prompt.
- The server registers as `computer-use`, except where a client already ships
  Computer Use: Claude Code reserves that exact server name, and Codex's
  bundled Computer Use plugin owns it along with `computer_use`, `node_repl`,
  `cua_repl` and `codex_apps`. `install --client claude|codex` defaults to
  `computer-use-local`, and `--name` refuses the names those clients already
  use. The Codex install merges one `[mcp_servers.<name>]` table and never
  touches `[computer_use]` or `~/.codex/computer-use/`.
- A release is drafted, not written by CI: publishing the release-drafter draft
  creates the tag, and `release.yml` only attaches assets to the release that is
  already there. It fails with that reason instead of creating a release from a
  tag that has none, so notes nobody reviewed cannot ship.

## Commands

- `cargo build`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt`
- `./target/debug/computer-use tools`
- `./target/debug/computer-use doctor`
- `./target/debug/computer-use call list_apps`
- `./target/debug/computer-use install --client codex --print`
- `./target/debug/computer-use apps list`
- `printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | ./target/debug/computer-use mcp`
- `sh -n install.sh && ./scripts/set-version.sh v0.1.0` (the second rewrites `Cargo.toml`; revert it after)

## Conventions

- Rust 2021, 4-space indent, default rustfmt, `anyhow::Result` with
  `.context(...)` for human-readable failures.
- No comments that restate the code; a comment explains an FFI signature, a
  platform quirk or an intent that is not obvious from the call.
- All `unsafe` is confined to `src/macos/`, where it names the framework call it
  wraps. Nothing unsafe leaks into `tools.rs`, `install.rs`, `policy.rs` or the
  CLI.
- Keep the binary dependency-light: standard library, `serde`, `serde_json`,
  `anyhow`, `base64`, `clap`, `toml_edit`, and `core-foundation-sys` on macOS.
- A new platform is a new `runtime` backend plus the same tool list; a backend
  that cannot implement a tool must return an error, never a success-shaped
  guess.
