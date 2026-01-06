---
name: verbump
description: Bump the Blackpepper version, review full git diffs (including unrelated changes), craft a Conventional Commit message, and push the current branch. Use when the user asks to "verbump", "bump version", "increment version", "release", or explicitly requests version bump + commit + push for this repo.
---

# Verbump

## Overview

Bump the version in `crates/blackpepper/Cargo.toml`, verify the full diff (including changes the agent did not make), commit with a clean message, and push the current branch.
For this project, commits are the changelog.

## Workflow

1. Confirm target version
   - If the user specifies a version, use it.
   - If the user says "verbump" or "bump", default to a patch bump and state the new version before applying.
   - Ask for clarification if the request is ambiguous (major/minor vs patch).

2. Review repository state and diffs
   - Run `git status -sb`.
   - Review complete diffs with `git diff --stat` and `git diff`.
   - If unrelated changes exist, call them out and confirm whether they should be included. Do not revert changes unless explicitly requested.

3. Bump the version
   - Update only `crates/blackpepper/Cargo.toml`.
   - Keep edits ASCII-only and preserve file formatting.
   - Re-check with `rg -n "version" crates/blackpepper/Cargo.toml` if needed.

4. Validate and re-check diffs
   - Re-run `git status -sb` and `git diff --stat`.
   - Ensure only expected files changed before committing.
   - If tests are run, report results; if skipped, state why.

5. Commit and push
   - Stage the relevant files (usually `crates/blackpepper/Cargo.toml`, plus anything already expected).
   - Craft a Changelog style Commit message.
   - `git commit -m "<message>"`.
   - `git push` the current branch.

## Output expectations

- Always report the chosen version and the final commit message.
- Summarize any unrelated diffs you saw and whether they were included.
