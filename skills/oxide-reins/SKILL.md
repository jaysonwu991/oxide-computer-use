---
name: oxide-reins
description: Drive this Mac's apps through the oxide-reins MCP server — list apps, read an accessibility tree, then click, type, select, scroll or drag. Use when a task needs a GUI app the agent cannot reach through files or a CLI.
---

# Computer Use

The `oxide-reins` MCP server exposes the desktop as tools. Reading is cheap;
acting moves the real pointer and keyboard, so look first and act on what you
saw. `oxide-reins install` registers it under `oxide-reins`, or
`oxide-reins-local` for Claude Code and Codex, which already ship their own
Computer Use under the plain `computer-use` name; the tool names are identical either way.

## Loop

1. Use `permissions` for setup problems: it checks this MCP process without prompting and identifies the hosting app. In Oxide Desktop grant Oxide.app, not an unrelated terminal.
2. `list_apps` — find the app's exact name or pid. Add
   `{"include_helpers": true}` when a native file picker or another
   window-server surface seems missing.
3. `get_app_state` with that `app` — read the indexed tree. Pass `window` (an
   index or a title substring) when the app has more than one window; the reply
   lists them.
4. Act with `element_index`, or `x`/`y` from `include_frames: true`.
5. Read again after the action. Indexes belong to the newest snapshot and to the
   window they came from; a refusal means the state moved, not that the tool is
   broken.

## Choosing a tool

- `click` uses the element's AXPress action when it has one; the reply says
  whether it used AXPress or a pointer event.
- `set_value` is the reliable way to fill a text field, and it fails loudly when
  a control is read-only or ignores the write. `type_text` needs the control
  focused and the app frontmost.
- `select_text` marks a span for copying or for a following `press_key`.
- `press_key` takes one key plus `modifiers` (`cmd`, `shift`, `alt`, `ctrl`),
  for example `{"key":"s","modifiers":"cmd"}` to save.
- `query` when a tree is large: filter by text or role, and take the index from
  its output, which also refreshes the snapshot.
- `perform_secondary_action` for AX actions such as `AXShowMenu` on a row; an
  unsupported action comes back with the list the element does support.
- `screenshot`, or `get_app_state` with `include_screenshot: true`, only when
  the tree is not enough — it needs Screen Recording, and not every agent can
  see the image.

## Keeping context small

- `max_output_chars` caps a reply; the truncation note says so.
- `query` replaces a full tree dump when you know what you are looking for.
- `text_limit: "max"` is for the one field you must read in full; the default
  truncates long values with their true length.
- Screenshots are never attached unless you ask. For visual review, capture the target window and return its image to the client; Oxide Desktop shows a clickable preview. Re-observe after changes and leave submission to the user when requested. Keyboard and pointer tools use the foreground; do not promise isolated background control.

## Failures worth reading

- `` `X` is on the deny list `` or `` not on the allow list `` — the local app
  policy. `oxide-reins apps allow "X"` changes it; do not work around it.
- `Accessibility permission is missing` — the app hosting the server is not
  trusted. Call `permissions` in this MCP connection and ask the user to approve the reported host in System
  Settings → Privacy & Security → Accessibility, then restart the host. Do not approve permissions yourself.
- `kAXErrorCannotComplete` — the app is busy, modal, or unresponsive. Wait, then
  read the tree again rather than repeating the click.
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
