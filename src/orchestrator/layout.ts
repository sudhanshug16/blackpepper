import { ASCIIFont, Box, Input, Text, TextAttributes } from "@opentui/core";

const outputSeparator = "-".repeat(28);

export function buildLayout() {
  return Box(
    { flexGrow: 1, flexDirection: "row", gap: 0, alignItems: "stretch" },
    Box(
      { id: "sidebar", width: 32, padding: 1, flexDirection: "column", gap: 1 },
      Text({ content: "Blackpepper", attributes: TextAttributes.BOLD }),
      Text({ content: "Workspaces", attributes: TextAttributes.BOLD }),
      Text({ content: "None yet.", attributes: TextAttributes.DIM }),
      Box(
        { flexGrow: 1, flexDirection: "column", justifyContent: "flex-end", gap: 1 },
        Text({
          id: "command-output-sep",
          content: outputSeparator,
          attributes: TextAttributes.DIM,
          visible: false,
        }),
        Text({
          id: "command-output",
          content: "",
          attributes: TextAttributes.DIM,
          wrapMode: "word",
          visible: false,
        }),
        Text({
          id: "mode-indicator",
          content: "-- NORMAL --",
          attributes: TextAttributes.DIM,
        }),
        Input({
          id: "command-input",
          width: "100%",
          height: 1,
          placeholder: ":create",
          focusedBackgroundColor: "#1a1a1a",
        }),
      ),
    ),
    Box({ width: 1 }),
    Box({
      width: 1,
      border: ["left"],
      customBorderChars: {
        topLeft: " ",
        topRight: " ",
        bottomLeft: " ",
        bottomRight: " ",
        horizontal: " ",
        vertical: "┆",
        topT: " ",
        bottomT: " ",
        leftT: " ",
        rightT: " ",
        cross: " ",
      },
    }),
    Box({ width: 1 }),
    Box(
      {
        id: "work-area",
        flexGrow: 1,
        padding: 1,
        flexDirection: "column",
        justifyContent: "center",
        alignItems: "center",
        gap: 1,
      },
      ASCIIFont({ font: "tiny", text: "Blackpepper" }),
      Text({ content: "Orchestrate agent tabs", attributes: TextAttributes.DIM }),
      Text({ content: "Ctrl+G: toggle Control mode", attributes: TextAttributes.DIM }),
      Text({ content: "':' opens command line (Control)", attributes: TextAttributes.DIM }),
    ),
  );
}
