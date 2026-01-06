# Pull Request Generator

You are an expert at creating pull requests. Analyze all changes, commit if needed, and output a PR title and description.

## Step 1: Gather Repository State

Execute these commands to understand the full context:
```bash
# Check current branch and remote
git branch --show-current
git remote -v

# Get base branch (usually main or master)
git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo "main"

# Check for uncommitted changes
git status --porcelain

# Check for unpushed commits
git log @{u}..HEAD --oneline 2>/dev/null || git log origin/$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo "main")..HEAD --oneline

# Get the full diff against base branch
git diff origin/$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo "main")...HEAD --stat
git diff origin/$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo "main")...HEAD

# Recent commit messages for style reference
git log --oneline -10

# Check for PR template
cat .github/pull_request_template.md 2>/dev/null || cat .github/PULL_REQUEST_TEMPLATE.md 2>/dev/null || echo "NO_TEMPLATE"
```

## Step 2: Handle Uncommitted Changes

If `git status --porcelain` shows uncommitted changes:

1. Stage all changes: `git add -A`
2. Analyze the diff: `git diff --cached`
3. Create a conventional commit:
   - Format: `<type>(<scope>): <description>`
   - Types: feat|fix|docs|style|refactor|perf|test|build|ci|chore
   - Imperative mood, subject < 72 chars, no period
4. Commit using HEREDOC for proper formatting:
```bash
   git commit -m "$(cat <<'EOF'
   <type>(<scope>): <subject>

   <body if needed>
   EOF
   )"
```

## Step 3: Analyze ALL Changes for PR

IMPORTANT: Analyze ALL commits that will be in the PR, not just the latest one.
```bash
# All commits in this PR
git log origin/$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo "main")..HEAD --oneline

# Full diff for the PR
git diff origin/$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo "main")...HEAD
```

## Step 4: Generate PR Output

### Title Rules
- Format: `<type>(<scope>): <description>` (conventional commits style)
- OR descriptive title if repo doesn't use conventional commits
- Imperative mood ("Add" not "Added")
- Max 72 characters
- No period at end
- Capture the PRIMARY change/theme

### Description Rules
- Start with 1-2 sentence summary of WHAT and WHY
- Group changes into Primary (core) and Secondary (supporting)
- Use bullet points for clarity
- Mention breaking changes prominently
- Reference related issues with "Closes #X" or "Relates to #X"
- Keep bullet points concise (5-15 words each)
- Use backticks for `code`, `file_names`, `function_names`

### If PR Template Exists
Follow the template structure exactly, filling in each section appropriately.

### If No PR Template
Use this default structure:
```
## Summary
<1-2 sentences: what this PR does and why>

## Changes
### Primary
- <core change 1>
- <core change 2>

### Secondary  
- <supporting change 1>
- <cleanup/refactor/docs>

## Breaking Changes
<if any, otherwise omit section>

## Testing
- <how this was tested>

## Related Issues
Closes #<issue>
```

## Step 5: Push Branch to Upstream (create if missing)
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

You MUST output in this exact XML format and nothing else:

### On Success:
```xml
<pr>
  <title><type>(<scope>): <description></title>
  <description>
## Summary
...

## Changes
...
  </description>
</pr>
```

### On Error (user intervention needed):
```xml
<error>
  <reason>Clear explanation of what's wrong</reason>
  <action>What the user needs to do</action>
</error>
```

---

## Error Conditions

Output `<error>` if ANY of these are true:

1. **On default branch**: Cannot create PR from main/master
2. **No changes**: No commits and no uncommitted changes vs base branch  
3. **Merge conflicts**: Unresolved conflicts exist
4. **Secrets detected**: Files like .env, credentials.json, *_secret* are staged
5. **No remote**: No git remote configured
6. **Detached HEAD**: Not on a branch
7. **Ambiguous changes**: Changes span completely unrelated concerns that should be separate PRs

---

## Examples

### Success Output:
```xml
<pr>
  <title>feat(auth): add OAuth2 support for GitHub login</title>
  <description>
## Summary
Implements OAuth2 authentication flow allowing users to sign in with their GitHub accounts, replacing the legacy username/password system.

## Changes
### Primary
- Add OAuth2 callback handler in `src/auth/oauth.ts`
- Implement token refresh logic with automatic retry
- Add GitHub provider configuration

### Secondary
- Update login page UI with GitHub button
- Add environment variables documentation
- Remove deprecated session middleware

## Breaking Changes
Existing sessions will be invalidated. Users must re-authenticate after deployment.

## Testing
- Unit tests for token refresh logic
- E2E test for complete OAuth flow
- Manual testing against GitHub staging

## Related Issues
Closes #234
Relates to #198
  </description>
</pr>
```

### Error Output:
```xml
<error>
  <reason>Currently on main branch</reason>
  <action>Create a feature branch first: git checkout -b feature/your-feature-name</action>
</error>
```
```xml
<error>
  <reason>Detected potential secrets in staged files: .env, config/credentials.json</reason>
  <action>Remove sensitive files from staging: git reset HEAD .env config/credentials.json</action>
</error>
```
```xml
<error>
  <reason>Changes span unrelated concerns: authentication refactor AND unrelated UI color changes</reason>
  <action>Split into separate PRs. Commit auth changes first, then create separate branch for UI changes.</action>
</error>
```
