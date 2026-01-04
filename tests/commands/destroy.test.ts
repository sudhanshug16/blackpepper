import fs from "node:fs/promises";
import { expect, test } from "bun:test";
import { runCommand } from "@/cli/commandRunner";
import { initRepo } from "../helpers/gitRepo";

test("destroy requires a name and errors for missing worktrees", async () => {
  const repoRoot = await initRepo();

  try {
    const usageResult = await runCommand("destroy", [], { cwd: repoRoot });
    expect(usageResult.ok).toBe(false);
    expect(usageResult.message).toContain("Usage: :destroy <animal>");

    const missingResult = await runCommand("destroy", ["ghost"], { cwd: repoRoot });
    expect(missingResult.ok).toBe(false);
    expect(missingResult.message).toContain("Failed to remove workspace 'ghost'");
  } finally {
    await fs.rm(repoRoot, { recursive: true, force: true });
  }
});
