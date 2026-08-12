# Design source

Pulled from the Claude Design project
`https://claude.ai/design/p/7f7b577d-e6a8-478f-b0a5-6c3178fa9275`,
committed here so agents and CI have a stable path rather than a live fetch.

| File | What it is |
|---|---|
| `blackpepper-logo-and-tui-v2.dc.html` | The current spec. Sections `1a` logo, `1b` TUI, `3a` accent alternatives (the seven palettes), and the `Mobile` sections. **This is the one to read.** |
| `blackpepper-tui-current.dc.html` | The pre-v2 recreation, kept as the "before" reference. |

## Reading it

It is HTML standing in for terminal cells. Translate literally:

- `2ch` is 2 character columns; panel `padding: 12px 2ch` means 2 columns of
  inset on the left and right *inside* that panel.
- `margin-left: auto` means that span is right-aligned to the panel's inner edge.
- `margin: 0 -2ch` on a selected row means the highlight bleeds through the
  gutter to the full panel width.
- Spacer divs: `height: 14px`/`18px` at `line-height: 20px` are one blank row;
  `6px`/`10px` are sub-row and collapse to none.
- Colours are exact at full colour depth. The `16-color floor` panel shows the
  *degraded* values, which are deliberately different — do not read those as
  the main palette.

`support.js` is the Claude Design canvas runtime and is **not** committed; it is
only needed to render the `.dc.html` in a browser, not to read the spec. Fetch
it from the project if you want a live preview.

## Implementation notes

- Palettes live in `crates/blackpepper/src/client_config/theme.rs`.
- Rendering tokens live in `crates/blackpepper/src/client/render/style.rs`.
- The shared 2ch gutter and right-alignment helpers are in
  `crates/blackpepper/src/client/render/chrome.rs`.
- Parity assertions are in
  `crates/blackpepper/src/client/render/tests/compliance.rs`.
