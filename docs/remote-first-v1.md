# Remote-first V1 migration

**Status:** Implemented; cross-platform acceptance remains in progress.
**Date:** 2026-08-11

## Decision

Blackpepper V1 is a standalone local/SSH client backed by Blackpepper Zellij
0.44.3-blackpepper.1 (based on upstream 0.44.3) and Worktrunk 0.72.0.

- Zellij owns sessions, panes, tabs, keybindings, scrolling, search, selection,
  and copy behavior.
- Worktrunk owns Git worktree listing, creation, switching, hooks, and removal.
- System OpenSSH owns authentication, host keys, agents, `Include`, `Match`, and
  jump-host behavior.
- Blackpepper owns folder registration, host/repository/workspace navigation,
  safe orchestration, agent status summaries, and client-local port forwards.
- A transient `bp-host` process keeps registry and status work on the workspace
  host; there is no persistent Blackpepper daemon.

Zellij clipboard writes remain OSC 52. Blackpepper accepts bounded UTF-8
writes, never answers clipboard reads, tries the client OS clipboard, and also
offers a normalized write to the outer terminal. Blackpepper reports a concise
success, outer-terminal handoff, or total failure without exposing clipboard
contents or transport diagnostics in the footer. The development ttyd harness
supplies its own write-only browser bridge because ttyd 1.7.7's embedded
xterm.js predates native OSC 52 support.

This replaces the tmux-centric design and the hand-written worktree/port/PR
surface. V1 does not use a Zellij plugin or Ghostty. A plugin can be considered
later only for client-private focus and richer integration.

The [v2 terminal design system](design-system-v2.md) is a presentation layer
over this same decision. Its borderless surfaces and public status words do not
add parser commands, move authority out of OpenSSH/Zellij/Worktrunk, or turn the
design-board examples into runtime claims.

## Why

The primary workspace may now be a home-lab VM rather than the laptop running
the UI. The chosen split keeps durable processes and repository state on that
VM, sends the incremental terminal byte stream plus compact state over SSH,
and reuses mature session/worktree tools instead of rebuilding their behavior.

## Migration from pre-V1

1. Back up any configuration you want to reference. Blackpepper does not edit
   old files, tmux sessions, worktrees, or the old TOML state.
2. Remove every `[tmux]` and `[tmux.*]` table. V1 rejects the file with an
   actionable error instead of ignoring those keys.
3. Register existing folders with `:workspace add <path>`. Starting `bp` also
   registers the current local folder.
4. Replace old workspace create/from-branch/from-PR commands with
   `:worktree create` or `:worktree open`, review the displayed Worktrunk
   mutation and project hook commands, then run `:approve`. Approval is bound
   to that exact plan. If Worktrunk has unapproved project commands, this also
   saves their persistent Worktrunk approval before the mutation runs; any
   plan change requires another review.
5. Remove `WORKSPACE_PORT_0` through `WORKSPACE_PORT_9` assumptions. Configure
   deterministic ports in the project or Worktrunk hooks; use `:ports` and
   `:forward <port|address:port>` for discovery and exact-socket access.
6. Remove PR create/merge and workspace rename automation. Those workflows are
   outside V1.
7. Move service definitions to `[[startup]]`. Entries with `auto_start = true`
   start when Blackpepper creates or recreates the workspace session; start any
   named entry with `:service start <name>`. `[workspace.env]` applies to the
   initial shell, configured services, and provider launches.

Old `~/.config/blackpepper/state.toml` data is not imported. New registries live
under `$XDG_STATE_HOME/blackpepper/` and use stable IDs rather than folder
basenames. Existing tmux sessions continue independently until the user ends
them.

## Compatibility and safety consequences

- Only macOS/Linux clients and Linux SSH workspace hosts are supported in V1.
- An exact Zellij/Worktrunk version is required; managed sidecars are pinned and
  checksum-verified rather than put on `PATH`.
- One production `bp` and one development `bp-dev` process are allowed per OS
  user. They share the stable host/workspace/session registry and lifecycle
  locks while keeping provider event stores channel-specific. Several clients
  may also attach to the same remote Zellij session.
- Disconnect means detach. Terminating a workspace session keeps the folder.
  Removing a worktree ends its session before Worktrunk removes the folder.
- Worktrunk mutations require visible approval of the exact mutation and any
  unapproved project hook commands. `:approve` persists those Worktrunk command
  approvals only after rechecking the reviewed plan; mutations never receive
  force or hook-skipping flags.
- Worktree removal is journaled and completed in the host registry by the same
  helper that invokes Worktrunk. After a crash or lost SSH response,
  `:worktree list` reconciles the marker from schema-2 output without retrying
  the removal.
- One host-owned lifecycle lease serializes session creation, verified attach,
  background tab creation, termination, and approved worktree removal for a
  workspace. A pending removal marker blocks session recreation until an
  authoritative Worktrunk list resolves it.
- Because the upstream Zellij 0.44.3 codebase's external tab creation steals a
  client's focus even with `focus=false`, Blackpepper creates service and agent
  tabs only when one client is attached, then restores that exact client's
  previous tab. Native Zellij tab controls remain available with several
  clients.
- Provider status data excludes prompts, responses, commands, tool content,
  and terminal text. Screen rules are a temporary needs-input hint, never the
  source of working or done state.

The blocker-only rules are conservative adaptations of Herdr's Apache-2.0
[agent manifests](https://herdr.dev/docs/agents/). Provider hooks and plugins
remain authoritative; the adapted rules only cover missed interactive prompts.

## Deferred

Zellij plugin mode, tmux compatibility, embedded library APIs for Zellij or
Worktrunk, PR/merge operations, cross-user collaboration, automatic worktree
creation, and agent-conversation restoration are not V1 requirements.
