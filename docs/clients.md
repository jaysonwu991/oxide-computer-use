# Use it from any MCP client

`oxide-computer-use mcp` is a plain stdio MCP server, so any client that can
launch a local command can use it. `oxide-computer-use install --client <name>`
writes the entry that client reads; `--print` shows it instead of writing.

```sh
oxide-computer-use install --client oxide     # ~/.oxide/mcp.json
oxide-computer-use install --client claude    # ~/.claude.json
oxide-computer-use install --client codex     # ~/.codex/config.toml
oxide-computer-use install --client cursor    # ~/.cursor/mcp.json
oxide-computer-use install --client desktop   # Claude Desktop
oxide-computer-use install --client claude --project   # ./.mcp.json
oxide-computer-use install --client <any> --print      # the entry, unedited
```

The registered name defaults to `oxide-computer-use`, except for the two clients that
already ship their own Computer Use. Claude Code reserves `computer-use` as a
server name and skips a server configured under it, and Codex's bundled
Computer Use plugin owns that name and the `computer_use` namespace, so both
get `oxide-computer-use-local`. `--name` picks another name for any client, and
`--name computer-use` is refused for those two with that reason.

## Oxide

```sh
oxide-computer-use install --client oxide
oxide mcp list          # oxide-computer-use … Connected
```

`oxide mcp add oxide-computer-use oxide-computer-use mcp` is the equivalent when the binary
is on `PATH`. Oxide's approval rules cover tool calls. With this direct stdio
setup, Desktop normally hosts the runtime under Oxide.app; check `permissions`
through that actual MCP connection rather than diagnosing from another terminal.

Newer Oxide Desktop builds show a mouse activity glyph, the last returned
screenshot in the details dock, a compact-window dialog and **Stop task**.
Snapshots restore with the conversation and stay with their originating chat.
They are not a continuous screen mirror.

For a separate native app and permission identity, use the opt-in
[Oxide Computer Use helper](oxide-helper.md). Register its executable with
`--mcp`; `oxide-computer-use install --client oxide` still selects the standalone
runtime and does not install or switch to the helper. Existing macOS grants to
Oxide.app do not grant the helper access. Neither setup provides Codex's isolated
background keyboard/pointer input or its per-app approval/settings interface.

## Claude Code

Docs: <https://docs.claude.com/en/docs/claude-code/mcp>

```sh
claude mcp add --transport stdio oxide-computer-use-local -- /absolute/path/oxide-computer-use mcp
claude mcp list
```

Or write the file and skip the CLI:

```sh
oxide-computer-use install --client claude           # user scope, ~/.claude.json
oxide-computer-use install --client claude --project # project scope, ./.mcp.json
```

Entry shape, for either file:

```json
{
  "mcpServers": {
    "oxide-computer-use-local": {
      "command": "/absolute/path/oxide-computer-use",
      "args": ["mcp"]
    }
  }
}
```

Two details that matter for Claude Code:

- `computer-use` is a reserved server name there; a server registered under it
  is skipped at load time. `--client claude` therefore registers
  `oxide-computer-use-local`, and asks for `--name` if you want something else.
- Tools that only read — `list_apps`, `get_app_state`, `query`, `screenshot` —
  carry `readOnlyHint: true`, so an approval rule can allow them without
  approving the actions too.
- Claude Code's oldest supported revision, `2024-11-05`, is answered with the
  same revision; the server echoes whichever supported version the client asks
  for.

## Codex CLI

Docs: <https://developers.openai.com/codex/mcp>

```sh
codex mcp add oxide-computer-use-local -- /absolute/path/oxide-computer-use mcp
codex mcp list
```

Or let this project edit `~/.codex/config.toml`, preserving the rest of the
file:

```sh
oxide-computer-use install --client codex
```

```toml
[mcp_servers.oxide-computer-use-local]
command = "/absolute/path/oxide-computer-use"
args = ["mcp"]
```

Codex ships Computer Use as a bundled plugin, so this server has to fit around
it rather than replace it:

- The plugin already answers to `computer-use`; a server registered under that
  name, or under `computer_use`, would sit in the same slot. `node_repl`,
  `cua_repl` and `codex_apps` are also names Codex gives its own MCP servers.
  `install --client codex` refuses all of them and defaults to
  `oxide-computer-use-local`.
- The install edits one table, `[mcp_servers.oxide-computer-use-local]`. Everything
  else in `~/.codex/config.toml` — including Codex's `[computer_use]` app-access
  policy and the plugin's own entries — is preserved, comments included.
- Nothing is written into `~/.codex/oxide-computer-use/`, the directory the signed
  Computer Use component installs into.

Both servers can be enabled at once: the plugin's tools live in Codex's
`computer_use` namespace, this server's are namespaced by `oxide-computer-use-local`,
and this project's app policy applies only to its own calls.

## Cursor

```sh
oxide-computer-use install --client cursor
```

Writes `~/.cursor/mcp.json` in the same `mcpServers` shape; `--project` writes
`.cursor/mcp.json` in the repository instead. Restart Cursor and check
Settings → MCP.

## Claude Desktop

```sh
oxide-computer-use install --client desktop
```

Writes `~/Library/Application Support/Claude/claude_desktop_config.json` and
asks for a restart. Desktop sessions can attach the PNG from `screenshot`, so
that tool is most useful there.

## Any other client

`oxide-computer-use install --print` emits the entry; a client that reads a
different file only needs to be pointed at it. The server needs nothing but a
command, arguments and a working `HOME`.

## Permissions and approvals

Two gates sit in front of every action:

1. **The client's approval prompt** — Oxide, Claude Code, Codex and Cursor each
   ask before a tool runs, per their own rules.
2. **The app policy** — `oxide-computer-use apps deny 1Password` (or
   `apps allow` to start an allow list) is enforced inside the server, before
   the accessibility call, and answers with the command that changes it.

macOS itself grants Accessibility to the process, not to the app being driven,
so the policy list is the per-app control; there is no OS prompt per target app.
