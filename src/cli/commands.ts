export type CommandSpec = {
  name: string;
  description: string;
};

export const COMMANDS: CommandSpec[] = [
  {
    name: "create",
    description: "Create a workspace worktree (auto-pick name if omitted)",
  },
  { name: "destroy", description: "Destroy a workspace worktree (name required)" },
  { name: "create-pr", description: "Create a pull request" },
  { name: "open-pr", description: "Open the current pull request" },
  { name: "merge-pr", description: "Merge the current pull request" },
  { name: "help", description: "Show available commands" },
];
