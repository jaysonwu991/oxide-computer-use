# Oxide Desktop native Computer Use helper

Oxide now has an opt-in development companion app, **Oxide Computer Use**, in
its Desktop source tree. The standalone runtime in this repository remains a
Rust stdio MCP server. This repository's installer does not yet ship the helper.

## Build and connect

In an Oxide checkout containing `crates/desktop/computer-use`, use a built copy
of this repository's server:

```sh
cd /path/to/oxide
python3 crates/desktop/computer-use/build.py \
  --server /absolute/path/oxide-computer-use/target/release/oxide-computer-use \
  --output "$PWD/crates/desktop/artifacts/Oxide Computer Use.app"
```

Requires Xcode Command Line Tools. The output must not already exist. The build
copies the supplied runtime into the app and signs both; default signing is
ad-hoc for development. `--identity` selects a Developer ID identity, but does
not notarize or publish an installer. The helper supports macOS 14+; the supplied
runtime can impose a higher minimum. Other OS/CPU versions need validation.

Move the app to its intended stable location, then register its proxy:

```sh
oxide mcp add oxide-computer-use "/absolute/path/Oxide Computer Use.app/Contents/MacOS/oxide-computer-use" --mcp
```

Review the existing entry before replacing it; use a separate `oxide-computer-use-*`
name for a side-by-side trial. Reconnect the MCP server or restart Oxide. Running
`oxide-computer-use install --client oxide` afterward registers the standalone runtime
again; it does not select the helper.

## Permissions, activity and cancellation

The proxy launches a new native app instance through LaunchServices, then
bridges stdio MCP over a private Unix socket. Each connection owns its helper and
runtime. It preserves `HOME`, `OXIDE_COMPUTER_USE_CONFIG` and cwd so custom app policy
is not lost. The native menu-bar cursor identifies a connected session and offers
permission diagnostics, user-initiated grant prompts, and **Stop computer use and
quit**. A connection alone is not evidence that an action is running.

Only the user can grant Accessibility and Screen Recording. Open the helper's
permission menu, approve the grants manually, and check `permissions` through the
actual MCP connection. Grants to Oxide.app or Codex Computer Use do not grant
this helper access. Rebuilding an ad-hoc app or moving it can affect grants;
stable signed distribution remains release work. Observed bundle ancestry is a
diagnostic hint, not a substitute for checking macOS's actual permission entry.

Newer Oxide Desktop builds add a mouse indicator during computer tool calls,
a persistent last-screenshot card, a compact-window dialog and **Stop task**.
The image is a captured snapshot, labeled with its originating app; text-only
accessibility calls do not refresh it. State belongs to the originating chat,
including background runs and restored history.

**Stop task** cancels the running Oxide turn. The helper menu terminates its MCP
runtime and disconnects that session. Proxy death is monitored; native smoke
checks cover EOF and abrupt proxy termination. Cancellation does not undo clicks
or writes already completed. Helper-crash recovery still needs validation.

## Verification and parity limits

From the Oxide checkout:

```sh
python3 crates/desktop/computer-use/check.py "/absolute/path/Oxide Computer Use.app"
node crates/desktop/check-app.mjs
node crates/desktop/check-shell.mjs
```

The native smoke check is read-only, requires a running windowed app for the
policy guard, and needs a context allowing Unix sockets and LaunchServices. It
checks concurrent sessions, protocol/discovery, helper host reporting, custom
relative deny policy, EOF and proxy-death cleanup. It requests no OS grants and
sends no GUI input. Live permission prompts, actual input/capture, visual QA,
other platforms and release paths are not covered by it.

Full Codex parity has **not** been achieved. The runtime still activates apps for
keyboard/pointer events; AXPress and AXValue are its limited background paths.
The helper does not add isolated input, continuous screen streaming, a system-wide
cursor overlay, Codex's per-app Allow/Always allow/revoke settings, its built-in
browser, or automatic unlock/relock behavior. The existing server allow/deny
policy and Oxide's generic tool approvals continue to apply.

See the Oxide checkout's `docs/computer-use-parity.md` for today's three-session
review, evidence, verification results and outstanding behaviors. The
[official Codex/ChatGPT Computer Use documentation](https://learn.chatgpt.com/docs/computer-use)
is the reference for the target product's public behavior; its private runtime
is not part of either repository.
