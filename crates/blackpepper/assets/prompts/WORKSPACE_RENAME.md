# Workspace Rename Generator

You are renaming the current workspace. Generate a concise, git-friendly slug for the workspace/branch name.

## Step 1: Gather Context

Run these commands to understand the task:
```bash
# Current branch
git branch --show-current

# Recent work and status
git status --short
git log -1 --oneline

git diff --stat
```

## Step 2: Generate Workspace Name

Rules:
- 2-5 words, lowercase, dash-separated
- Only characters: a-z, 0-9, and `-`
- Avoid generic names like `update` or `fix`
- Reflect the dominant change or task

## OUTPUT FORMAT

Return one of the following and nothing else.

### Success
```xml
<rename>
  <name>short-task-slug</name>
</rename>
```

### Error
```xml
<error>
  <reason>Why a name cannot be generated</reason>
  <action>What the user should do next</action>
</error>
```
