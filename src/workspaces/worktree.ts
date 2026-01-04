import { joinPath } from "@/lib/path";

const WORKSPACE_ROOT = "workspaces";

export function isValidWorkspaceName(name: string): boolean {
  return /^[a-z][a-z-]*$/.test(name);
}

export function workspacePath(name: string): string {
  if (name.includes("/") || name.includes("\\")) {
    throw new Error("Workspace name must not include path separators.");
  }

  return joinPath(WORKSPACE_ROOT, name);
}

export function workspaceRootPath(repoRoot: string): string {
  return joinPath(repoRoot, WORKSPACE_ROOT);
}

export async function ensureWorkspaceRoot(repoRoot: string): Promise<void> {
  const markerPath = joinPath(workspaceRootPath(repoRoot), ".pepper-keep");
  await Bun.write(markerPath, "", { createPath: true });
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
