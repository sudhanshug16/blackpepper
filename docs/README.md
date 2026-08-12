# Blackpepper documentation

- [Architecture](../ARCHITECTURE.md) — runtime boundaries, data flow, safety
  invariants, and persistence.
- [V2 terminal design system](design-system-v2.md) — borderless layout,
  terminal mark, color tiers, public status words, acceptance anchors, and the
  remaining live visual-acceptance gap.
- [Remote-first V1 migration](remote-first-v1.md) — the accepted move from tmux
  and hand-rolled worktrees to Zellij, Worktrunk, OpenSSH, and host registries.
- [V1 roadmap](roadmap.md) — what is wired, what still blocks a complete V1,
  and what is deliberately deferred.
- [Browser development terminal](web-dev.md) — loopback-only ttyd/xterm.js
  harness for interactive and browser-controlled TUI inspection.
- [macOS SSH PTY acceptance](macos-ssh-pty.md) — isolated, stdlib-only check
  for the Ghostty-compatible macOS-to-Linux terminal boundary, including the
  optional outer-SSH terminal guard and its hard recovery limit.
- `install.sh` — checksum-verifying release installer for the versioned `bp`,
  native `bp-host`, and Linux x86_64/arm64 remote-helper bundle.
- `../scripts/setup.sh` — native debug installer that publishes `bp-dev` with
  a private exact-build helper, without changing production `bp` or `bp-host`;
  `--list-builds` and guarded `--prune <build-id>` manage retained builds.
- `../scripts/web-dev.sh` — checksum-pinned development-only browser PTY; it is
  excluded from release and production installation.
- `../scripts/ssh-terminal-guard.sh` — non-installed POSIX wrapper for the
  fallback workflow where Linux `bp` runs inside an outer SSH session; native
  local `bp` with `:host connect` remains the default.

For current commands and configuration, start with the
[project README](../README.md).
