import { joinPath, normalizePath } from "@/lib/path";

type GitResult = {
  stdout: string;
  stderr: string;
};

export async function runGit(args: string[], cwd: string): Promise<GitResult> {
  const proc = Bun.spawn(["git", ...args], { cwd, stdout: "pipe", stderr: "pipe" });
  const stdoutPromise = proc.stdout ? new Response(proc.stdout).text() : Promise.resolve("");
  const stderrPromise = proc.stderr ? new Response(proc.stderr).text() : Promise.resolve("");
  const [stdout, stderr, exitCode] = await Promise.all([stdoutPromise, stderrPromise, proc.exited]);

  if (exitCode !== 0) {
    throw new Error(`git ${args.join(" ")} failed: ${(stderr || stdout).trim()}`);
  }

  return { stdout, stderr };
}

export async function initRepo(): Promise<string> {
  const tmpRoot = Bun.env.TMPDIR ?? Bun.env.TEMP ?? Bun.env.TMP ?? "/tmp";
  const dir = joinPath(tmpRoot, `blackpepper-${crypto.randomUUID()}`);
  await runGit(["init", dir], tmpRoot);
  await runGit(["config", "user.email", "test@example.com"], dir);
  await runGit(["config", "user.name", "Test User"], dir);
  await Bun.write(joinPath(dir, "README.md"), "seed", { createPath: true });
  await runGit(["add", "README.md"], dir);
  await runGit(["commit", "-m", "init"], dir);
  return dir;
}

export async function listWorktreePaths(repoRoot: string): Promise<string[]> {
  const { stdout } = await runGit(["worktree", "list", "--porcelain"], repoRoot);
  const worktreePaths = stdout
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.startsWith("worktree "))
    .map((line) => line.slice("worktree ".length));

  return worktreePaths.map((worktreePath) => normalizePath(worktreePath));
}

export async function branchExists(repoRoot: string, name: string): Promise<boolean> {
  try {
    await runGit(["show-ref", "--verify", "--quiet", `refs/heads/${name}`], repoRoot);
    return true;
  } catch {
    return false;
  }
}
