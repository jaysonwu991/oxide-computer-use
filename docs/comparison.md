# Compared with other Computer Use servers

This repository supplies a macOS MCP runtime with explicit action semantics and
client registration. Oxide's Desktop repository additionally provides an opt-in
[native companion app](oxide-helper.md). A bundle and a mouse icon do not make
its behavior identical to Codex Computer Use; the README's
[Codex comparison](../README.md#what-is-different-from-codex-computer-use) lists
what belongs to the client, what this runtime now matches and what is absent with
its reason, and [parity.md](parity.md) has the per-item record.

The Codex-specific statements below were checked against the
[official Computer Use documentation](https://learn.chatgpt.com/docs/computer-use)
on 2026-11-08. The existing `open-computer-use` issue references describe those
linked reports, not a fresh verification of every third-party release.

| | oxide-computer-use | open-computer-use | Codex Computer Use |
| --- | --- | --- | --- |
| Platforms | macOS and Windows; Linux builds return an explicit error | macOS, Linux, Windows | macOS and Windows; background operation differs by OS |
| Distribution | macOS arm64/x64 archives or source build; optional development helper built in the Oxide repo | npm package, app bundle, onboarding app | part of Codex |
| MCP client setup | `install --client oxide\|claude\|codex\|cursor\|desktop`, or `--print` | documented config snippets, `install-codex-mcp` | built in; this server registers beside it as `oxide-computer-use` |
| Tool surface | 14 tools: the 11 macOS window operations plus `permissions`, `query` and `screenshot` | 9–10, `select_text` on macOS | product-defined |
| Observation | accessibility diff/full tree plus app-window screenshot; indexes bound to the window they came from | accessibility tree; screenshot embedded in every state by default | screenshots plus accessibility |
| Value writes | refused unless the element reports `AXValue` settable, then read back | reported success on controls that ignored the write ([#83](https://github.com/iFurySt/open-computer-use/issues/83)) | n/a |
| Stale indexes | rejected by name, with the app and window they belong to | not detected | n/a |
| An app that stops answering | the read fails inside a two-second messaging bound; the previous snapshot stays addressable | not detected | n/a |
| A call that breaks | answered as an `isError` result; the connection keeps serving | n/a | n/a |
| Long text | per-value `text_limit` (`"max"` allowed), explicit truncation notes | fixed truncation per platform ([#36](https://github.com/iFurySt/open-computer-use/issues/36)) | n/a |
| Reply size | `max_output_chars` cap; app-state screenshot can be disabled | screenshot in every reply ([#83](https://github.com/iFurySt/open-computer-use/issues/83) 6a) | n/a |
| Per-app control | deny/allow lists enforced in the server, before any AX call | configurable allow list ([#12](https://github.com/iFurySt/open-computer-use/issues/12)) | product approvals |
| Dragging | paced HID events with deltas, the fix for [#53](https://github.com/iFurySt/open-computer-use/issues/53) | paced pointer events ([#53](https://github.com/iFurySt/open-computer-use/issues/53) open) | n/a |
| Testing | Unit/integration checks, no permissions needed for the config, protocol and policy paths | app-level smoke suite | internal |
| Onboarding | `doctor`, host-aware MCP `permissions`; optional Oxide helper permission menu | `doctor` plus a permission onboarding window | app UI |

## What this server does deliberately differently

- **Accessibility first, pointer second.** A left `click` uses the element's
  `AXPress` action when available, allowing supported controls to work without
  foreground input. Other buttons and coordinate clicks use pointer events and
  activate the app. The reply names the path. Pointer input is never isolated
  from the person — the real cursor moves and the app is activated — while the
  keyboard does have a `background` delivery path that leaves the frontmost app
  alone.
- **Refusals instead of silent no-ops.** `set_value` checks `AXIsAttributeSettable`
  and reads the value back; `perform_secondary_action` refuses an action the
  element does not list. An agent can tell "nothing happened" from "it worked".
- **Observation matches the current app window.** The first state is a full
  accessibility tree, later states are diffs, and the reply includes the window
  screenshot by default. `include_screenshot: false` gives a text-only
  observation, `max_output_chars` caps text with a visible note, and `query`
  searches a large tree.
- **Elements belong to a window.** A snapshot binds each index to its
  `AXWindow`; an index from before a window closed or moved is refused rather
  than applied to whatever is now at that position.
- **A wait is bounded and a broken call is answered.** An app that stops
  answering fails the read within two seconds and does not render as an empty
  window; a panic inside one call becomes an `isError` result instead of ending
  the session. A server process that is killed is still the client's to restart.
- **One tool list, one test.** `tools::tests` counts the tool specs, and the
  README table, the `tools` command and `tools/list` all come from it, so the
  documented surface cannot drift from the shipped one.

## When the other project is the better choice

- **You need Linux.** Windows now has a UI Automation backend; Linux still
  compiles to a stub whose tools return "not implemented here".
  `open-computer-use` has real runtimes there, with the issues that follow from
  that ([#89](https://github.com/iFurySt/open-computer-use/issues/89),
  [#91](https://github.com/iFurySt/open-computer-use/issues/91),
  [#51](https://github.com/iFurySt/open-computer-use/issues/51)).
- **You want a ready-to-install native product.** The standalone server has no UI.
  Oxide's helper adds a native permission/stop menu and Desktop adds activity and
  snapshot controls, but they remain an opt-in development workflow. Client-side
  app approval settings, continuous preview and isolated pointer input are still
  missing; keys can be posted to one app with `background: true`, which is the half
  of isolated input a stdio server can offer.
- **You want a signed, notarised download.** This project publishes unsigned
  archives built in CI from a tag, with a shell installer; the others ship a
  signed app bundle or an npm package.
- **You need `select_text` on apps whose text is not exposed as `AXValue`.** Both
  servers use the selected-text-range API, but the other has had more field
  testing against real apps.

## Who wins on which machine

- For a configurable macOS MCP runtime: this server supplies a standalone binary
  and explicit action semantics. The Oxide helper is an optional host for it.
- For Codex's integrated app approvals, macOS background workflow, browser and
  lock behavior: use the built-in product; these are not provided by this server
  or by wrapping it in an app bundle.
- A team automating macOS and Windows: this project has native backends for
  both; Linux remains the reason to choose `open-computer-use`.
