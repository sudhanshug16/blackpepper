# Remote-first V1 roadmap

This is an implementation checklist, not a release promise. Checked items have
a code path and targeted tests; they may still need end-to-end acceptance.

## V2 terminal renderer

The [v2 terminal design system](design-system-v2.md) changes presentation, not
the V1 runtime or parser surface. The supplied design board is reference work,
not a shipped-runtime screenshot.

- [x] Record the exact terminal mark, borderless layout rules, single-accent
  palette, public status vocabulary, and semantic acceptance anchors.
- [x] Ship borderless `HOSTS` / `SESSION` / `PORTS` surfaces with one stable
  `bp  blackpepper` anchor across Manage and terminal views.
- [x] Ship the public `· idle`, `▸ running`, `! asks`, `✓ done`, `× exited`,
  and `? unsure` vocabulary without changing provider/storage protocol states.
- [x] Cover exact/default and custom surface tokens plus truecolor, 256-color,
  16-color, and `NO_COLOR` behavior with renderer/config unit tests.
- [ ] Implement state-grounded command argument completion; never offer an
  argument the current parser/action cannot execute.
- [ ] Complete representative live PTY and visual acceptance on supported
  macOS and Linux clients before describing v2 as cross-platform accepted.

## Wired

- [x] Stable host, workspace, repository, session, pane, and agent-run IDs.
- [x] Private SQLite WAL host/session registry and separate production/dev
  per-user process locks with owner PID reporting; shared lifecycle locks keep
  cross-channel mutations serialized.
- [x] Local and SSH exec/PTY/forward transport with a foreground OpenSSH
  ControlMaster and fail-closed child channels.
- [x] Manual host registration and explicit literal-alias SSH config preview.
- [x] Transient, versioned JSON-lines `bp-host` protocol and remote registry
  discovery.
- [x] Zellij 0.44.3 attach/create/terminate and multiple-client safety rules.
- [x] Zellij and Worktrunk pinned sidecar download, checksum verification,
  versioned cache, and atomic Linux upload.
- [x] Folder workspaces plus automatic Git repository grouping and persistent
  ungroup.
- [x] Worktrunk 0.72.0 schema-2 list, create/open/remove, exact-plan `:approve`
  with rechecked persistent project-command approvals, repository mutation
  lock, setup-failed state, and no automatic retry after an ambiguous
  disconnect.
- [x] Host-owned Worktrunk removal journal with stable-ID/path/common-dir
  validation and list-time, no-retry crash reconciliation.
- [x] Host-owned workspace lifecycle lease serializing session restoration,
  attach readiness, background service/agent tabs, termination, and worktree
  removal across multiple laptops.
- [x] Linux `ss` and macOS `lsof` listener discovery with visible partial
  results and PID/cwd workspace attribution.
- [x] Manual loopback forwarding with same-port preference and explicit
  cancellation.
- [x] Manual SSH reconnect restores forwards on the exact original local port
  or reports a visible conflict.
- [x] Launch-scoped Codex, Claude Code, and OpenCode adapter builders, provider
  preflight/health gate, edge-only OpenCode heartbeat freshness, redacted event
  storage, and per-client completion cursors.
- [x] Pinned host-local blocker engine that can only add/clear a needs-input
  overlay and emits redacted transitions.
- [x] Standalone TUI with Host → Repository → Workspace navigation and native
  Zellij terminal behavior.
- [x] Keep SSH bootstrap/reconnect, periodic observation, sidecar work,
  provider health checks, Worktrunk operations, Zellij lifecycle actions, and
  port forwarding off the render thread with host-scoped progress and
  cancellation.
- [x] Side-by-side `bp-dev` installer with private exact-build helper bundles;
  development never replaces or remotely reuses production `bp`/`bp-host`.
- [x] Loopback-only, one-client browser development PTY using checksum-pinned
  ttyd, per-launch authentication, an immutable `bp-dev` bundle path, and no
  release-install impact.
- [x] Require formatting, tests, warning-free Clippy, binary builds, installer
  checks, browser-harness checks, and local/SSH TUI acceptance before automatic
  version tags or release packaging.

## Required before V1 release

- [x] Bundle native `bp-host` plus static Linux x86_64 and arm64 helpers in
  every client release; install the checksum-verified V1 layout atomically.
- [x] Start, supervise, and stop the host-local blocker subscription for each
  live agent pane from the interactive client.
- [x] Supervise provider tab/process exit so Ctrl-C becomes unknown and process
  termination becomes exited without depending on a final provider hook.
- [x] Restore registered workspace shells and configured services after host
  reboot, never agent conversations.
- [x] Execute `[[startup]]` services automatically or through `:service start`,
  and inject `[workspace.env]` into the initial shell, services, and providers.
- [x] Apply parsed `[ui]` colors in the V1 renderer.
- [x] Reject structured SSH host fields other than the OpenSSH `destination`
  alias.
- [x] Add a selectable one-click port action alongside `:forward`.
- [x] Discover workspace listeners automatically instead of waiting for
  `:ports`.
- [x] Locate and retain older managed Zellij sidecars when an existing session
  records a version older than the current release pin.
- [x] Rediscover persisted live agent-run IDs after a client restart so their
  host snapshots appear without spawning a new run.
- [x] Validate the effective Zellij configuration without changing it, avoid
  injected shortcuts, and force client close to detach rather than terminate
  the persistent session.
- [x] Complete live direct SSH acceptance for first-use host keys, reconnect,
  parallel PTY/exec/forward channels, stdin/cancellation, managed sidecars, and
  dead/fake mux sockets.
- [x] Complete live ProxyJump, encrypted-key passphrase, and changed-host-key
  acceptance on Linux, including proof that mux children cannot connect
  directly if their owned socket is missing.
- [ ] Complete live password and FIDO/security-key prompt acceptance on
  supported client platforms.
- [ ] Complete end-to-end Zellij, Worktrunk, provider, privacy, reboot, and port
  acceptance tests on macOS and Linux.
- [ ] Remove or archive uncompiled tmux-era Rust modules once migration fixtures
  no longer need them.

## Deferred after V1

- Zellij plugin mode and verified client-private focus controls.
- Cross-Unix-user Zellij collaboration.
- PR creation/merge, branch renaming, and Blackpepper-owned forge workflows.
- Automatic worktree creation when an agent starts.
- Automatic resumption of an agent conversation after reboot.
- tmux compatibility, a Ghostty parser migration, and remote desktop/video
  streaming.
