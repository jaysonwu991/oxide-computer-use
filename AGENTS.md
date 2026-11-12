# oxide-computer-use

Computer Use as a stdio MCP server for any agent on macOS: launchable/running apps, an
app's accessibility tree, and the actions that drive it.

## Layout

- `src/main.rs`: the CLI (`mcp`, `call`, `tools`, `verify`, `doctor`, `install`,
  `apps`).
- `src/mcp.rs`: newline-delimited JSON-RPC over stdio; echoes the client's
  protocol revision when it is one of the three we answer. Publishes the
  `computer-use://screen` resource; a subscriber gets
  `notifications/resources/updated` after any call that could have changed the
  screen.
- `src/tools.rs`: the single source of the tool list. A tool's schema, its
  dispatch arm and its row in the README table are the same tool; adding one
  means changing all three, and `tools::tests` counts them.
- `src/install.rs`: writes each client's config file. JSON is merged with
  `serde_json` and Codex's TOML is edited in place with `toml_edit`, so a user's
  other settings and comments survive.
- `src/permissions.rs`: permission status in the MCP process, observed launching
  app bundle, and user-only setup guidance.
- `src/verify.rs`: the `verify` command — one native check per path the tools
  depend on, with the evidence, so a granted machine can prove them.
- `src/policy.rs`: per-app approvals — allow, deny, allow-once, and the default
  for an app nobody decided about — enforced before any accessibility call. An
  approval can be pinned to a bundle identity, and the file is re-read on every
  call so a decision reaches a running server.
- `src/runtime.rs`: picks the backend. macOS is real; every other platform
  compiles to a stub whose methods return an explicit error, so the tool list
  stays identical.
- `src/macos/`: `ax` (elements, actions, settability, text ranges), `tree`
  (snapshot rendering, per-node window binding), `apps` (window server, bundle
  identity), `input` (paced synthetic events from a private HID source, key and
  text decoding), `takeover` (the listen-only tap that yields to a person),
  `screen` (screenshot), `cf` (Core Foundation bridging).
- `tests/cli.rs`: the binary as an MCP client sees it.
- `install.sh`: the `curl | sh` installer. It reads `oxide-computer-use-manifest` from
  the latest release (or a pinned `OXIDE_COMPUTER_USE_VERSION`), verifies the
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
  `troubleshooting.md`, `development.md`, `comparison.md`, `issues.md` (audit
  against the open-computer-use tracker) and `parity.md` (each Codex Computer Use
  follow-up from the Oxide Desktop review and its status here). A behaviour change updates the relevant
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
- A wait is bounded and a broken call is answered. One accessibility message
  gives an app two seconds (`ax::MESSAGING_TIMEOUT_SECONDS`); a read that misses
  it fails as an error and leaves the newest snapshot addressable, because an
  unresponsive app must not look like an empty window. A panic inside a call is
  an `isError` result (`mcp::contained`), not the end of the session.
- Policy first, permission second. `apps deny` answers before the Accessibility
  check, so a refusal is about the app, not about setup. A refusal names the
  command that changes it, and never prompts.
- A pinned approval is checked against the running app's bundle identity, and a
  one-call approval is spent by the call that uses it. Do not weaken either into
  a name-only match or a file that is read once per process.
- A person's input wins: a paced gesture stops when input arrives from anything
  but this process, and says so.
- `background: true` posts keys to one process (`CGEventPostToPid`) instead of the
  HID tap; it must never activate the app, and its reply must ask for
  verification because an app may ignore delivered keys.
- The hosting bundle is refused unless `apps self-automation allow` says
  otherwise. Compare bundle paths; a missing identity is not the host.
- Observations include the current app-window screenshot by default; later reads
  are diffs, `include_screenshot: false` gives text-only state, and
  `max_output_chars` plus `query` bound large trees.
- Tool failures are `isError` results with a message an agent can act on, not
  JSON-RPC errors. Protocol errors are reserved for bad JSON and unknown
  methods.
- Accessibility and Screen Recording are user permissions. Ask, explain, and
  fail with instructions; never loop on a permission prompt.
- The server registers as `oxide-computer-use` for every client. That name is
  distinct from Claude Code's reserved `computer-use` and Codex's bundled
  `computer-use`, `computer_use`, `node_repl`, `cua_repl` and `codex_apps`
  names. `--name` refuses those reserved names. The Codex install merges one
  `[mcp_servers.oxide-computer-use]` table and never touches `[computer_use]`
  or `~/.codex/computer-use/`.
- A release is drafted, not written by CI: publishing the release-drafter draft
  creates the tag, and `release.yml` only attaches assets to the release that is
  already there. It fails with that reason instead of creating a release from a
  tag that has none, so notes nobody reviewed cannot ship.

## Commands

- `cargo build`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt`
- `./target/debug/oxide-computer-use tools`
- `./target/debug/oxide-computer-use doctor`
- `./target/debug/oxide-computer-use call list_apps`
- `./target/debug/oxide-computer-use install --client codex --print`
- `./target/debug/oxide-computer-use apps list` (and `apps allow|deny|revoke|grant-once|policy|self-automation|terminals|reset`)
- `./target/debug/oxide-computer-use verify --json` (the native matrix; add `--app` and
  `--input` to prove background typing, which writes to that app)
- `printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | ./target/debug/oxide-computer-use mcp`
- `sh -n install.sh && sh tests/install.sh && ./scripts/set-version.sh v0.1.0` (the last rewrites `Cargo.toml`; revert it after)

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
