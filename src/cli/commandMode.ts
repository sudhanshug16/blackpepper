import { COMMANDS } from "@/cli/commands";

type CommandMatch = {
  ok: true;
  name: string;
  args: string[];
  raw: string;
};

type CommandError = {
  ok: false;
  error: string;
  raw: string;
};

export type CommandParseResult = CommandMatch | CommandError;

const commandNames = new Set(COMMANDS.map((command) => command.name));

export function parseCommand(input: string): CommandParseResult {
  const raw = input;
  const trimmed = input.trim();

  if (trimmed.length === 0) {
    return { ok: false, error: "Empty command", raw };
  }

  if (!trimmed.startsWith(":")) {
    return { ok: false, error: "Commands must start with ':'", raw };
  }

  const tokens = trimmed.slice(1).split(/\s+/).filter(Boolean);
  const [name, ...args] = tokens;

  if (!name) {
    return { ok: false, error: "Missing command name", raw };
  }

  if (!commandNames.has(name)) {
    return { ok: false, error: `Unknown command: ${name}`, raw };
  }

  return { ok: true, name, args, raw };
}

export function commandHelpLines(): string[] {
  const longest = COMMANDS.reduce((max, command) => Math.max(max, command.name.length), 0);
  return COMMANDS.map((command) => {
    const padded = command.name.padEnd(longest, " ");
    return `:${padded} ${command.description}`;
  });
}
