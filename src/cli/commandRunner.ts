import { commandHelpLines } from "@/cli/commandMode";
import {
  buildCreateWorktreeCommand,
  buildRemoveWorktreeCommand,
  ensureWorkspaceRoot,
  isValidWorkspaceName,
  workspacePath,
  workspaceRootPath,
} from "@/workspaces/worktree";
import { animalNames } from "@/lib/animals";
import { normalizePath } from "@/lib/path";

type CommandResult = {
  ok: boolean;
  message: string;
};

const uniqueAnimalNames = (() => {
  const seen = new Set<string>();
  return animalNames.filter((name) => {
    if (!isValidWorkspaceName(name)) return false;
    if (seen.has(name)) return false;
    seen.add(name);
    return true;
  });
})();

type ExecResult = {
  ok: boolean;
  exitCode: number;
  stdout: string;
  stderr: string;
};

async function runGit(command: string[], cwd: string): Promise<ExecResult> {
  try {
    const proc = Bun.spawn(command, { cwd, stdout: "pipe", stderr: "pipe" });
    const stdoutPromise = proc.stdout ? new Response(proc.stdout).text() : Promise.resolve("");
    const stderrPromise = proc.stderr ? new Response(proc.stderr).text() : Promise.resolve("");
    const [stdout, stderr, exitCode] = await Promise.all([
      stdoutPromise,
      stderrPromise,
      proc.exited,
    ]);

    return { ok: exitCode === 0, exitCode, stdout, stderr };
  } catch (error) {
    return {
      ok: false,
      exitCode: -1,
      stdout: "",
      stderr: error instanceof Error ? error.message : String(error),
    };
  }
}

async function resolveRepoRoot(cwd: string): Promise<string | null> {
  const result = await runGit(["git", "rev-parse", "--show-toplevel"], cwd);
  if (!result.ok) {
    return null;
  }

  return result.stdout.trim();
}

function formatExecOutput(result: ExecResult): string {
  const stdout = result.stdout.trim();
  const stderr = result.stderr.trim();
  return [stdout, stderr].filter(Boolean).join("\n");
}

function extractWorkspaceName(repoRoot: string, worktreePath: string): string | null {
  const normalizedRoot = normalizePath(workspaceRootPath(repoRoot));
  const prefix = `${normalizedRoot}/`;
  const normalizedPath = normalizePath(worktreePath);

  if (!normalizedPath.startsWith(prefix)) {
    return null;
  }

  const remainder = normalizedPath.slice(prefix.length);
  const [name] = remainder.split("/");
  return name || null;
}

async function listWorktreeNames(repoRoot: string): Promise<Set<string>> {
  const names = new Set<string>();
  const result = await runGit(["git", "worktree", "list", "--porcelain"], repoRoot);
  if (!result.ok) {
    return names;
  }

  let lastWorktreePath: string | null = null;
  for (const rawLine of result.stdout.split("\n")) {
    const line = rawLine.trim();
    if (line.length === 0) continue;

    if (line.startsWith("worktree ")) {
      lastWorktreePath = line.slice("worktree ".length);
      const name = extractWorkspaceName(repoRoot, lastWorktreePath);
      if (name) names.add(name);
      continue;
    }

    if (line.startsWith("branch ")) {
      const ref = line.slice("branch ".length).trim();
      if (ref.startsWith("refs/heads/")) {
        names.add(ref.slice("refs/heads/".length));
      }
      continue;
    }

    if (line.startsWith("detached") && lastWorktreePath) {
      const name = extractWorkspaceName(repoRoot, lastWorktreePath);
      if (name) names.add(name);
    }
  }

  return names;
}

async function listUsedWorkspaceNames(repoRoot: string): Promise<Set<string>> {
  return listWorktreeNames(repoRoot);
}

async function branchExists(repoRoot: string, name: string): Promise<boolean> {
  const result = await runGit(
    ["git", "show-ref", "--verify", "--quiet", `refs/heads/${name}`],
    repoRoot,
  );
  return result.ok;
}

function pickUnusedAnimalName(usedNames: Set<string>): string | null {
  for (const name of uniqueAnimalNames) {
    if (!usedNames.has(name)) return name;
  }
  return null;
}

type CommandOptions = {
  cwd?: string;
};

export async function runCommand(
  name: string,
  args: string[],
  options: CommandOptions = {},
): Promise<CommandResult> {
  const cwd = options.cwd ?? import.meta.dir;
  switch (name) {
    case "help":
      return { ok: true, message: commandHelpLines().join("\n") };
    case "create": {
      let workspaceName = args[0];
      const repoRoot = await resolveRepoRoot(cwd);
      if (!repoRoot) {
        return { ok: false, message: "Not inside a git repository." };
      }

      await ensureWorkspaceRoot(repoRoot);
      const usedNames = await listUsedWorkspaceNames(repoRoot);
      if (!workspaceName) {
        workspaceName = pickUnusedAnimalName(usedNames) ?? undefined;
      }
      if (!workspaceName) {
        return {
          ok: false,
          message: "No unused animal names available. Use :create <unique-name>.",
        };
      }
      if (!isValidWorkspaceName(workspaceName)) {
        return {
          ok: false,
          message: "Workspace name must be lowercase letters or dashes.",
        };
      }
      if (usedNames.has(workspaceName)) {
        return {
          ok: false,
          message: `Workspace name '${workspaceName}' is already in use. Choose another.`,
        };
      }
      if (await branchExists(repoRoot, workspaceName)) {
        return {
          ok: false,
          message: `Branch '${workspaceName}' already exists. Choose another workspace name.`,
        };
      }

      const command = buildCreateWorktreeCommand(workspaceName);
      const result = await runGit(command, repoRoot);
      if (!result.ok) {
        const output = formatExecOutput(result);
        const details = output ? `\n${output}` : "";
        return {
          ok: false,
          message: `Failed to create workspace '${workspaceName}'.${details}`,
        };
      }

      const output = formatExecOutput(result);
      const details = output ? `\n${output}` : "";
      return {
        ok: true,
        message: `Created workspace '${workspaceName}' at ${workspacePath(workspaceName)}.${details}`,
      };
    }
    case "destroy": {
      const name = args[0];
      if (!name) {
        return { ok: false, message: "Usage: :destroy <animal>" };
      }
      if (!isValidWorkspaceName(name)) {
        return { ok: false, message: "Workspace name must be lowercase letters or dashes." };
      }
      const repoRoot = await resolveRepoRoot(cwd);
      if (!repoRoot) {
        return { ok: false, message: "Not inside a git repository." };
      }

      const command = buildRemoveWorktreeCommand(name);
      const result = await runGit(command, repoRoot);
      if (!result.ok) {
        const output = formatExecOutput(result);
        const details = output ? `\n${output}` : "";
        return { ok: false, message: `Failed to remove workspace '${name}'.${details}` };
      }

      const output = formatExecOutput(result);
      const details = output ? `\n${output}` : "";
      return {
        ok: true,
        message: `Removed workspace '${name}' from ${workspacePath(name)}.${details}`,
      };
    }
    case "create-pr":
      return { ok: true, message: "PR creation is not implemented yet." };
    case "open-pr":
      return { ok: true, message: "PR opening is not implemented yet." };
    case "merge-pr":
      return { ok: true, message: "PR merge is not implemented yet." };
    default:
      return {
        ok: false,
        message: `Unhandled command: ${name} ${args.join(" ")}`.trim(),
      };
  }
}
