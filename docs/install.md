# Install

Two ways in: a release archive from the installer, or a build from a checkout.

## The installer

```sh
curl -fsSL https://raw.githubusercontent.com/jaysonwu991/oxide-reins/main/install.sh | sh
```

It reads `uname` for the platform, downloads the matching archive from the
latest [release](https://github.com/jaysonwu991/oxide-reins/releases), verifies
its SHA-256 against the published `.sha256`, and installs the binary to
`$HOME/.local/bin/oxide-reins`.

| Environment variable | Default | Purpose |
| --- | --- | --- |
| `OXIDE_REINS_INSTALL_DIR` | `$HOME/.local/bin` | Where the binary goes. |
| `OXIDE_REINS_VERSION` | latest release | A version to pin, with or without the leading `v`. |
| `OXIDE_REINS_REPO` | `jaysonwu991/oxide-reins` | The repository to install from, for a fork or a mirror. |
| `OXIDE_REINS_RELEASES` | `https://github.com/<repo>/releases` | Where the manifest and archives come from; a `file://` base is how the offline check in `tests/install.sh` runs. |

A pinned version skips the release manifest and fetches
`releases/download/v<version>/oxide-reins-<platform>.tar.gz` directly. Without
one, the installer reads the `oxide-reins-manifest` that every release carries,
so asset names can change between releases without breaking an old script.

The install leaves a marker beside the binary, `~/.local/bin/.oxide-reins-install`,
naming the repository and the version that was installed:

```sh
cat ~/.local/bin/.oxide-reins-install   # source jaysonwu991/oxide-reins
                                         # version 0.1.0
```

To update, rerun the installer; there is no self-update subcommand. To remove
it, delete the binary, the marker and the client entry
`oxide-reins install` wrote.

## Checking for an update

```sh
sh install.sh --check
```

It compares the version in the marker beside the installed binary
(`.oxide-reins-install`) with the version the latest release's manifest names, and
exits 0 when they match, 1 when an update is available, and 2 when it cannot tell
— no installed copy, or no reachable manifest. Versions are compared as the
release stamps them with any leading `v` removed, so nothing in the script
interprets ordering.

The check reads only the manifest: it downloads no archive and changes nothing.
Re-running the installer is how an update is applied, and it rewrites the marker
— version, platform and SHA-256 of the installed binary — from what it actually
put in place.

`tests/install.sh` runs all four outcomes against a staged release directory
through `file://`, with no network and no macOS API, and CI runs it on every
change.

## Release assets

Publishing the draft release that
[`.github/workflows/release-drafter.yml`](../.github/workflows/release-drafter.yml)
keeps up to date is what releases oxide-reins: it creates the `v*` tag, and
[`.github/workflows/release.yml`](../.github/workflows/release.yml) stamps that
tag into the crate and attaches the archives below a few minutes later. The
stamp is why the binary in an archive reports the release it came from;
`oxide-reins --version` prints the tag, not the `Cargo.toml` a checkout was
built from. Until the workflow finishes, the release page shows the notes
without the assets, and the installer reports that it found no release manifest.

| Platform | Archive | Checksum |
| --- | --- | --- |
| macOS, Apple Silicon | `oxide-reins-macos-arm64.tar.gz` | `.tar.gz.sha256` |
| macOS, Intel | `oxide-reins-macos-x64.tar.gz` | `.tar.gz.sha256` |

Each archive holds one file, `oxide-reins`. No asset carries its version in the
name, so the newest archive of a platform keeps one name from release to
release; the release page is where the version is written down. The
`oxide-reins-manifest` asset lists the archive for each platform token
(`macos-arm64`, `macos-x64`), which is what the installer reads.

There is no Linux or Windows archive. The crate compiles on other platforms, but
every tool there returns an explicit "not implemented here" error — the server
drives the macOS accessibility and event APIs, so a stub build is not an
install. `install.sh` refuses another OS by name instead of downloading
something that cannot work.

## From source

Requires macOS 14 or later, Rust 1.85 or newer and the Xcode Command Line
Tools.

```sh
cargo build --release
install -m 755 target/release/oxide-reins ~/.local/bin/oxide-reins
```

`cargo install --path .` works too and puts the binary in `~/.cargo/bin`. A
checkout reports the version in `Cargo.toml`; only a release tag rewrites that,
and only inside CI, so a local build never claims to be a published release.

## Then

```sh
oxide-reins doctor                         # Accessibility, Screen Recording, policy
oxide-reins install --client codex         # or oxide, claude, cursor, desktop
```

Permission attribution depends on how the server is launched. For direct stdio,
start the agent from the terminal or app you granted, then call `permissions`
through that MCP connection. Oxide's optional [native helper](oxide-helper.md)
launches the runtime under its own app bundle; it requires separate grants.
`install.sh` and `oxide-reins install` continue to install/register the standalone
runtime, not the helper. See
[docs/configuration.md](configuration.md) for the policy file. Registration
names, including the clients that already ship their own Computer Use, are in
[docs/clients.md](clients.md).
