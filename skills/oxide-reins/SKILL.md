---
name: oxide-reins
description: Drive this Mac's apps through the oxide-reins MCP server — list apps, read an accessibility tree, then click, type, select, scroll or drag. Use when a task needs a GUI app the agent cannot reach through files or a CLI.
---

# Computer Use

The `oxide-reins` MCP server exposes the desktop as tools. Look first and
act on what you saw; acting moves the real pointer and keyboard.
`oxide-reins install` registers the same server name for Oxide, Codex and
Claude Code, beside their built-in Computer Use servers.

## Loop

1. Use `permissions` for setup problems: it checks this MCP process without prompting and identifies the hosting app. In Oxide Desktop grant Oxide.app, not an unrelated terminal.
2. `list_apps` — find the app's bundle id, display name, path or pid. Add
   `{"include_helpers": true}` when a native file picker or another
   window-server surface seems missing.
3. `get_app_state` with that `app` — launches the app if needed, returns the
   indexed tree plus the app-window screenshot, and uses a diff after the first
   view. Pass `window` (an index or title substring) when there are several
   windows; pass `disableDiff: true` for a full tree.
4. Act with `element_index`, or `x`/`y` relative to the returned app-window
   screenshot.
5. Read again after the action. Indexes belong to the newest snapshot and to the
   window they came from; a refusal means the state moved, not that the tool is
   broken.

## Choosing a tool

- `click` uses AXPress for one plain left click when available. It also supports
  `mouse_button`, `click_count`, a held modifier `key`, and `duration`; the
  reply says which path ran.
- `set_value` is the reliable way to fill a text field, and it fails loudly when
  a control is read-only or ignores the write. `type_text` needs the control
  focused and the app frontmost.
- `paste` inserts `text`, `md` or `html` and restores the previous clipboard;
  prefer it for formatted or multiline content.
- `select_text` marks a span with optional `prefix`/`suffix`, or places the
  cursor with `cursor_before` / `cursor_after`.
- `press_key` uses xdotool-style names and chords, such as `Super+s`,
  `Control_L+a` or `KP_0`, plus an optional `duration`.
- `query` when a tree is large: filter by text or role, and take the index from
  its output, which also refreshes the snapshot.
- `perform_secondary_action` for AX actions such as `AXShowMenu` on a row; an
  unsupported action comes back with the list the element does support.
- `get_app_state` includes the window screenshot by default. Set
  `include_screenshot: false` only when a text-only observation is enough.

## Keeping context small

- `max_output_chars` caps a reply; the truncation note says so.
- `query` replaces a full tree dump when you know what you are looking for.
- `text_limit: "max"` is for the one field you must read in full; the default
  truncates long values with their true length.
- App state is a diff after the first observation. Use `disableDiff: true` when
  an index shift is unclear. Re-observe after changes and leave submission to
  the user when requested. Pointer tools use the foreground; `type_text` and
  `press_key` with `background: true` post to one app without activating it, and
  the reply asks for verification because an app may ignore them.

## Failures worth reading

- `` `X` is on the deny list ``, `` not on the allow list `` or `` is not
  approved `` — the local app policy. `oxide-reins apps allow "X"` approves from
  now on and `oxide-reins apps grant-once "X"` approves one call; a deny is
  lifted with `apps revoke "X"`. Do not work around it.
- `was approved as … but this process is …` — the app's bundle identity changed
  since it was approved. Ask the user whether that is still the app they meant;
  do not re-approve it yourself.
- `unknown key` naming `\n` — `\n` is a backslash and an `n`, not a navigation
  key. Use `press_key` with `Return` or `Tab`.
- `user takeover` — a person used the keyboard or pointer during the gesture.
  Leave the desktop to them, and re-read the app before acting again.
- `is the app hosting this MCP server` — self-automation is refused. Ask the user
  whether they want the hosting app driven at all; `apps self-automation allow`
  is their decision, not yours.
- `is a terminal` — terminal automation is refused by default. If the user asked
  for something in a terminal, tell them the policy refuses it and let them
  decide about `apps terminals allow`; do not route around it with another app.
- `Accessibility permission is missing` — the app hosting the server is not
  trusted. Call `permissions` in this MCP connection and ask the user to approve the reported host in System
  Settings → Privacy & Security → Accessibility, then restart the host. Do not approve permissions yourself.
- `kAXErrorCannotComplete` — the app is busy, modal, or unresponsive. Wait, then
  read the tree again rather than repeating the click.
- `could not read … accessibility tree` — the app stopped answering within the
  two-second bound, so the read failed instead of returning an empty tree. The
  previous snapshot is still the newest one and its indexes still address that
  app; retry once the app answers instead of renumbering by guess.
- `is no longer in the window the snapshot bound it to` or `out of range` — call
  `get_app_state` before retrying; the index no longer points at what you saw.
- `reports AXValue as not settable` — use `click` to focus it and `type_text`
  instead, or `press_key` between segments for date fields.

## Rules

- One action, one confirmation: read the state back instead of chaining five
  blind clicks.
- Never send a password, a payment or an irreversible confirmation through this
  server without the user asking for exactly that.
- The app must be in front for typing and pointer events; activating it is
  allowed, taking over the pointer without need is not.
