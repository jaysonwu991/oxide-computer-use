# Install

Two ways in: a release archive from the installer, or a build from a checkout.

## The installer

```sh
curl -fsSL https://raw.githubusercontent.com/jaysonwu991/oxide-computer-use/main/install.sh | sh
```

It reads `uname` for the platform, downloads the matching archive from the
latest [release](https://github.com/jaysonwu991/oxide-computer-use/releases), verifies
its SHA-256 against the published `.sha256`, and installs the binary to
`$HOME/.local/bin/oxide-computer-use`.

| Environment variable | Default | Purpose |
| --- | --- | --- |
| `OXIDE_COMPUTER_USE_INSTALL_DIR` | `$HOME/.local/bin` | Where the binary goes. |
| `OXIDE_COMPUTER_USE_VERSION` | latest release | A version to pin, with or without the leading `v`. |
| `OXIDE_COMPUTER_USE_REPO` | `jaysonwu991/oxide-computer-use` | The repository to install from, for a fork or a mirror. |

A pinned version skips the release manifest and fetches
`releases/download/v<version>/oxide-computer-use-<platform>.tar.gz` directly. Without
one, the installer reads the `oxide-computer-use-manifest` that every release carries,
so asset names can change between releases without breaking an old script.

The install leaves a marker beside the binary, `~/.local/bin/.oxide-computer-use-install`,
naming the repository and the version that was installed:

```sh
cat ~/.local/bin/.oxide-computer-use-install   # source jaysonwu991/oxide-computer-use
                                         # version 0.1.0
```

To update, rerun the installer; there is no self-update subcommand. To remove
it, delete the binary, the marker and the client entry
`oxide-computer-use install` wrote.

## Release assets

Publishing the draft release that
[`.github/workflows/release-drafter.yml`](../.github/workflows/release-drafter.yml)
keeps up to date is what releases oxide-computer-use: it creates the `v*` tag, and
[`.github/workflows/release.yml`](../.github/workflows/release.yml) stamps that
tag into the crate and attaches the archives below a few minutes later. The
stamp is why the binary in an archive reports the release it came from;
`oxide-computer-use --version` prints the tag, not the `Cargo.toml` a checkout was
built from. Until the workflow finishes, the release page shows the notes
without the assets, and the installer reports that it found no release manifest.

| Platform | Archive | Checksum |
| --- | --- | --- |
| macOS, Apple Silicon | `oxide-computer-use-macos-arm64.tar.gz` | `.tar.gz.sha256` |
| macOS, Intel | `oxide-computer-use-macos-x64.tar.gz` | `.tar.gz.sha256` |

Each archive holds one file, `oxide-computer-use`. No asset carries its version in the
name, so the newest archive of a platform keeps one name from release to
release; the release page is where the version is written down. The
`oxide-computer-use-manifest` asset lists the archive for each platform token
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
install -m 755 target/release/oxide-computer-use ~/.local/bin/oxide-computer-use
```

`cargo install --path .` works too and puts the binary in `~/.cargo/bin`. A
checkout reports the version in `Cargo.toml`; only a release tag rewrites that,
and only inside CI, so a local build never claims to be a published release.

## Then

```sh
oxide-computer-use doctor                         # Accessibility, Screen Recording, policy
oxide-computer-use install --client codex         # or oxide, claude, cursor, desktop
```

Permission attribution depends on how the server is launched. For direct stdio,
start the agent from the terminal or app you granted, then call `permissions`
through that MCP connection. Oxide's optional [native helper](oxide-helper.md)
launches the runtime under its own app bundle; it requires separate grants.
`install.sh` and `oxide-computer-use install` continue to install/register the standalone
runtime, not the helper. See
[docs/configuration.md](configuration.md) for the policy file. Registration
names, including the clients that already ship their own Computer Use, are in
[docs/clients.md](clients.md).
