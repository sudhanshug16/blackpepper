# Private Zellij runtime

## Decision

Blackpepper uses a small, pinned Zellij patch set until the required
notification and focus-transport changes ship upstream. The resulting `zellij`
executable remains a separate process. Blackpepper downloads it on first use to
its private XDG data directory, invokes it by absolute path, and never adds it
to `PATH` or replaces a user's Zellij installation.

The executable is not embedded in the `bp` binary or repeated in every
Blackpepper installer archive. A local client downloads only its native target.
When a Linux SSH host needs a different architecture, the client downloads that
target, verifies it, and uploads the binary and complete generated license
bundle to the host's private Blackpepper data directory. The remote host does
not need internet access.

## Required behavior

- New sessions use the branded runtime version
  `0.44.3-blackpepper.2` and cannot fall back to a same-version executable on
  `PATH`.
- Stock sessions keep the exact `bp-<workspace UUID>` backend name. Branded
  sessions append a stable short hash of their exact Zellij version, so a new
  generation cannot attach to a surviving stock or older branded server. Both
  use Zellij's standard socket namespaces, which keeps the recorded name
  discoverable by older Blackpepper clients.
- Existing session records retain their exact runtime, including stock
  `0.44.3` or the earlier `.1` generation and their downloadable assets. They
  are not silently migrated.
- Archives, extracted executables, and license files are checksum verified and
  stored under a version and target triple. Remote publication is atomic.
- A user moves a workspace to the current `.2` runtime by terminating its old
  Blackpepper session and attaching again. The registered folder is preserved.

## Distribution

The four published targets are Linux x86-64 and ARM64 (static musl), plus macOS
x86-64 and Apple Silicon. They are built from the exact source and patch pins
under `third_party/zellij/`.

The dedicated workflow is manually dispatched. Its default is build-only and
produces expiring Actions artifacts. Publication is a separate explicit input:
it verifies the complete four-target set and creates a new Blackpepper-owned
prerelease named `zellij-v0.44.3-blackpepper.2`. The release is never marked
latest, because the Blackpepper installer resolves its own release through
GitHub's `latest` URL. An existing dependency tag is never reused or
overwritten. The repository does not need GitHub's optional immutable-release
setting: activation pins every archive, executable, and license SHA-256, so
replacement fails closed and deletion produces an explicit download failure.

That prerelease is published and active. The runtime manifest pins its real
release URLs and archive, extracted-binary, and license SHA-256 values, while
[`third_party/zellij/ARTIFACTS.sha256`](../third_party/zellij/ARTIFACTS.sha256)
records the same published digests. New sessions now pin
`0.44.3-blackpepper.2`. Publication and activation remain separate reviewed
steps for later patched generations, so a missing or partial dependency
release cannot break workspace creation.

## Existing workspace migration

Activation does not rewrite a recorded stock or earlier branded session. To
move an existing workspace to the current `.2` runtime, select it, run
`:workspace terminate`, then reopen it by pressing `Enter` or running
`:workspace switch <name|id>`. The termination ends processes in the old
Zellij session but preserves the registered folder. Reopening creates its
shell and configured `auto_start` services; it does not restore agent
conversations.

## Validation

- Apply all patches to the exact upstream commit and reject any diff or hash
  drift.
- Run focused OSC, focus parser, client/server contract, and server delivery
  tests.
- Build all four targets and execute each branded binary's `--version` on its
  native architecture or pinned ARM64 emulation.
- Generate `LICENSES.html` offline from the locked workspace graph with the
  checksum-pinned `cargo-about` source, failing on an unknown or unaccepted
  license. The conservative workspace scan includes build-only workspace tools
  in addition to every native and embedded-WASM runtime dependency. Pinned
  clarifications include the nested libcurl, nghttp2, zlib, OpenSSL, and AWS-LC
  notices that Rust package metadata alone does not expose. The final assembly
  also includes the pinned Rust standard-library copyright report and the musl
  and LLVM libunwind notices for Rust's self-contained Linux runtime.
- Verify every archive contains only `zellij` and `LICENSES.html`, plus archive,
  binary, and license SHA-256 values and build provenance.
- Test that the private runtime ignores `PATH`, uses a version-isolated backend
  session name in the legacy socket namespace, preserves old recorded
  sessions, installs its license, and forwards a real notification/focus
  sequence end to end.
