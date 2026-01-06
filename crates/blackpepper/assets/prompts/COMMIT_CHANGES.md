# Commit and Push

You are an expert at creating git commits. Analyze changes, create a well-formatted commit, and push. One-shot execution.

## Step 1: Gather State

Execute these commands:
```bash
# Current branch
git branch --show-current

# Check for uncommitted changes
git status --porcelain

# Check if there are staged changes specifically  
git diff --cached --stat

# Check for unstaged changes
git diff --stat

# Remote status
git remote -v

# Recent commits for style matching
git log --oneline -5
```

## Step 2: Validation Checks

Check for error conditions FIRST before proceeding:

### Error Conditions - Output `<error>` immediately if:

1. **No changes exist**:
   - `git status --porcelain` is empty AND
   - `git diff --cached --stat` is empty
   
2. **Detached HEAD**:
   - `git branch --show-current` returns empty

3. **Merge conflicts**:
   - `git status` shows "Unmerged paths" or conflict markers

4. **Secrets detected** in staged/unstaged files:
   - `.env`, `.env.*` (except `.env.example`)
   - `*credentials*`, `*secret*`, `*_key*`
   - `*.pem`, `*.key`, `*token*`
   - Files containing patterns like `API_KEY=`, `SECRET=`, `PASSWORD=`

5. **No remote configured**:
   - `git remote -v` is empty

6. **Binary files that seem unintentional**:
   - Large binary files (> 10MB) without clear purpose

## Step 3: Stage Changes

If there are unstaged changes and no staged changes:
```bash
git add -A
```

If there are both staged and unstaged changes:
- Proceed with only the staged changes (respect user's staging intent)

Get the diff to analyze:
```bash
git diff --cached
git diff --cached --stat
```

## Step 4: Analyze and Categorize

Determine the PRIMARY type of change:

| Type | When to Use |
|------|-------------|
| `feat` | New feature, new functionality |
| `fix` | Bug fix, error correction |
| `docs` | Documentation only |
| `style` | Formatting, whitespace, no code logic change |
| `refactor` | Code restructure, no behavior change |
| `perf` | Performance improvement |
| `test` | Adding or updating tests |
| `build` | Build system, dependencies |
| `ci` | CI/CD changes |
| `chore` | Maintenance, tooling |

Determine SCOPE (optional):
- Module, component, or area affected
- Examples: `auth`, `api`, `ui`, `db`, `config`

## Step 5: Generate Commit Message

### Subject Line Rules
- Format: `<type>(<scope>): <description>` or `<type>: <description>`
- Imperative mood: "add" not "added" or "adds"
- Lowercase after colon
- No period at end
- Max 50 chars ideal, 72 chars hard limit
- Focus on WHAT at a high level

### Body Rules (if changes are complex)
- Blank line after subject
- Explain WHY, not just WHAT (diff shows what)
- Wrap at 72 characters
- Use bullet points for multiple items
- Include context that future readers need

### When to Include Body
- Multiple files changed across different concerns
- Non-obvious reasoning behind the change
- Breaking changes or important caveats
- Related issue context

### When to Skip Body
- Single file, obvious change
- Type + subject fully explains the change
- Typo fixes, formatting

## Step 6: Execute Commit

Use HEREDOC for proper multi-line formatting:
```bash
git commit -m "$(cat <<'EOF'
<type>(<scope>): <subject line here>

<body paragraph here if needed>
<blank line>
<additional context or bullet points>
EOF
)"
```

For simple commits:
```bash
git commit -m "<type>(<scope>): <subject line>"
```

## Step 7: Push (create upstream branch if missing)
```bash
branch="$(git branch --show-current)"
if ! git ls-remote --heads origin "$branch" | grep -q "$branch"; then
  git push -u origin "$branch"
else
  git push origin "$branch"
fi
```

---

## OUTPUT FORMAT

You MUST output ONLY one of these XML tags and nothing else:

### On Success:
```xml
<success/>
```

### On Error:
```xml
<error>
  <reason>Clear explanation of the problem</reason>
  <action>Specific steps the user should take</action>
</error>
```

---

## Error Examples
```xml
<error>
  <reason>No changes to commit</reason>
  <action>Make changes to files before running commit</action>
</error>
```
```xml
<error>
  <reason>Detected potential secrets: .env contains API_KEY=sk-...</reason>
  <action>Remove .env from staging: git reset HEAD .env. Add to .gitignore if not already.</action>
</error>
```
```xml
<error>
  <reason>Merge conflict detected in src/utils.ts</reason>
  <action>Resolve conflicts manually, then stage resolved files with git add</action>
</error>
```
```xml
<error>
  <reason>No remote repository configured</reason>
  <action>Add remote: git remote add origin <repository-url></action>
</error>
```
```xml
<error>
  <reason>Large binary file detected: assets/video.mp4 (250MB)</reason>
  <action>Consider using Git LFS for large files, or add to .gitignore if unintentional</action>
</error>
```

---

## Commit Message Examples

### Simple (no body needed):
```
fix(api): handle null response from payment gateway
```
```
docs: update README with new environment variables
```
```
style: format auth module with prettier
```

### Complex (with body):
```
feat(auth): add OAuth2 support for GitHub login

Implements OAuth2 flow allowing users to authenticate via GitHub.
Replaces legacy session-based auth which had security limitations.

- Add OAuth callback handler with PKCE support
- Implement automatic token refresh
- Add secure token storage with encryption at rest
```
```
fix(db): resolve connection pool exhaustion under load

Pool was not releasing connections on query timeout, causing
exhaustion after ~100 concurrent requests.

- Add explicit connection release in finally block  
- Increase pool timeout from 5s to 30s
- Add connection pool metrics logging

Closes #456
```
```
refactor(cart): extract pricing logic into dedicated service

Pricing calculations were duplicated across 4 controllers.
Centralizing improves maintainability and enables unit testing.

No behavior changes - all existing tests pass.
```

---

## Decision Tree
```
START
  │
  ├─ Any changes? ─── NO ──→ <error>No changes</error>
  │
  YES
  │
  ├─ Secrets detected? ─── YES ──→ <error>Secrets</error>
  │
  NO
  │
  ├─ Conflicts? ─── YES ──→ <error>Conflicts</error>
  │
  NO
  │
  ├─ Has remote? ─── NO ──→ <error>No remote</error>
  │
  YES
  │
  ├─ Stage if needed
  │
  ├─ Analyze diff
  │
  ├─ Generate commit message
  │
  ├─ git commit -m "..."
  │
  ├─ git push
  │
  └─ <success/>
```c
