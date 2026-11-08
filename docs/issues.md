# Audit against the open-computer-use issue tracker

Read on the day this project was written, from
<https://github.com/iFurySt/open-computer-use/issues> (24 open issues and the
closed reports that shaped it). Each row says what this project does about it,
and where. "Designed out" means the bug cannot happen here without rewriting the
design; "mitigated" means the failure is possible but reported accurately;
"open" means it is a real gap.

## macOS reports

| Issue | Here |
| --- | --- |
| [#83](https://github.com/iFurySt/open-computer-use/issues/83) macOS 26 findings | Designed out, except the date-input item. |
| ↳ wrong-window targeting, no window selector | `get_app_state` takes `window` (index or title substring) and binds every index to its `AXWindow`; acting on an index whose window changed is refused (`checked_element`, `src/macos/mod.rs`). |
| ↳ `set_value` silently does nothing | Refused when `AXIsAttributeSettable` says no, and the value is read back after writing; a control that ignores the write fails with what it still reads. |
| ↳ screenshot in every reply | Screenshots are opt-in (`include_screenshot`, `screenshot`), never attached to action replies. |
| ↳ `scroll` needs a snapshot first | `scroll` needs no snapshot: `app` alone scrolls the frontmost window. |
| ↳ `--calls-file` paths | The error names the resolved absolute path and the path as typed (`src/main.rs`). |
| ↳ docs/version drift | One tool list feeds `tools`, `tools/list` and the README table; a test pins the count. |
| ↳ `<input type="date">` segments | **Open.** No script evaluation exists. Workaround: `set_value`/`type_text` on each segment, or `press_key` between fields. |
| [#82](https://github.com/iFurySt/open-computer-use/issues/82) Firefox file picker vs page | Mitigated. `list_apps {"include_helpers": true}` reveals the separate save-panel service, and the observation path is app-scoped, so the page tree is the page's. The picker is still another process: switch app to drive it. |
| [#57](https://github.com/iFurySt/open-computer-use/issues/57) `cgWindowNotFound` after logout | Not applicable to observation: the AX path never uses a `CGWindowID`. The screenshot path does, and reports the failure instead of returning a wrong image. |
| [#53](https://github.com/iFurySt/open-computer-use/issues/53) drag does nothing | Designed out: a private HID event source, a move then a paced press, sixteen paced drag steps carrying deltas, then the release (`src/macos/input.rs`). |

## Anything that needs a platform this project does not implement

| Issue | Here |
| --- | --- |
| [#98](https://github.com/iFurySt/open-computer-use/issues/98), [#91](https://github.com/iFurySt/open-computer-use/issues/91), [#90](https://github.com/iFurySt/open-computer-use/issues/90), [#89](https://github.com/iFurySt/open-computer-use/issues/89) Windows | Not applicable. A non-macOS build returns "not implemented on this platform" from every tool instead of half-working. |
| [#66](https://github.com/iFurySt/open-computer-use/issues/66), [#51](https://github.com/iFurySt/open-computer-use/issues/51), [#49](https://github.com/iFurySt/open-computer-use/issues/49), [#87](https://github.com/iFurySt/open-computer-use/issues/87) Linux, Windows | Same. |
| [#23](https://github.com/iFurySt/open-computer-use/issues/23) POSIX-only install scripts | No shell scripts: `install` writes the client's config file directly (JSON, or TOML through a round-trip edit), so nothing depends on a POSIX shell. macOS only, though. |
| [#21](https://github.com/iFurySt/open-computer-use/issues/21) stdio output mangled on Windows | Not applicable on macOS, where the server writes UTF-8 JSON to stdout and never consults a code page. Verified end to end with a non-ASCII app name (`tests/cli.rs`). |

## Ergonomics and safety reports

| Issue | Here |
| --- | --- |
| [#16](https://github.com/iFurySt/open-computer-use/issues/16) significant context usage | Mitigated: `max_output_chars` (default 20000) truncates with a visible note, screenshots are opt-in, and `query` searches a tree without dumping it. |
| [#40](https://github.com/iFurySt/open-computer-use/issues/40) 1200-node ceiling hides content | Mitigated: the reply says it truncated and how to raise it; `max_tree_nodes` and `max_tree_depth` are parameters. |
| [#36](https://github.com/iFurySt/open-computer-use/issues/36) fixed 160-character truncation | Mitigated: `text_limit` per call, `"max"` for full values, and truncated values carry their true length. |
| [#15](https://github.com/iFurySt/open-computer-use/issues/15) prompt before touching an app | Mitigated: the client's own approval prompt, plus an app policy (`apps allow|deny`) enforced inside the server before any accessibility call. macOS has no OS-level per-app prompt, so the policy list is the per-app control. |
| [#12](https://github.com/iFurySt/open-computer-use/issues/12) configurable allow list | Implemented: `apps allow`, `apps deny`, `apps reset`, `apps list`, stored in `~/.config/computer-use/config.json`; a deny rule wins over an allow list. |
| [#39](https://github.com/iFurySt/open-computer-use/issues/39) auditing and visual feedback | Partly. Every action reply names the path it took (AX action versus pointer event), and `include_screenshot` attaches what the agent just read. There is no audit log and no post-action screenshot — **open**. |
| [#11](https://github.com/iFurySt/open-computer-use/issues/11), [#18](https://github.com/iFurySt/open-computer-use/issues/18) macOS permission confusion, CLI `click` errors | Mitigated by messages: `doctor` reports both permissions and the app policy, a missing permission names the Settings pane, and an out-of-range or wrong-app index says which snapshot it belongs to. |
| [#60](https://github.com/iFurySt/open-computer-use/issues/60), [#14](https://github.com/iFurySt/open-computer-use/issues/14), [#55](https://github.com/iFurySt/open-computer-use/issues/55) platform and product requests | Out of scope for a macOS-only server. |

## What this audit could not verify

This machine has Accessibility and Screen Recording ungranted for the terminal,
so these paths compile and are unit-tested but were not exercised against a live
app: window binding and the `window` selector, `set_value`'s settability check
and read-back, `select_text`, the paced drag, `AXPress` versus pointer clicks,
and `include_screenshot`'s window match. The config, protocol, policy, protocol
negotiation and UTF-8 paths were verified end to end, and `list_apps` ran
against the real window server.

To close that gap on a machine where the terminal is trusted:

```sh
computer-use doctor --prompt
computer-use call get_app_state --args '{"app":"TextEdit","window":0}'
computer-use call click --args '{"app":"TextEdit","element_index":3}'
computer-use call --calls '[{"tool":"get_app_state","args":{"app":"TextEdit","include_frames":true}},{"tool":"drag","args":{"app":"TextEdit","from_x":200,"from_y":300,"to_x":400,"to_y":300}}]'
```
