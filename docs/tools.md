# Tool reference

Every tool is available as an MCP tool (`tools/list`, `tools/call`) and through
the CLI (`oxide-reins call <tool> --args '<json>'`). The schemas are defined
once in `src/tools.rs`; this page explains what each one does with them.

## Conventions that apply to every tool

### How `app` is resolved

`app` accepts, in this order:

1. a process id, when the value parses as a number and that process owns a
   window;
2. an app name, case-insensitively, exactly as `list_apps` prints it;
3. a case-insensitive substring of an app name, first match wins.

Resolution searches every window owner, including the window-server helpers
that `list_apps` hides — so a native file picker announced as
`Open and Save Panel Service` can be driven by pid even when it is not listed.
An ambiguous or missing name fails with the list of running app names.

The **app policy** is checked after resolution, against the app's real name, and
before any accessibility or input call (see
[configuration](configuration.md#app-policy) for the file):

- a name on the deny list → ``` `X` is on the deny list ``` with the `apps revoke`
  command that lifts it;
- an approved name whose bundle identity changed → the recorded and the running
  bundle identifiers, both paths, and `apps allow` to re-approve;
- a name with no decision under a default-deny policy → `apps grant-once` for one
  call and `apps allow` for every call;
- a non-empty allow list that excludes it → ``` `X` is not on the allow list ```
  with the current list.

The file is re-read on every call, so an approval or a revocation made in a
terminal lands in a running `mcp` server. An app marked `[approved for one call]`
in `list_apps` has a one-time approval that the next call spends.

### Element indexes

`get_app_state` and `query` build a *snapshot*: a flat, numbered rendering of an
app's accessibility tree. The number on the left of each line is the
`element_index` that `click`, `perform_secondary_action`, `set_value`,
`select_text`, `type_text`, `scroll` and `drag` take.

- Indexes belong to the newest snapshot **of that process**. `oxide-reins mcp`
  keeps one snapshot for the life of the connection; `oxide-reins call` can run
  a whole sequence in one process with `--calls` (`--calls-file` too).
- Each index is bound to the `AXWindow` it was captured in. Before acting, the
  server walks up from the element and compares window identity; if the element
  moved to another window the call is refused (`is no longer in the window the
  snapshot bound it to`) instead of hitting whatever is at that position now.
- An index from a different app, or past the end of the snapshot, is refused and
  names the app and size it should belong to.

### Reply shape

A call returns an MCP `CallToolResult`:

```json
{
  "content": [{ "type": "text", "text": "Clicked element 12 (AXButton) with AXPress; it never needed the pointer." }],
  "isError": false
}
```

`get_app_state` with `include_screenshot`, and `screenshot`, add a second part:

```json
{ "type": "image", "data": "<base64 PNG>", "mimeType": "image/png" }
```

A failure is the same shape with `"isError": true` and a message written for an
agent: what failed, why, and what to do instead. Failures are never JSON-RPC
protocol errors; those are reserved for malformed JSON and unknown methods.

### Read-only hints

`permissions`, `list_apps`, `get_app_state`, `query` and `screenshot` carry
`annotations.readOnlyHint: true` (and `destructiveHint: false`); the rest are
marked as actions, with `idempotentHint: true` for `set_value` and
`select_text`. Clients use these hints for approval rules.

### Limits

| Budget | Default | Where it can be changed |
| --- | --- | --- |
| Tree nodes per snapshot | 1200 | `max_tree_nodes` |
| Tree depth | 64 | `max_tree_depth` |
| Characters per text value | 200 | `text_limit` (`"max"` for the full value) |
| Reply text | 20000 characters | `max_output_chars` |
| `query` results | 20 | `max_results` |

Truncation is always stated in the reply, with the real size, and with the
argument that raises the budget.

## Observation tools

### `list_apps`

Lists apps that own at least one usable window.

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `include_helpers` | boolean | `false` | Also show window-server helper surfaces, such as a save panel that belongs to no app. |

A window counts as usable when it is layer 0, at least 120×80 points, and its
owner is not a window-server agent (`Window Server`, `loginwindow`, `Dock`,
`Control Center`, `Notification Center`, `通知中心`, `SystemUIServer`,
`通用控制`, `自动填充`, or any name ending in `Service`). Windows in other Spaces
are included, because accessibility still reaches those apps.

Example reply:

```text
Code (pid 65697) — 1728x1084, 500x500
Microsoft Edge (pid 641) — 1727x1084 [blocked by app policy]
Preview (pid 812) — 900x700 [approved for one call]
```

A marker is only ever a policy statement: `[blocked by app policy]` covers the
deny list, an allow list that excludes the app and a default-deny policy, and
`[approved for one call]` marks a grant the next call spends.

### `get_app_state`

Reads one app's accessibility tree.

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | required | See resolution above. |
| `window` | string | whole app | Zero-based window index or a title substring; binds the snapshot to that window. |
| `text_limit` | integer or `"max"` | 200 | Characters shown per `value=`. |
| `max_tree_nodes` | integer | 1200 | Node budget. |
| `max_tree_depth` | integer | 64 | Depth budget. |
| `include_frames` | boolean | `false` | Append each element's on-screen center as `@(x,y)`. |
| `include_screenshot` | boolean | `false` | Attach a PNG of the same window. |
| `max_output_chars` | integer | 20000 | Cap on the reply text. |

The reply starts with a header, then one numbered line per element:

```text
App: TextEdit (pid 4711)
Window: "Untitled" (element indexes are bound to this window)
Elements: 42

   0 AXApplication "TextEdit" · focused
   1   AXWindow "Untitled" @(120,80)
   2     AXTextArea value="" @(140,120)
   3     AXButton "Bold" @(150,300)
```

Without `window`, the header lists the app's windows, and — when the app exposes
more than one `AXWindow` — the exact `window` values to choose from. Lines carry
role first, then `"title"` or `"description"`, `value=`, `id=`, and the
`· focused` / `· disabled` markers. A skipped window title is not an error: some
apps keep their title in the window server only.

When `include_screenshot` is set, a capture failure does not lose the tree; the
reply ends with `Screenshot unavailable: …`.

### `query`

Searches the tree without printing it, and refreshes the element indexes.

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | required | See resolution above. |
| `window` | string | whole app | Same binding as `get_app_state`. |
| `text` | string | — | Case-insensitive substring of the rendered line (role, title, description, value). |
| `role` | string | — | Case-insensitive substring of the role, e.g. `AXButton`. |
| `max_results` | integer | 20 | Matches shown. |
| `max_tree_nodes` | integer | 1200 | Node budget while searching. |
| `max_output_chars` | integer | 20000 | Cap on the reply text. |

```text
2 matches in TextEdit:
  12 AXButton "Bold"
  18 AXMenuItem "Save"
```

No match is a normal, empty result — not an error.

### `screenshot`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | — | Capture that app's window instead of the screen. |
| `window` | string | largest window | With `app`: window index or title substring. |

The PNG is returned as an `image` content part and needs Screen Recording
permission. Without `app` the whole screen is captured. A covered or off-Space
window can fail in `screencapture`; the error says so instead of returning a
picture of something else.

## Action tools

### `click`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | required | See resolution above. |
| `element_index` | integer | — | Element from the newest snapshot. |
| `x`, `y` | number | — | Screen coordinates, when no index is given. |
| `button` | `"left"` or `"right"` | `left` | Pointer button. |

Two paths, and the reply names the one that ran:

1. **`AXPress`** when the element lists that action. This reaches a background
   app and never moves the pointer.
2. **Pointer click** at the element's center (or at `x`/`y`): the app is
   activated first, then a move, a press and a release are posted from a private
   HID event source with a settled delay between them.

An element with no on-screen frame and no `x`/`y` is refused rather than guessed.

### `perform_secondary_action`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | required | See resolution above. |
| `element_index` | integer | required | Element from the newest snapshot. |
| `action` | string | `AXPress` | An accessibility action such as `AXShowMenu`, `AXConfirm`, `AXIncrement`. |

The action is checked against the element's action list first; an unsupported
action fails with the list of actions it does support.

### `set_value`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | required | See resolution above. |
| `element_index` | integer | required | Element from the newest snapshot. |
| `value` | string | required | The text to write. |

Before writing, the element must report `AXValue` as settable; after writing, the
value is read back and compared. A control that is not settable, or that ignores
the write, produces an error — never a success message. On failure, the message
names what it still reads, which is the clue that the control needs focus plus
`type_text` or `press_key` instead.

### `select_text`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | required | See resolution above. |
| `text` | string | required | The text to select inside the element. |
| `element_index` | integer | focused element | Element to select in. |

The element's text is read from `AXValue`, or through `AXStringForRange` when the
value is empty. The match is located in UTF-16 units — the unit Cocoa text
ranges use — and applied as `AXSelectedTextRange`. Case-sensitive first, then a
case-insensitive retry. Not finding the text fails with the element's opening
characters, so an agent can correct the needle. When the app reports
`AXSelectedText`, it is compared; a different selection is an error.

### `type_text`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `text` | string | required | Text to type. |
| `app` | string | — | App to activate and, with `element_index`, the element to focus. |
| `element_index` | integer | — | Element to focus before typing. |

The text is sent as keyboard event pairs carrying a UTF-16 string, so it arrives
in the app's key handling rather than being pasted. The target must be the
focused control of the frontmost app; `set_value` or `select_text` is the more
reliable path for text fields.

A newline inside `text` is pressed as the `Return` key: an app that receives a
newline inside one keyboard event usually inserts nothing. The reply counts the
characters and the line breaks that were sent. A literal backslash and `n` in
`text` is two characters and is typed as such, and the reply says so — use a real
newline in `text`, or `press_key` with `Return` or `Tab`, to move on. Typing stops
if a person's keystroke or pointer movement arrives mid-way, and the reply says
that the rest was not sent.

### `press_key`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `key` | string | required | A key name, a single character, or `cmd+s` style with inline modifiers. |
| `modifiers` | string | — | `cmd`, `shift`, `alt`/`opt`, `ctrl`, combined with `+` or `,`. |
| `app` | string | — | App to activate first. |

Key names: `Return`, `Enter`, `Tab`, `Space`, `Delete`/`Backspace`,
`Forward_Delete`, `Escape`, `Home`, `End`, `Page_Up`, `Page_Down`, `Left`,
`Right`, `Up`, `Down`, `F1`–`F12`, or any single character that maps to a US
keyboard key. The spellings `\n`, `\r`, `newline`, `linefeed`/`lf` and `cr` press
`Return`; `\t` presses `Tab`. Modifier flags are set on both the press and the
release. The key is resolved before the app is activated or a permission is
asked for, and an unknown key is refused with a hint that names both the escape
spellings and the alternative, not silently dropped.

### `scroll`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | — | App to activate first. |
| `direction` | `"up"` or `"down"` | `down` | Direction. |
| `lines` | integer | 3 | Line count; the absolute value is used. |
| `element_index` | integer | — | Scroll at that element's center. |
| `x`, `y` | number | — | Scroll at that point. |

Without a position, the wheel event goes wherever the pointer is. No snapshot is
required, so an app can be scrolled before it is ever read.

### `drag`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | — | App to activate first. |
| `element_index` | integer | — | Start at that element's center. |
| `from_x`, `from_y` | number | — | Start point, when no index is given. |
| `to_x`, `to_y` | number | required | End point. |

A drag a window manager and a text view both accept: the pointer moves to the
start, the button goes down after a settled delay, sixteen paced drag steps carry
per-step deltas, and the button is released only after the last step. A single
unpaced event, the usual cause of a drag that does nothing, is never posted.

The drag yields to the person: a listen-only event tap watches for input that did
not come from this process, and the first such keystroke, pointer movement or
scroll releases the button at the last position and fails the call with "user
takeover". Without the Accessibility grant the tap cannot be installed, and the
drag then runs without that arbitration.

### Taking over from the agent

A person's input during a paced gesture — a drag, or a `type_text` with newlines
— wins. The runtime releases what it holds, says what happened, and stops; the
next call starts from a fresh observation. Nothing is queued behind it.

## Nothing is silently successful

| Situation | What the tool does |
| --- | --- |
| Element not settable | Refuses before writing. |
| Value ignored by the control | Refuses after reading it back. |
| Unsupported AX action | Refuses, listing supported actions. |
| Element moved to another window | Refuses; asks for a fresh `get_app_state`. |
| Index out of range or from another app | Refuses, naming the snapshot it belongs to. |
| App denied or not allowed by policy | Refuses, with the command that changes the policy. |
| App's bundle identity changed since approval | Refuses, naming both identifiers and paths. |
| Policy file does not parse | Refuses every app, naming the path and `apps reset`. |
| One-time approval already spent | Refuses with the two approval commands. |
| A person takes over during a drag or a long type | Releases the button, stops, and reports the takeover. |
| Missing Accessibility or Screen Recording | Refuses, naming the Settings pane. |
| Screenshot unavailable during `include_screenshot` | Returns the tree plus the reason the image is missing. |

### `permissions`

No arguments. Reports Accessibility and Screen Recording from the server process,
its observed launching app bundle, and setup guidance, without showing permission
prompts. Use this through the client's MCP connection: `doctor` in an unrelated
terminal measures that terminal's permissions instead. Accessibility enables
reading and driving apps; Screen Recording is needed only for screenshots.
The observed ancestor bundle helps identify the host; macOS decides which
responsible process receives a grant.

A right `click` always uses a pointer event, including on elements exposing
AXPress; AXPress is a primary action and would otherwise turn a context click
into an ordinary activation.
