# Tool reference

Every tool is available as an MCP tool (`tools/list`, `tools/call`) and through
the CLI (`oxide-computer-use call <tool> --args '<json>'`). The schemas are defined
once in `src/tools.rs`; this page explains what each one does with them.

## Conventions that apply to every tool

### How `app` is resolved

`app` accepts, in this order:

1. a process id, when the value parses as a number and that process owns a
   window;
2. a bundle identifier, application path, display name or process name from
   `list_apps`, case-insensitively;
3. a case-insensitive substring of a discoverable app name or bundle identifier,
   first match wins.

A leading `@`, as in an `@Chrome` client mention, is accepted and removed before
resolution.

Resolution searches every window owner, including the window-server helpers
that `list_apps` hides — so a native file picker announced as
`Open and Save Panel Service` can be driven by pid even when it is not listed.
If the selected app is installed but not running, the first tool call launches
it in the background and waits up to ten seconds for a window. A missing or
ambiguous name fails with the running app names.

The **app policy** is checked after resolution, against the app's real name, and
before any accessibility or input call (see
[configuration](configuration.md#app-policy) for the file):

- a name on the deny list → ``` `X` is on the deny list ``` with the `apps revoke`
  command that lifts it;
- an approved name whose bundle identity changed → the recorded and the running
  bundle identifiers, both paths, and `apps allow` to re-approve;
- a name with no decision under a default-deny policy → `apps grant-once` for one
  call and `apps allow` for every call;
- a terminal, or the hosting app, under the rules that refuse them by default;
- a non-empty allow list that excludes it → ``` `X` is not on the allow list ```
  with the current list.

The file is re-read on every call, so an approval or a revocation made in a
terminal lands in a running `mcp` server. An app marked `[approved for one call]`
in `list_apps` has a one-time approval that the next call spends.

**Terminals are refused** by the same gate, by bundle identity — driving a shell
is how an agent starts another agent — unless `apps terminals allow` says
otherwise. A single terminal cannot be exempted by name: the identity is
authoritative, which is what keeps a renamed app from slipping through.

**Self-automation** is a rule of its own, checked after the app policy: the app
bundle that launched this server is refused by default, because an agent clicking
the window it is shown in is a loop. The refusal names the bundle and
`oxide-computer-use apps self-automation allow`. A server started from a shell has no
app bundle above it, so nothing is refused there; a server hosted by Oxide, an
editor or a terminal refuses that host and reaches everything else normally.

### Element indexes

`get_app_state` and `query` build a *snapshot*: a flat, numbered rendering of an
app's accessibility tree. The number on the left of each line is the
`element_index` that `click`, `perform_secondary_action`, `set_value`,
`select_text`, `type_text`, `scroll` and `drag` take.

- Indexes belong to the newest snapshot **of that process**. `oxide-computer-use mcp`
  keeps one snapshot for the life of the connection; `oxide-computer-use call` can run
  a whole sequence in one process with `--calls` (`--calls-file` too).
- Each index is bound to the `AXWindow` it was captured in. Before acting, the
  server walks up from the element and compares window identity; if the element
  moved to another window the call is refused (`is no longer in the window the
  snapshot bound it to`) instead of hitting whatever is at that position now.
- An index from a different app, or past the end of the snapshot, is refused and
  names the app and size it should belong to.
- A snapshot that could not be read is not a snapshot. If the app stops
  answering, the read fails after two seconds per message
  (`kAXErrorCannotComplete`, naming the bound) and the previous snapshot stays
  the newest one, so its indexes still address the app they came from. Nothing is
  renumbered by guesswork.

### Reply shape

A call returns an MCP `CallToolResult`:

```json
{
  "content": [{ "type": "text", "text": "Clicked element 12 (AXButton) with AXPress; it never needed the pointer." }],
  "isError": false
}
```

`get_app_state` attaches a screenshot by default; the standalone `screenshot`
tool always does:

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
| App state tree | full on first access; diff afterward | `disableDiff` |
| Screenshot | on by default | `include_screenshot: false` disables it on `get_app_state` |
| Tree nodes per snapshot | 1200 | `max_tree_nodes` |
| Tree depth | 64 | `max_tree_depth` |
| Characters per text value | 200 | `text_limit` (`"max"` for the full value) |
| Reply text | 20000 characters | `max_output_chars` |
| `query` results | 20 | `max_results` |

Truncation is always stated in the reply, with the real size, and with the
argument that raises the budget.

## Observation tools

### `list_apps`

Lists launchable applications in the standard application directories, plus
running app bundles that live elsewhere. The reply is a JSON array:

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `include_helpers` | boolean | `false` | Also show window-server helper surfaces, such as a save panel that belongs to no app. |

```json
[
  {
    "id": "com.apple.TextEdit",
    "displayName": "TextEdit",
    "isRunning": true,
    "path": "/System/Applications/TextEdit.app",
    "policy": "allow"
  }
]
```

`id` is the bundle identifier when one can be read, otherwise the application
path. The policy field is a statement only: a blocked app is still listed so a
person can see why it cannot be reached and which command changes that.

### `app_policy`

Reads or changes the per-app access used by Oxide's Computer Use settings.

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `action` | string | required | `list`, `allow`, `grant_once`, `deny`, `revoke`, `set_default`, `set_self_automation`, `set_terminals` or `reset`. |
| `app` | string | — | Bundle id, name, path or `@App` mention for an app operation. |
| `mode` | `"allow"` or `"deny"` | — | Fallback for a `set_*` operation. |

The reply contains the resolved default, allow/deny lists, pinned identities,
one-time grant, self-automation and terminal settings, plus launchable app rows
with their current policy. Oxide calls this tool from its Computer Use settings
dialog, so Always allow, Allow once, Block and Revoke all use the same
bundle-identity checks as an agent call.

### `get_app_state`

Launches the app if necessary and reads its accessibility tree. The first read
returns the full tree; later reads of the same app/window return only changed,
added and removed element lines, with current indexes. A read that the app does
not answer fails as an error within a two-second bound per message, instead of
returning an empty or half-read tree that looks like a real answer.

After an action, `get_app_state` waits about one second before reading. If the
new tree still exposes a busy/progress indicator, it retries in 250 ms steps up
to a five-second bound. For selected common apps, the first successful read also
prepends an independently authored `<app_specific_instructions>` block; the
guidance is delivered once per app bundle in that runtime process.

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | required | See resolution above. |
| `window` | string | whole app | Zero-based window index or a title substring; binds the snapshot to that window. |
| `disableDiff` | boolean | `false` | Return the complete tree instead of the diff from the previous state. |
| `text_limit` | integer or `"max"` | 200 | Characters shown per `value=`. |
| `max_tree_nodes` | integer | 1200 | Node budget. |
| `max_tree_depth` | integer | 64 | Depth budget. |
| `include_frames` | boolean | `false` | Append each element's on-screen center as `@(x,y)`. |
| `include_screenshot` | boolean | `true` | Attach a PNG of the same app window; `false` requests a text-only observation. |
| `max_output_chars` | integer | 20000 | Cap on the reply text. |

The first reply starts with a header, then one numbered line per element:

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

A later reply replaces the unchanged lines with `No accessibility changes since
the previous state.` or a `Changed accessibility elements:` section. `~` marks a
changed value at the same index. If the number of elements changed, the reply
prints the complete current tree because reusing an index across an insertion or
removal would be ambiguous.

Coordinates in the attached image are the coordinates `click`, `scroll` and
`drag` accept. The runtime maps them through the app-window origin and Retina
scale. A capture failure does not lose the tree; the reply ends with
`Screenshot unavailable: …`.

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

## Preview: the screen resource

The server publishes one MCP resource, `computer-use://screen` (PNG):

- `resources/list` names it;
- `resources/read` captures the screen then and returns a base64 PNG block, so a
  client can show a current view without spending a tool call of its own — and
  without the image inflating a tool reply;
- `resources/subscribe` registers interest. After that, every call that could
  have changed the screen — any action tool, and any read that returned an image
  — is followed by `notifications/resources/updated` with the resource URI. A
  call that only reads the accessibility tree reports nothing.

That is the whole streaming contract a stdio server can offer: the client decides
how often to re-read, and it is told when re-reading is worth it. The server does
not post frames on its own timer, and capture still needs Screen Recording; a
read without it is an error, not an empty image.

## Action tools

### `click`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | required | See resolution above. |
| `element_index` | integer | — | Element from the newest snapshot. |
| `x`, `y` | number | — | App-window screenshot coordinates, when no index is given. |
| `mouse_button` | `left`, `right`, `middle`, `l`, `r`, `m` | `left` | Pointer button. |
| `click_count` | integer | 1 | Number of clicks, for double/triple click. |
| `key` | string | — | Modifier chord held across the click, such as `Shift` or `Super+Shift`. |
| `duration` | number | — | Seconds to hold the button before release. |

Two paths, and the reply names the one that ran:

1. **`AXPress`** for one plain left click when the element lists that action.
   This reaches a background app and never moves the pointer.
2. **Pointer click** for every other option, or at `x`/`y`: the app is activated
   first, then move/down/up events are posted from a private HID event source.

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
| `element_index` | integer | required | Element to select in. |
| `prefix` | string | — | Text immediately before the target, for disambiguation. |
| `suffix` | string | — | Text immediately after the target, for disambiguation. |
| `selection_type` | `text`, `cursor_before`, `cursor_after` | `text` | Select the span or place a zero-width cursor on either side. |

The element's text is read from `AXValue`, or through `AXStringForRange` when the
value is empty. The match is located in UTF-16 units — the unit Cocoa text
ranges use — and applied as `AXSelectedTextRange`. Case-sensitive first, then a
case-insensitive retry. Prefix/suffix matching filters repeated occurrences, and
the cursor modes apply the same match with a zero-length range. Not finding the
text fails with the element's opening characters. The selected range and text
are read back; a different selection is an error.

### `type_text`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `text` | string | required | Text to type. |
| `app` | string | required | App to activate and, with `element_index`, the element to focus. |
| `element_index` | integer | — | Element to focus before typing. |
| `background` | boolean | `false` | Deliver to `app`'s process only, without activating it. |

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
| `key` | string | required | An xdotool-style key or chord such as `Return`, `Super+c`, `Control_L+a`, `KP_0`. |
| `duration` | number | 0 | Seconds to hold the key before releasing it. |
| `modifiers` | string | — | Legacy separate modifier list; `Key+modifier` chords are preferred. |
| `app` | string | required | App to activate first, or the app a background press is addressed to. |
| `background` | boolean | `false` | Deliver to that app's process only, without activating it. |

Key names include `Return`, `Tab`, `Space`, `Backspace`, `Forward_Delete`,
navigation keys, `F1`–`F20`, keypad keys such as `KP_Enter`, modifier keys and
their `_L`/`_R` variants, punctuation names, or any single character that maps to
a US keyboard key. X Window aliases such as `Prior` and `Next` are accepted,
whitespace around `+` is ignored, and unknown modifier tokens are refused instead
of silently dropped. The spellings `\n`, `\r`, `newline`, `linefeed`/`lf` and
`cr` press `Return`; `\t` presses `Tab`. Modifier flags are set on both the press
and release. The key is resolved before the app is activated or a permission is
asked for.

### Background input

`type_text` and `press_key` activate the target app by default, because that is
what the real keyboard and pointer require. `background: true` with `app` set
posts the same events to that process alone (`CGEventPostToPid`): the app is not
brought forward and the person keeps their focus. Both tools say which path ran.

An app that routes key handling through its own window or focus state can ignore
input delivered this way — the events never reach the system keyboard focus — so
the reply tells you to confirm with `get_app_state` rather than claiming the text
landed. `background` without `app` is refused, since there is no process to
address. The takeover rule does not apply here: nothing was taken from the person.

### `scroll`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | required | App to activate first. |
| `direction` | `up`, `down`, `left`, `right` | required | Scroll axis and direction. |
| `pages` | number | 1 | Distance in pages. |
| `element_index` | integer | — | Scroll at that element's center. |
| `x`, `y` | number | — | Scroll at that app-window screenshot coordinate. |

Without a position, the wheel event goes wherever the pointer is. No snapshot is
required, so an app can be scrolled before it is ever read. One page is the
matching dimension of the bound app window, posted as pixel-unit wheel events.

### `drag`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | required | App to activate first. |
| `from_x`, `from_y` | number | required | Start app-window screenshot coordinate. |
| `to_x`, `to_y` | number | required | End app-window screenshot coordinate. |
| `element_index` | integer | — | Optional start at that element's center. |

A drag a window manager and a text view both accept: the pointer moves to the
start, the button goes down after a settled delay, sixteen paced drag steps carry
per-step deltas, and the button is released only after the last step. A single
unpaced event, the usual cause of a drag that does nothing, is never posted.

The drag yields to the person: a listen-only event tap watches for input that did
not come from this process, and the first such keystroke, pointer movement or
scroll releases the button at the last position and fails the call with "user
takeover". A tap the window server refuses leaves the drag without that arbitration, and
whether an installed tap receives a person's events is only proven on a granted
machine; `oxide-computer-use verify` reports both.

### `paste`

| Parameter | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app` | string | required | App to activate and paste into. |
| `text` | string | required | Plain text, Markdown source or HTML. |
| `format` | `text`, `md`, `html` | required | Pasteboard content format. |

The runtime snapshots every data type on the general pasteboard, writes the new
value, activates the app and presses `Command+V`, then restores the previous
clipboard items. `md` is inserted as plain text containing Markdown; `html`
uses the native HTML pasteboard type. This is the preferred path for formatted
or multiline content that should not be submitted by a Return key.

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
| The hosting app is named under a deny-self-automation policy | Refuses, naming the bundle and `apps self-automation allow`. |
| A terminal is named under the default terminal rule | Refuses, naming the bundle and `apps terminals allow`. |
| `background: true` without `app` | Refuses before changing focus or asking for a permission. |
| The screen resource is read without Screen Recording | JSON-RPC error carrying the permission message. |
| Missing Accessibility or Screen Recording | Refuses, naming the Settings pane. |
| Screenshot unavailable | Returns the accessibility state plus the reason the image is missing. |
| Clipboard restore fails after `paste` | Reports both the paste result and the restore failure. |

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
