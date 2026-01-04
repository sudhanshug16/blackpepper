import { joinPath } from "@/lib/path";

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

async function readTomlFile(filePath: string): Promise<RawConfig | null> {
  const file = Bun.file(filePath);
  if (!(await file.exists())) {
    return null;
  }

  const raw = await file.text();
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

export async function loadConfig(cwd = import.meta.dir): Promise<Config> {
  const workspacePath = joinPath(cwd, ".config", "blackpepper", "pepper.toml");
  const homeDir = Bun.env.HOME ?? "";
  const userPath = homeDir ? joinPath(homeDir, ".config", "blackpepper", "pepper.toml") : "";

  const [userConfig, workspaceConfig] = await Promise.all([
    userPath ? readTomlFile(userPath) : Promise.resolve(null),
    readTomlFile(workspacePath),
  ]);

  return mergeConfig(userConfig, workspaceConfig);
}
