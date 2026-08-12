# Blackpepper v2 terminal design system

**Status:** Implemented renderer contract. Live cross-platform visual
acceptance remains in progress.

**Scope:** Visual language, terminal layout, public status words, and acceptance
anchors. It does not change Blackpepper's remote-first V1 runtime model or add
commands.

![Blackpepper v2 design board (design direction, not a shipped runtime screenshot)](assets/blackpepper-v2-design-board.webp)

*Blackpepper v2 design board (design direction, not a shipped runtime
screenshot). The pictured version, branch, PR, ports, elapsed time, test count,
and command results are illustrative data.*

## Product mark

The terminal mark is exactly four rows:

```text
█
█▀▄  █▀▄
█▄▀  █▄▀
     █
```

It uses only full/half blocks and spaces. It must remain legible without color
and must not depend on a branded font. The brand wordmark is lowercase
`blackpepper`; graphic/web treatments may accent only `pepper`. The terminal
anchor instead accents `bp` and keeps `blackpepper` neutral.

The square app icon is a grinder plate: four dark holes on a Peppercorn field.
The supplied board defines 72 px, 32 px, and 16 px treatments. Those bitmap and
web assets are separate from the terminal mark.

## Principles

1. **Borderless hierarchy.** Surfaces are separated by shade, spacing, and dim
   uppercase labels. Rounded or bright panel outlines are not part of v2.
2. **One stable anchor.** Manage and terminal views use the same left-aligned
   `bp  blackpepper` anchor so changing modes does not move the user's visual
   reference.
3. **One accent.** Peppercorn is reserved for the terminal mark, `bp` anchors,
   mode badge, and command prompt colon. Selection uses reverse video so it
   remains distinct at every color tier.
4. **Terminal-owned behavior.** Zellij still owns its terminal content,
   keybindings, panes, tabs, scrollback, search, selection, and copy.
5. **Words describe user meaning.** Public status uses six short words. Storage
   and provider protocol names do not leak into the navigation UI.
6. **Color is optional.** Shape, words, spacing, and reverse video carry every
   important distinction when color is reduced or disabled.

## Color and surface tokens

| Token | Value or rule | Use |
| --- | --- | --- |
| Peppercorn | `#E4834F`; `oklch(.70 .14 45)` | Terminal mark, `bp` anchors, mode badge, command colon |
| Canvas | Configured terminal/UI background | Terminal and central session surface |
| Ink | Configured terminal/UI foreground | Primary text |
| Raised surface | `#232427` by default; derived from the configured Canvas and Ink for custom themes | Header, footer, host rail, port rail |
| Muted | ANSI dark gray, or dim under `NO_COLOR` | Labels, hints, secondary metadata |
| Selection | Reverse video | Selected row at every color tier |

The default Canvas/Ink pair is `#1c1d1f` / `#e6e4e1`, with an exact default
Raised value of `#232427`. For custom `[ui].background` and `[ui].foreground`
values, the renderer derives a corresponding Raised surface instead of
retaining the default dark shade. No border blend is required in v2.

The renderer chooses one tier in this order: `NO_COLOR`, then truecolor/24-bit
`COLORTERM`, then a `TERM` containing `256color`, then ANSI 16 colors.

Current color degradation:

| Terminal capability | Renderer behavior |
| --- | --- |
| Truecolor | Exact configured RGB; Peppercorn is `#E4834F` |
| 256 colors | Configured RGB maps to the nearest xterm color; Peppercorn is 173 |
| 16 colors | Configured RGB maps to the nearest ANSI color; Peppercorn is yellow |
| `NO_COLOR` | No foreground/background SGR; bold accent and reverse selection retain meaning |

Unit tests cover tier precedence, the exact default tokens, custom surface
derivation, all four accent treatments, and status/selection meaning under
`NO_COLOR`. At the ANSI-16 floor the Raised shade may merge with Canvas; labels,
spacing, and selection still carry hierarchy. Live PTY and visual coverage
remains a separate acceptance gate.

## Manage layout

The full-width header is one row:

```text
bp  blackpepper  <host>:<workspace-path>                  v<version>
```

The version comes from the running binary. The host/path context truncates when
space is tight. Branch and PR data pictured on the board are illustrative and
are not synthesized by the current renderer.

The body has three borderless surfaces:

- `HOSTS`: host → repository → workspace navigation; 32 columns in wide and
  medium layouts.
- `SESSION`: the flexible center area containing the current Zellij terminal
  view or a truthful empty-state instruction; at least 40 columns in the wide
  layout and 30 in the medium layout.
- `PORTS`: discovered listeners and client-local forward state; 30 columns in
  the wide layout.

Section names are dim uppercase labels inside their surfaces. Background shade
and whitespace separate the columns; there are no surrounding box glyphs.

The footer begins with an accent-inverted ` MANAGE ` badge, followed by compact
controls or transient output.

At 102 columns and above, all three surfaces appear side by side. From 62 to
101 columns, `PORTS` moves below `SESSION` when height allows. Below 62 columns,
the selector occupies up to six rows above `SESSION` and the port rail is
hidden; `:ports` and `:forward` remain available. Focused approval,
authentication, and detail views use the session/port space. Unit tests cover
the 102, 100, 80, 66, 65, 62, and sub-62-column cases.

## Terminal view

The workspace terminal uses the whole window except for one raised status row.
There is no public `WORK` badge. The row starts with:

```text
bp  blackpepper
```

It may then show the current agent's compact public state. Workspace switching
and Manage shortcuts stay at the right. Terminal output must never be parsed as
a mode marker; acceptance uses the Blackpepper-owned status row.

Manage-only `HOSTS` and `PORTS` surfaces must not consume terminal-view space.
Other input and terminal output pass through to Zellij unchanged.

## Public status vocabulary

| Internal display state | Public form | Meaning |
| --- | --- | --- |
| no run / provider ready | `· idle` | No active turn |
| working | `▸ running` | Authoritative provider activity |
| needs input | `! asks` | User action is required; the only nagging state |
| done | `✓ done` | Turn completed; seen state remains client-local |
| exited | `× exited` | Provider pane/process is gone; no relaunch implied |
| unknown | `? unsure` | Authority is incomplete; inspect `:status explain` |

`ready`, `working`, and `input` remain valid internal/provider event names where
the protocol requires them. They are not public navigation labels. A host-local
screen rule may contribute only to `asks`; it cannot establish `running` or
`done`.

## Connection and activity glyphs

The renderer's connection/activity glyph budget is:

```text
● ○ ◐ ◆ ▸ ! ✓ × ? ⚠
```

- `●` connected/local; `○` disconnected; `◐` connecting/authenticating;
  `◆` authentication required.
- `▸`, `!`, `✓`, `×`, and `?` belong to agent status.
- `⚠` marks an actionable warning.

## First run and empty state

When space permits, first run shows the four-row terminal mark, the running
Blackpepper version, and only actions that the current parser supports. For a
launch folder that was registered successfully, the current content is:

```text
<workspace-path> is registered.
enter  open this workspace
:host add <name> <alias>
```

With no registered workspace it instead shows `No workspaces registered.`,
`:workspace add <path>`, and the same host command. Example Zellij/Worktrunk
versions from the design board are illustrative, not emitted product state.

## Commands, help, and completion

The `:` prompt owns one Peppercorn accent. Help may group commands by current
workspace, repository, and hosts, but it may list only parser-supported
commands:

```text
:host add <name> <ssh-alias>
:host import
:host connect <name>
:host disconnect <name>
:workspace add <path>
:workspace switch <name|id>
:workspace ungroup
:workspace terminate
:worktree list
:worktree create <branch> [--base <ref>]
:worktree open <branch|pr:123|url>
:worktree remove
:agent spawn <codex|claude|opencode>
:service start <name>
:ports [--all-host]
:forward <port|address:port>
:forward cancel <port|address:port>
:status explain
:approve
:refresh
:help
:quit
:q
```

State gating dims a command with a reason only if parsing it would otherwise be
valid. Completion is progressive: command paths narrow first, then the prompt
names one missing argument at a time. Observed hosts, workspaces, services,
providers, listeners, and active forwards become selectable argument rows.
Inserted values are shell-quoted when needed, and parse errors remain beside the
editable prompt.

## Pointer behavior

Every visible Blackpepper action has a hit target built from the current frame;
responsive layout and scrolling therefore cannot leave invisible controls. A
workspace row selects it, the Manage-mode session enters Work mode, and the
Work-mode status row returns to Manage. Picker rows attach, completion and help
rows prefill commands, port rows forward or explain their current forward, and
the wheel acts on the panel under the pointer. Overlays capture pointer input so
clicks cannot leak into the session behind them.

The Work-mode session remains terminal-transparent. Blackpepper captures only
its one status row; mouse input inside the viewport still follows the embedded
terminal's requested protocol. If the child disables mouse reporting,
Blackpepper temporarily captures pointer reports only to keep its status row
clickable and drops viewport reports instead of sending unexpected bytes.

## Reviews, authentication, and errors

A Worktrunk review leads with `worktrunk will mutate this repository` and uses
the stable semantic labels `repository`, `mutation`, `unapproved project
hooks`, and `:approve`. It shows the exact argv and hook plan and states that
approval binds to both. `Esc` dismisses without approving or running anything.

Authentication copy must state that OpenSSH owns authentication and that
Blackpepper does not store credentials. Actual host-key, password, passphrase,
and security-key prompts remain OpenSSH output; the renderer must not imitate
or pre-answer them.

Errors lead with the failed object/action and one recovery step. Sensitive
clipboard, authentication, provider, and terminal payloads never appear in
footer diagnostics.

## Acceptance contract

Tests should assert Blackpepper-owned semantic anchors, not borders, hard-coded
coordinates, or decorative sample copy:

- Manage: ` MANAGE ` and the `HOSTS`, `SESSION`, and `PORTS` labels when the
  viewport is wide enough.
- Terminal: the owned `bp  blackpepper` row and absence of Manage-only rails;
  workspace/status assertions remain separate because transient output can
  temporarily occupy the rest of that row.
- Status: exact public forms `· idle`, `▸ running`, `! asks`, `✓ done`,
  `× exited`, and `? unsure`, while fixtures separately verify the unchanged
  internal provider state.
- Approval/auth: the stable semantic labels above, not capitalization or box
  glyphs.
- Transparency: native Zellij scroll/search/copy, resize propagation, and
  bounded OSC 52 behavior remain unchanged.

Visual snapshots may check shade and accent placement, but protocol tests must
continue to prove behavior without relying on color.

## Explicit non-claims

The design board does not add PR create/merge, branch rename, automatic
worktree creation, agent-conversation restoration, a Zellij plugin, or remote
desktop streaming. It does not prove password,
FIDO/security-key, macOS, or live visual acceptance. Those stay tracked in the
roadmap until their own gates pass.
