import fs from "node:fs";
import path from "node:path";

export const DEFAULT_TOGGLE_MODE = "ctrl+g";

export type Config = {
  keymap: {
    toggleMode: string;
  };
};

type RawConfig = {
  keymap?: {
    toggle_mode?: string;
    toggleMode?: string;
  };
};

function readTomlFile(filePath: string): RawConfig | null {
  if (!fs.existsSync(filePath)) {
    return null;
  }

  const raw = fs.readFileSync(filePath, "utf-8");
  if (!raw.trim()) {
    return null;
  }

  const parsed = Bun.TOML.parse(raw) as RawConfig;
  return parsed ?? null;
}

function mergeConfig(user: RawConfig | null, workspace: RawConfig | null): Config {
  const toggleMode =
    workspace?.keymap?.toggle_mode ??
    workspace?.keymap?.toggleMode ??
    user?.keymap?.toggle_mode ??
    user?.keymap?.toggleMode ??
    DEFAULT_TOGGLE_MODE;

  return {
    keymap: {
      toggleMode,
    },
  };
}

export function loadConfig(cwd = process.cwd()): Config {
  const workspacePath = path.join(cwd, ".config", "blackpepper", "pepper.toml");
  const homeDir = process.env.HOME ?? "";
  const userPath = homeDir ? path.join(homeDir, ".config", "blackpepper", "pepper.toml") : "";

  const userConfig = userPath ? readTomlFile(userPath) : null;
  const workspaceConfig = readTomlFile(workspacePath);

  return mergeConfig(userConfig, workspaceConfig);
}
