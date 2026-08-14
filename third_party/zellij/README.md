# Blackpepper Zellij sidecar

Blackpepper needs two terminal-transport changes that are not in Zellij
0.44.3: bounded OSC 9/777 forwarding and host focus-event delivery. The
corresponding upstream pull requests are still open:

- [zellij-org/zellij#5099](https://github.com/zellij-org/zellij/pull/5099)
- [zellij-org/zellij#5456](https://github.com/zellij-org/zellij/pull/5456)

`source.env` pins the exact upstream release commit and records the upstream
pull-request heads used to prepare these backports. `PATCHES.sha256` protects
the checked-in patch set from an unnoticed local rewrite. The build-tool patch
pins the two tools that Zellij's release task installs from crates.io and makes
every Cargo build honor `Cargo.lock`. The workflow pins every action, the
Protobuf compiler, and Linux Cross images; each artifact carries the resolved
toolchain and image provenance. The patched binary reports
`zellij 0.44.3-blackpepper.1`; this distinct identity is required so a stock
binary on `PATH` or in Blackpepper's existing `0.44.3` cache can never silently
satisfy a new patched session.

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
retained stock binary. Activating the fork does not rewrite running sessions;
users must terminate and recreate an old session before it gains the new
transport.

## Build and activation

`.github/workflows/zellij-sidecar.yml` is manual and defaults to artifact-only.
It checks out the exact upstream commit, verifies and applies these patches,
runs the focused tests, builds all four supported targets, and uploads archives
plus checksums and provenance as workflow artifacts. Its separately confirmed
`publish` input verifies the complete set and creates a dedicated prerelease;
it never changes Blackpepper's production runtime manifest.

Activation is deliberately a separate change after the four archives are
published under a new dependency tag. That change must update Blackpepper's
Zellij version, release URLs, and trusted archive SHA-256 values together. A
changed asset then fails checksum verification instead of being accepted
silently. The publisher refuses to reuse an existing tag or release. Do not
replace the existing `0.44.3` assets in place.

The runtime and rollout invariants are recorded in
[`docs/private-zellij-runtime.md`](../../docs/private-zellij-runtime.md).
