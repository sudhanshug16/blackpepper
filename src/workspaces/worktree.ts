import path from "node:path";

const WORKSPACE_ROOT = "workspaces";

export function isValidWorkspaceName(name: string): boolean {
  return /^[a-z][a-z-]*$/.test(name);
}

export function workspacePath(name: string): string {
  if (name.includes("/") || name.includes("\\")) {
    throw new Error("Workspace name must not include path separators.");
  }

  return path.join(WORKSPACE_ROOT, name);
}

export function buildCreateWorktreeCommand(name: string, baseRef = "HEAD"): string[] {
  return ["git", "worktree", "add", workspacePath(name), "-b", name, baseRef];
}

export function buildRemoveWorktreeCommand(name: string): string[] {
  return ["git", "worktree", "remove", workspacePath(name)];
}

export function buildPruneWorktreeCommand(): string[] {
  return ["git", "worktree", "prune"];
}

export function formatCommand(parts: string[]): string {
  return parts.join(" ");
}
