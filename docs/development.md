# Development

## Prerequisites

- macOS 14 or later — the accessibility and event frameworks are macOS only.
- Rust 1.85 or newer (`rust-toolchain` is not pinned; the crate's
  `rust-version` is).
- Xcode Command Line Tools for the frameworks and `screencapture`.
- Nothing else: the release binary is one file.

## Commands

```sh
cargo build                     # debug binary
cargo build --release           # what you install
cargo test                      # unit tests + tests/cli.rs
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo fmt --check
```

```sh
./target/debug/oxide-reins tools
./target/debug/oxide-reins doctor
./target/debug/oxide-reins call list_apps
./target/debug/oxide-reins install --client codex --print
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | ./target/debug/oxide-reins mcp
```

```sh
sh -n install.sh                # the installer parses
./scripts/set-version.sh v0.1.0 # what a release tag stamps into Cargo.toml
```

## What the tests cover

Run in the same binary, so no permissions and no GUI session are needed except
where noted.

| Where | Guards |
| --- | --- |
| `src/mcp.rs` tests | initialize shape (tools and a subscribable resource) and instructions that name every tool and the policy commands, revision echo (all three supported, unknown → newest), the screen resource listed and read by URI with an unknown-URI error, a capture turned into a PNG block, a subscribed client told once on subscribe and again only after an action, unsubscribe stopping that, notifications not answered, unknown method → `-32601`, tool failure → `isError` result, a call that panics answered as an error with the server still serving the next request |
| `src/tools.rs` tests | every tool has a description, an object schema and a read-only hint; every description carries a word an agent actually searches for; observers are read-only and actions are not; names unique; result shape with and without an image; every tool documented in `docs/tools.md` and listed in the README table |
| `src/install.rs` tests | client parsing and aliases, name validation (the names Claude Code and Codex already use are refused), per-client default names, JSON and TOML snippets |
| `src/policy.rs` tests | empty policy allows all, deny beats allow, allow list excludes the rest, a changed bundle identity refused, a one-call approval spent and persisted away, an unreadable file refusing every app, a decision written by another process reaching a running policy, self-automation refused by default, save/load round trip |
| `src/macos/input.rs` tests | key codes for named keys, `\n`/`\r`/`\t` read as `Return`/`Tab`, the pressed key named back, modifier aliases, unknown key refused with the escape hint |
| `src/macos/takeover.rs` tests | each kind of person input named, the tap mask covering buttons, movement, keys and scrolling, only a foreign event source counting as a person |
| `src/macos/apps.rs` tests | the outermost `.app` wins for a nested helper, a binary outside a bundle has no identity, an unreadable bundle has no identifier |
| `src/macos/tree.rs` tests | value truncation with true length, index/indent rendering, multi-line values flattened |
| `src/macos/ax.rs` tests | only a code that means the app did not answer fails a read (an unsupported attribute and a vanished element stay absent, so one node cannot cost the snapshot), and the timeout error names the bound it hit |
| `src/macos/mod.rs` tests | UTF-16 range math (including an emoji), case-insensitive fallback, output cap note, line-break counting in a `type_text` reply, a literal `\n` named in the reply, only the hosting bundle counting as driving itself, lenient argument parsing |
| `src/verify.rs` tests | only a missing check fails the run (skipped does not), every state has a name, a refused permission names the Settings pane |
| `tests/cli.rs` (verify) | `verify --json` names every check, each state is one of three values, and the exit status follows the missing count on a granted and an ungranted machine alike |
| `tests/install.sh` | `install.sh --check` against a staged release directory through `file://`: current exits 0, behind exits 1 and names both versions, no installed copy and no manifest exit 2 |
| `tests/cli.rs` | tool list, MCP handshake with a real subprocess, installers writing every client's file, merge preserving other settings (including Codex's own `[computer_use]` table), reserved-name refusal, policy round trip with `revoke`/`grant-once`/`policy deny`, the self-automation rule and its `doctor` report, an unreadable policy refusing every app, `doctor --json` provenance, `tools --json`, a stdio session that subscribes to the screen resource and receives its notification after an action, UTF-8 through stdio, unknown tool exit code, `--calls-file` error naming the resolved path |

What they cannot cover: anything that needs Accessibility or Screen Recording —
window binding, `set_value` read-back against a real control, `select_text`,
paced drags, `AXPress` versus pointer clicks, window-scoped screenshots, a read
from an app that has stopped answering, and the takeover tap itself (its callback
runs only once the grant exists). Two of those paths need no grant, and `verify`
measures them: the bundle identity of a running app, and the read bound the system
accepts for it. The rest need a trusted terminal; the manual checklist below is
the substitute.

## Provenance

`install.sh` writes `.oxide-reins-install` beside the installed binary:

```text
source jaysonwu991/oxide-reins
version 0.1.0
platform darwin-arm64
sha256 3f2c…
```

`oxide-reins doctor --json` reports that marker as `provenance.install`, together
with `provenance.path` (the binary that is actually running), `provenance.target`
(the build's architecture and OS) and `provenance.profile` (debug or release). A
client that bundles the runtime — the Oxide helper copies it into an app — can
report which build it started and whether it came from a release, without
trusting a version string alone. A binary built in a checkout has no marker, so
`install` is `null`.

## Adding a tool

A tool exists in six places, and `tools::tests` fails if the docs drift:

1. `src/tools.rs` — the schema in `specs()`, with `readonly(...)` or
   `action(...)`, and the dispatch arm in `execute`.
2. `src/runtime.rs` — the method on the non-macOS stub, returning an explicit
   error.
3. `src/macos/mod.rs` — the real implementation on `Runtime`: resolve the app,
   `self.guard()?`, act, verify, and return a sentence that says what happened.
4. `docs/tools.md` — a `### \`name\`` section: parameters, behaviour, failure
   modes, example reply. The test greps for the heading.
5. `README.md` — a `| \`name\` | … |` row in the tool table. The test greps for
   the row.
6. `src/tools.rs` tests — update the expected count, and add behaviour tests for
   pure helpers the tool uses.

Rules that keep the surface honest:

- An action must verify what it changed, or say it could not.
- A refusal must name the fix, including the argument or command.
- Anything that moves the pointer or types must be described as doing so.
- Read-only tools get `readonly(...)`; state-changing ones get `action(...)` with
  `idempotent: true` only when repeating it changes nothing.

## Adding an MCP client

`src/install.rs` holds one enum. Add a variant and fill in:

- `parse` (with aliases), `label`, `format` (`Json` or `Toml`),
  `supports_project`, and `target` (user scope and project scope paths);
- a line in the trailing message that tells the user how to confirm the install;
- `default_name` and `reserved_name` if the client already ships its own
  Computer Use or gives names to its own MCP servers;
- tests: parsing, target paths, and the written file in `tests/cli.rs`.

The writer merges rather than replaces, preserves unrelated keys, and refuses to
rewrite a file it cannot parse.

## Adding a platform backend

`src/runtime.rs` is the seam:

1. define `State { text, image }` (already shared);
2. implement every method the macOS `Runtime` implements, plus `guard`;
3. re-export the backend for the target OS;
4. never return a success-shaped guess — a tool that cannot work must fail with
   a message saying so.

Keep `unsafe` inside the backend. The macOS backend confines it to
`src/macos/` and names the framework call at every `extern` block; a new backend
should do the same. Filter helper surfaces at the enumeration layer, as
`apps.rs` does, so agents see apps rather than window-server internals.

## FFI rules in `src/macos/`

- `cf::Owned` wraps a `+1` Core Foundation reference and releases on drop. Use
  `from_create` for `Create`/`Copy` results and `from_get` for borrowed ones.
  Getting this backwards is a leak or a crash, so match the API's ownership
  convention, not a guess.
- `ax::Element` is the same idea for `AXUIElementRef`; `clone` retains, drop
  releases, and snapshots hold retained elements on purpose.
- Decode attribute values by type (`CFString`, `CFNumber`, `CFBoolean`,
  `CFArray`, `AXValue`) before use; a type mismatch is `None`, never a cast.
- A new `extern` block needs the framework name in its `#[link(...)]` and the
  header's signature verbatim; comment only where the signature is surprising
  (variadic-like APIs, out-parameters, ownership).

## Design rules worth preserving

- **Accessibility first, pointer second** — AX reaches background apps and does
  not steal the pointer. Say which path ran.
- **Policy before permission** — a refusal should be about the app, not about
  setup.
- **No silent success** — check settability, read values back, list supported
  actions, refuse stale indexes by name.
- **Bound every wait** — one accessibility message may wait two seconds for an
  app; a read that times out fails the call and leaves the previous snapshot
  addressable, because an unresponsive app must not look like an empty window. A
  panic inside a call is an `isError` result, not the end of the session.
- **Pace synthetic input** — one unpaced drag event is a bug report.
- **Cap the output and say so** — context is the scarce resource in an agent
  loop.
- **One tool list** — schemas, dispatch, docs and tests come from `specs()`.

## Manual smoke checklist

On a machine where the terminal is trusted (`oxide-reins doctor` reports both
permissions granted):

```sh
oxide-reins call list_apps
oxide-reins call get_app_state --args '{"app":"TextEdit","window":0,"include_frames":true}'
oxide-reins call click --args '{"app":"TextEdit","element_index":6}'
oxide-reins call set_value --args '{"app":"TextEdit","element_index":6,"value":"hello"}'
oxide-reins call select_text --args '{"app":"TextEdit","element_index":6,"text":"hello"}'
oxide-reins call press_key --args '{"app":"TextEdit","key":"c","modifiers":"cmd"}'
oxide-reins call drag --args '{"app":"TextEdit","from_x":300,"from_y":300,"to_x":500,"to_y":300}'
oxide-reins call get_app_state --args '{"app":"TextEdit","include_screenshot":true}'
oxide-reins call type_text --args '{"app":"TextEdit","text":"line one\nline two"}'
oxide-reins call press_key --args '{"key":"\\n"}'
oxide-reins apps list
oxide-reins apps policy deny
oxide-reins call get_app_state --args '{"app":"TextEdit"}'   # refused, naming grant-once and allow
oxide-reins apps grant-once "TextEdit"
oxide-reins call get_app_state --args '{"app":"TextEdit"}'   # allowed once, then the grant is gone
oxide-reins apps revoke "TextEdit"
oxide-reins apps self-automation deny
oxide-reins call type_text --args '{"app":"TextEdit","text":"background","background":true}'
# an app that stops answering must fail the read, not wedge the server
pid=$(pgrep -x TextEdit); kill -STOP "$pid"
oxide-reins call get_app_state --args '{"app":"TextEdit"}'
kill -CONT "$pid"
```

Expected: the click reports AXPress or a pointer event; `set_value` echoes the
value it read back; a read-only control fails instead of claiming success;
`select_text` reports a character range; the drag selects text or moves the
window; the screenshot returns a PNG. `type_text` reports two line breaks and
both lines land in the document; `press_key` with a literal `\n` reports
`Pressed Return (for …)`. `apps list` shows `TextEdit` as blocked under the
deny-by-default policy and as approved for one call after `grant-once`, and the
config no longer holds the grant after the call that used it. Watch the pointer
during the drag and press a key while it runs: the drag should stop and report
`user takeover`. `background: true` types into TextEdit while another app keeps
the keyboard focus and says so in the reply. With self-automation denied, a call
against the hosting app is refused; with it allowed, the same call proceeds. Read
`computer-use://screen` once and then as a subscriber: the read returns a PNG,
and each action is followed by one `notifications/resources/updated`. Then run
the same sequence through `--calls` and confirm the indexes stay valid across
calls. The stopped app is the exception: its `get_app_state` fails with `the app
did not answer within 2 seconds` instead of returning an empty tree, the previous
snapshot's indexes still address what they came from, `oxide-reins tools` keeps
answering from the same process, and the call after `kill -CONT` reads the tree
again.

## Releasing

A release is a draft that a maintainer publishes. Two workflows split the job:

1. `.github/workflows/release-drafter.yml` runs on every push to `main`, and on
   pull request events to apply type labels, keeping one draft release in step
   with the merged pull requests. The notes and the next version come from
   [`.github/release-drafter.yml`](../.github/release-drafter.yml): a `breaking`
   pull request bumps the major, a feature the minor, everything else the patch.
   It never publishes.
2. Publishing that draft — **Releases → the draft → Edit → Publish release** —
   is the release and creates the `v*` tag. `.github/workflows/release.yml` runs
   on that tag:
   - `check` — `cargo fmt --check`, clippy with `-D warnings` and the tests, on
     macOS.
   - `build` — both macOS targets. Each job stamps the tag into `Cargo.toml`
     with `scripts/set-version.sh v<version>` and refreshes `Cargo.lock` with
     `cargo update --workspace`, so the binary reports the release it came from;
     neither change is committed back.
   - `attach` — writes `oxide-reins-manifest` and uploads the two archives,
     their `.sha256` files, the manifest and `install.sh` itself, which makes
     `releases/latest/download/install.sh` address the current release. It does
     not create the release and does not touch the notes: a tag pushed by hand
     has no release behind it, and the job fails saying so rather than inventing
     one.

The first release is seeded by hand, because with no earlier release the drafter
resolves `v0.0.1`: **Actions → Release Drafter → Run workflow** with
`version: v0.1.0`.

`install.sh` reads the release through that manifest, or one pinned with
`OXIDE_REINS_VERSION`. Its failure paths matter more than its happy path: with
no release published it must stop with the manifest message rather than install
half a binary, and a truncated download must fail the checksum.

## Release checklist

1. `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
2. `cargo build --release`
3. Run `doctor` and the smoke checklist above on a trusted machine.
4. `install --client oxide|claude|codex|cursor|desktop` into a throwaway `HOME`
   and confirm each client lists the server under the name the docs give it.
5. Update `README.md`, `docs/tools.md`, `docs/architecture.md` and
   `docs/issues.md` for any behaviour change; the doc test enforces the first
   two for tool drift.
6. Publish the draft release, which is what creates the tag, then wait for
   `release.yml` to go green and read the release page: two archives, two
   checksums, the manifest and `install.sh`. Run the one-liner on a clean Mac
   before announcing it.
7. Write the commands you ran into the commit message.


## Optional Oxide helper checks

The native companion source, build script and read-only smoke test live in the
Oxide repository, not in this server's release archive. See
[the helper integration guide](oxide-helper.md). Its check verifies separate MCP
sessions, host reporting, a relative custom deny policy, normal EOF and abrupt
proxy-death cleanup. It requests no OS grants and sends no GUI input. It requires
a running windowed app for its read-only policy guard check.

Passing that check is not evidence of granted screenshot/input behavior or full
Codex parity. Validate permission dialogs, actual capture/input, user takeover,
helper failure, signing, updates and OS/CPU coverage separately before promoting
the helper into the release workflow. The server's existing tests still apply;
no tools or protocol schemas were added by the wrapper.
