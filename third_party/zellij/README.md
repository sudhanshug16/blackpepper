# Blackpepper Zellij sidecar

Blackpepper needs two terminal-transport changes that are not in Zellij
0.44.3: bounded OSC 9/777 forwarding and host focus-event delivery. It also
backports Zellij's merged foreground-pane command-discovery fix so tab creation
does not time out while scanning unrelated host processes. The corresponding
upstream changes are:

- [zellij-org/zellij#5099](https://github.com/zellij-org/zellij/pull/5099)
- [zellij-org/zellij#5456](https://github.com/zellij-org/zellij/pull/5456)
- [zellij-org/zellij#5324](https://github.com/zellij-org/zellij/pull/5324)

`source.env` pins the exact upstream release commit and records the upstream
change commits used to prepare these backports. `PATCHES.sha256` protects
the checked-in patch set from an unnoticed local rewrite. `ARTIFACTS.sha256`
records the published archives, extracted executables, and shared license
bundle. The build-tool patch pins the two tools that Zellij's release task
installs from crates.io and makes every Cargo build honor `Cargo.lock`. The
workflow pins every action, the Protobuf compiler, and Linux Cross images; each
artifact carries the resolved toolchain and image provenance. The published
binary reports `zellij 0.44.3-blackpepper.2`; this distinct identity is required
so a stock binary on `PATH` or in Blackpepper's existing `0.44.3` cache can
never silently satisfy a new patched session.

The workflow also builds `cargo-about` 0.9.1 from its checksum-pinned crate and
bundled lockfile. The checked-in `licenses/about.toml` and `licenses/about.hbs`
generate one deterministic `LICENSES.html` covering Zellij and the locked
workspace dependency graph for all four native targets and the embedded WASM
plugins. Checksum-backed clarifications add the upstream notices for native
libcurl, nghttp2, zlib, OpenSSL, and AWS-LC sources compiled by their Rust
`-sys` crates. A checked-in assembler then adds the pinned Rust standard
library notice plus musl and LLVM libunwind notices for the self-contained
Linux runtime. The workflow fails if an input checksum drifts or a required
notice is absent. Each runtime archive contains that complete notice next to
`zellij`.

The patches retain Zellij's upstream `contract_version_1`. Existing sessions
therefore remain interoperable with ordinary Zellij clients, while
Blackpepper's recorded backend version keeps old stock sessions on their
retained stock binary. The active fork does not rewrite running sessions;
users must terminate and reopen an old workspace before it gains the new
transport.

## Published release and rebuilds

`.github/workflows/zellij-sidecar.yml` is manual and defaults to artifact-only.
It checks out the exact upstream commit, verifies and applies these patches,
runs the focused tests, builds all four supported targets, and uploads archives
plus checksums and provenance as workflow artifacts. Its separately confirmed
`publish` input verifies the complete set and creates a dedicated prerelease;
it never changes Blackpepper's production runtime manifest.

The four target archives are published and active under the dedicated
prerelease tag
[`zellij-v0.44.3-blackpepper.2`](https://github.com/sudhanshug16/blackpepper/releases/tag/zellij-v0.44.3-blackpepper.2).
Blackpepper's runtime manifest pins that tag's release URLs plus the archive,
extracted-binary, and license SHA-256 values. A changed asset therefore fails
checksum verification instead of being accepted silently.

The runtime manifest keeps the upstream `0.44.3` and earlier `.1` assets
addressable for recorded sessions while selecting `.2` for new sessions.

New sessions use the current branded runtime. An existing workspace remains on
its recorded runtime, including stock `0.44.3` or the earlier `.1` generation,
until the user runs `:workspace terminate` and reopens it by pressing `Enter`
(or with `:workspace switch`). Termination ends the old session but preserves
the registered folder. The publisher refuses to reuse an existing tag or
release; do not replace the upstream `0.44.3` assets in place. A later patched
generation must again publish first and activate its new version and checksums
in a separate reviewed change.

The runtime and rollout invariants are recorded in
[`docs/private-zellij-runtime.md`](../../docs/private-zellij-runtime.md).
