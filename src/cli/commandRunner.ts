import { commandHelpLines } from "./commandMode";
import {
  buildCreateWorktreeCommand,
  buildRemoveWorktreeCommand,
  formatCommand,
  isValidWorkspaceName,
} from "../workspaces/worktree";

type CommandResult = {
  ok: boolean;
  message: string;
};

export function runCommand(name: string, args: string[]): CommandResult {
  switch (name) {
    case "help":
      return { ok: true, message: commandHelpLines().join("\n") };
    case "create": {
      const name = args[0];
      if (!name) {
        return { ok: false, message: "Usage: :create <animal>" };
      }
      if (!isValidWorkspaceName(name)) {
        return { ok: false, message: "Workspace name must be lowercase letters or dashes." };
      }
      const command = buildCreateWorktreeCommand(name);
      return { ok: true, message: `Dry run: ${formatCommand(command)}` };
    }
    case "destroy": {
      const name = args[0];
      if (!name) {
        return { ok: false, message: "Usage: :destroy <animal>" };
      }
      if (!isValidWorkspaceName(name)) {
        return { ok: false, message: "Workspace name must be lowercase letters or dashes." };
      }
      const command = buildRemoveWorktreeCommand(name);
      return { ok: true, message: `Dry run: ${formatCommand(command)}` };
    }
    case "create-pr":
      return { ok: true, message: "PR creation is not implemented yet." };
    case "open-pr":
      return { ok: true, message: "PR opening is not implemented yet." };
    case "merge-pr":
      return { ok: true, message: "PR merge is not implemented yet." };
    default:
      return { ok: false, message: `Unhandled command: ${name} ${args.join(" ")}`.trim() };
  }
}
