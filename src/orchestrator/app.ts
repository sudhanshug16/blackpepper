import {
  createCliRenderer,
  InputRenderable,
  InputRenderableEvents,
  TextRenderable,
} from "@opentui/core";
import { parseCommand } from "../cli/commandMode";
import { runCommand } from "../cli/commandRunner";
import { matchesChord, parseKeyChord } from "../cli/keymap";
import { DEFAULT_TOGGLE_MODE, loadConfig } from "../config/loadConfig";
import { buildLayout } from "./layout";

const COMMAND_INPUT_ID = "command-input";
const COMMAND_OUTPUT_ID = "command-output";
const COMMAND_OUTPUT_SEP_ID = "command-output-sep";
const MODE_INDICATOR_ID = "mode-indicator";
const SIDEBAR_ID = "sidebar";
const WORK_AREA_ID = "work-area";

type Mode = "normal" | "control";

export async function runApp() {
  const renderer = await createCliRenderer({ exitOnCtrlC: true });
  renderer.root.add(buildLayout());

  const commandInput = renderer.root.findDescendantById(COMMAND_INPUT_ID);
  const commandOutput = renderer.root.findDescendantById(COMMAND_OUTPUT_ID);
  const commandOutputSep = renderer.root.findDescendantById(COMMAND_OUTPUT_SEP_ID);
  const modeIndicator = renderer.root.findDescendantById(MODE_INDICATOR_ID);
  const sidebar = renderer.root.findDescendantById(SIDEBAR_ID);
  const workArea = renderer.root.findDescendantById(WORK_AREA_ID);

  if (!(commandInput instanceof InputRenderable)) {
    throw new Error("Command input renderable not found.");
  }

  if (!(commandOutput instanceof TextRenderable)) {
    throw new Error("Command output renderable not found.");
  }

  if (!(commandOutputSep instanceof TextRenderable)) {
    throw new Error("Command output separator renderable not found.");
  }

  if (!(modeIndicator instanceof TextRenderable)) {
    throw new Error("Mode indicator renderable not found.");
  }

  if (!sidebar) {
    throw new Error("Sidebar renderable not found.");
  }

  if (!workArea) {
    throw new Error("Work area renderable not found.");
  }

  let mode: Mode = "normal";
  let commandLineOpen = false;
  const config = loadConfig();
  const toggleChord = parseKeyChord(config.keymap.toggleMode) ?? parseKeyChord(DEFAULT_TOGGLE_MODE);

  const setMode = (nextMode: Mode) => {
    mode = nextMode;
    modeIndicator.content = mode === "normal" ? "-- NORMAL --" : "-- CONTROL --";
  };

  const setOutput = (message: string) => {
    const trimmed = message.trim();
    const hasOutput = trimmed.length > 0;

    commandOutputSep.visible = hasOutput;
    commandOutput.visible = hasOutput;
    commandOutput.content = trimmed;
  };

  const showCommandLine = (prefill = ":") => {
    commandLineOpen = true;
    commandInput.visible = true;
    modeIndicator.visible = false;
    commandInput.value = prefill;
    commandInput.cursorPosition = commandInput.value.length;
    commandInput.focus();
  };

  const hideCommandLine = () => {
    commandLineOpen = false;
    commandInput.visible = false;
    commandInput.value = "";
    commandInput.cursorPosition = 0;
    commandInput.blur();
    modeIndicator.visible = true;
  };

  const enterControlMode = () => {
    setMode("control");
    if (commandLineOpen) {
      hideCommandLine();
    }
  };

  const enterNormalMode = () => {
    setMode("normal");
    if (commandLineOpen) {
      hideCommandLine();
    }
  };

  hideCommandLine();
  setMode(mode);

  sidebar.onMouseDown = () => {
    enterControlMode();
  };

  workArea.onMouseDown = () => {
    enterNormalMode();
  };

  commandInput.on(InputRenderableEvents.ENTER, (value: string) => {
    const parsed = parseCommand(value);
    if (!parsed.ok) {
      setOutput(`Error: ${parsed.error}`);
      hideCommandLine();
      return;
    }

    const result = runCommand(parsed.name, parsed.args);
    setOutput(result.message);
    hideCommandLine();
  });

  renderer.keyInput.on("keypress", (key) => {
    if (toggleChord && matchesChord(key, toggleChord)) {
      key.preventDefault();
      if (mode === "normal") enterControlMode();
      else enterNormalMode();
      return;
    }

    if (commandInput.focused) {
      if (key.name === "escape") {
        key.preventDefault();
        hideCommandLine();
      }
      return;
    }

    if (key.name === "escape" && mode === "control") {
      key.preventDefault();
      enterNormalMode();
      return;
    }

    if (key.sequence === ":" && mode === "control") {
      key.preventDefault();
      showCommandLine(":");
    }
  });
}
