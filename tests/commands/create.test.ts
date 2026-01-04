import fs from "node:fs/promises";
import { expect, test } from "bun:test";
import { runCommand } from "@/cli/commandRunner";
import { animalNames } from "@/lib/animals";
import { joinPath, normalizePath } from "@/lib/path";
import { isValidWorkspaceName } from "@/workspaces/worktree";
import { branchExists, initRepo, listWorktreePaths, runGit } from "../helpers/gitRepo";

const uniqueAnimalNames = (() => {
  const seen = new Set<string>();
  return animalNames.filter((name) => {
    if (!isValidWorkspaceName(name)) return false;
    if (seen.has(name)) return false;
    seen.add(name);
    return true;
  });
})();

test("create auto-picks an unused animal and registers a worktree", async () => {
  const repoRoot = await initRepo();

  try {
    const createResult = await runCommand("create", [], { cwd: repoRoot });
    expect(createResult.ok).toBe(true);
    const match = createResult.message.match(/Created workspace '([^']+)'/);
    expect(match).not.toBeNull();
    const workspaceName = match?.[1] ?? "";

    expect(isValidWorkspaceName(workspaceName)).toBe(true);
    expect(uniqueAnimalNames.includes(workspaceName)).toBe(true);

    const worktreePaths = await listWorktreePaths(repoRoot);
    const expectedSuffix = normalizePath(joinPath("workspaces", workspaceName));
    expect(worktreePaths.some((worktreePath) => worktreePath.endsWith(expectedSuffix))).toBe(true);

    const hasBranch = await branchExists(repoRoot, workspaceName);
    expect(hasBranch).toBe(true);

    const destroyResult = await runCommand("destroy", [workspaceName], { cwd: repoRoot });
    expect(destroyResult.ok).toBe(true);

    const remainingWorktrees = await listWorktreePaths(repoRoot);
    expect(remainingWorktrees.some((worktreePath) => worktreePath.endsWith(expectedSuffix))).toBe(
      false,
    );
  } finally {
    await fs.rm(repoRoot, { recursive: true, force: true });
  }
});

test("create rejects invalid workspace names", async () => {
  const repoRoot = await initRepo();
  const invalidNames = ["Bad", "bad_name", "bad/name", "bad\\name", "1bad", "bad123"];

  try {
    for (const name of invalidNames) {
      const result = await runCommand("create", [name], { cwd: repoRoot });
      expect(result.ok).toBe(false);
      expect(result.message).toContain("lowercase letters or dashes");
    }
  } finally {
    await fs.rm(repoRoot, { recursive: true, force: true });
  }
});

test("create fails outside a git repository", async () => {
  const tmpRoot = Bun.env.TMPDIR ?? Bun.env.TEMP ?? Bun.env.TMP ?? "/tmp";
  const dir = joinPath(tmpRoot, `blackpepper-nogit-${crypto.randomUUID()}`);

  try {
    const result = await runCommand("create", [], { cwd: dir });
    expect(result.ok).toBe(false);
    expect(result.message).toContain("Not inside a git repository");
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
});

test("create refuses when branch already exists", async () => {
  const repoRoot = await initRepo();

  try {
    await runGit(["branch", "otter"], repoRoot);
    const result = await runCommand("create", ["otter"], { cwd: repoRoot });
    expect(result.ok).toBe(false);
    expect(result.message).toContain("Branch 'otter' already exists");
  } finally {
    await fs.rm(repoRoot, { recursive: true, force: true });
  }
});
