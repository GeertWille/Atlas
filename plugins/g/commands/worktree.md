# Git Worktree Manager

Create, list, or clean up git worktrees. Fully bootstrapped and ready to code.

---

## Route: Determine intent from `$ARGUMENTS`

Analyze `$ARGUMENTS` to determine which mode to run:

1. **Empty** → List mode
2. **Starts with `cleanup`, `remove`, `delete`, or `done`** → Cleanup mode (the rest of the arguments identify which worktree)
3. **Anything else** → Create mode (treat as feature description)

Examples:
- `/g:worktree` → List mode
- `/g:worktree cleanup wallet-dark-mode` → Cleanup mode
- `/g:worktree done with the auth fix` → Cleanup mode
- `/g:worktree remove feat/wallet-dark-mode` → Cleanup mode
- `/g:worktree add dark mode to wallet app` → Create mode

---

## List Mode

If `$ARGUMENTS` is empty, run interactive worktree management:

### List Step 1: Get worktrees

```bash
git worktree list
```

### List Step 2: Present interactive selection

Parse the output of `git worktree list` and present the worktrees to the user using `AskUserQuestion`.

**If only the main worktree exists** (single line of output), show:

```
No linked worktrees found.

Usage:
  /g:worktree <feature description>   Create a new worktree
  /g:worktree cleanup <name>          Remove a worktree

Examples:
  /g:worktree add dark mode to wallet app
  /g:worktree fix the login redirect bug
```

Stop here.

**If linked worktrees exist**, present them as selectable options using `AskUserQuestion`:

- Each option should show the **branch name** and **path** (e.g. `feat/wallet-dark-mode → ../project-feat-wallet-dark-mode`)
- Add a final option: **"Create new worktree"**
- Question: "Which worktree would you like to manage?"

### List Step 3: Handle selection

Based on the user's selection:

- **If they selected an existing worktree**, ask a follow-up question with `AskUserQuestion`:
  - **"Remove this worktree"** — Proceed with Cleanup Mode (starting at Cleanup Step 2, since the worktree is already identified)
  - **"Open in Cursor"** — Run `cursor "<WORKTREE_PATH>"`
  - **"Open tmux session/window"** — Run the tmux logic from Create Mode Step 8, using the worktree path and deriving the session label from the branch name

- **If they selected "Create new worktree"**, tell them to run `/g:worktree <feature description>` with a description of what they want to build.

Stop here — do not proceed with other modes.

---

## Cleanup Mode

The user wants to remove a worktree. The text after the trigger word (`cleanup`/`remove`/`delete`/`done`) identifies which worktree.

### Cleanup Step 1: Find the worktree and main worktree path

List worktrees and match the user's description against them:

```bash
git worktree list
```

From the output, identify:
1. The **main worktree path** (the first line — the bare/main checkout)
2. The **target worktree** to remove (matched from the user's description)

Match the target by:
- Exact branch name (e.g. `feat/wallet-dark-mode`)
- Partial match on directory name (e.g. `wallet-dark-mode`)
- Fuzzy match on the description (e.g. "done with the auth fix" → find worktree with `auth-fix` in its path)

If multiple matches or no match, show the list and ask the user to pick one.

**CRITICAL:** Save the main worktree path as `MAIN_WORKTREE_PATH` — you will need it in Step 4 to avoid deleting your own working directory.

### Cleanup Step 2: Check for uncommitted changes

Before removing, check if the worktree has uncommitted work:

```bash
git -C "<WORKTREE_PATH>" status --porcelain
```

If there are uncommitted changes, **warn the user** and ask for confirmation before proceeding. List the changed files.

### Cleanup Step 3: Check if branch is merged

```bash
git branch --merged main | grep "<BRANCH_NAME>"
```

If the branch is **not merged** into main, warn the user:
- "Branch `<BRANCH_NAME>` has not been merged into main. Delete anyway?"
- Wait for confirmation.

### Cleanup Step 4: Change to main worktree, then remove the target

**CRITICAL:** If Claude Code is currently running inside the worktree being deleted, removing it will destroy the shell's working directory. All subsequent commands will fail with "no such file or directory". To prevent this, **always `cd` to the main worktree first**, then remove.

```bash
cd "<MAIN_WORKTREE_PATH>" && git worktree remove "<WORKTREE_PATH>" --force
echo "  Worktree removed: <WORKTREE_PATH>"
```

All subsequent commands in this cleanup MUST also use `cd "<MAIN_WORKTREE_PATH>" &&` prefix or run with `git -C "<MAIN_WORKTREE_PATH>"` to ensure they execute from a valid directory.

### Cleanup Step 5: Delete the branch

If the branch was merged (or user confirmed deletion):

```bash
cd "<MAIN_WORKTREE_PATH>" && git branch -d "<BRANCH_NAME>" 2>/dev/null || cd "<MAIN_WORKTREE_PATH>" && git branch -D "<BRANCH_NAME>"
echo "  Branch deleted: <BRANCH_NAME>"
```

### Cleanup Step 6: Prune

```bash
cd "<MAIN_WORKTREE_PATH>" && git worktree prune
```

### Cleanup Step 7: Kill tmux window/session (LAST)

Derive the session label the same way as creation (the descriptive part without the prefix).
This MUST be the final step — if Claude Code is running inside this tmux window, killing it earlier would terminate the cleanup process before the worktree and branch are removed.

```bash
if command -v tmux &> /dev/null; then
    # Try killing a window with that name in the current session
    tmux kill-window -t "<SESSION_LABEL>" 2>/dev/null && echo "  tmux window '<SESSION_LABEL>' closed"
    # Try killing a session with that name
    tmux kill-session -t "<SESSION_LABEL>" 2>/dev/null && echo "  tmux session '<SESSION_LABEL>' closed"
fi
```

### Cleanup Summary

```
Worktree cleaned up!
  Branch:    <BRANCH_NAME> — deleted / kept (unmerged)
  Directory: <WORKTREE_PATH> — removed
  tmux:      ✓ window/session closed / ✗ not found

Remaining worktrees:
```

Then run `cd "<MAIN_WORKTREE_PATH>" && git worktree list`.

---

## Create Mode

### Step 1: Derive branch name

From the feature description `$ARGUMENTS`, generate a concise kebab-case branch name following these rules:
- Use conventional prefix: `feat/`, `fix/`, `chore/`, `refactor/`, or `docs/` based on the description
- Keep it short (2-4 words after the prefix)
- Use only lowercase letters, numbers, and hyphens
- Examples:
  - "add dark mode to wallet app" → `feat/wallet-dark-mode`
  - "fix the login redirect bug" → `fix/login-redirect`
  - "refactor job query service" → `refactor/job-query-service`
  - "update deployment docs" → `docs/deployment`

Also derive a short **session label** from the branch name for tmux (e.g. `wallet-dark-mode`, without the prefix — just the descriptive part).

Tell the user the branch name you chose before proceeding.

### Step 2: Validate

```bash
if ! git rev-parse --git-dir > /dev/null 2>&1; then
    echo "Error: Not a git repository"
    exit 1
fi
```

### Step 3: Get current branch and set worktree path

Use the derived branch name as `BRANCH_NAME` in the commands below.
For the directory name:
1. Get the current folder name (e.g. `my-project`)
2. Replace `/` with `-` in the branch name (e.g. `feat/wallet-dark-mode` → `feat-wallet-dark-mode`)
3. Combine as `<PROJECT_NAME>-<BRANCH_DIR_NAME>` (e.g. `my-project-feat-wallet-dark-mode`)

```bash
CURRENT_BRANCH=$(git branch --show-current)
PROJECT_NAME=$(basename "$(pwd)")
PARENT_DIR=$(dirname "$(pwd)")
WORKTREE_PATH="$PARENT_DIR/$PROJECT_NAME-<BRANCH_DIR_NAME>"

echo "Current branch: $CURRENT_BRANCH"
echo "Project: $PROJECT_NAME"
echo "Worktree path: $WORKTREE_PATH"
```

### Step 3b: Determine base branch

Check if the current branch is `main` or `master`. If not, ask the user which branch to base the new worktree on.

```bash
# Determine the default branch (main or master)
DEFAULT_BRANCH=$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@')
if [ -z "$DEFAULT_BRANCH" ]; then
    # Fallback: check if main or master exists
    if git show-ref --verify --quiet refs/heads/main; then
        DEFAULT_BRANCH="main"
    elif git show-ref --verify --quiet refs/heads/master; then
        DEFAULT_BRANCH="master"
    fi
fi
echo "Default branch: $DEFAULT_BRANCH"
```

**If current branch equals the default branch** (e.g., on `main`), use it as the base and proceed to Step 4.

**If current branch is NOT the default branch**, use `AskUserQuestion` to ask:

- Question: "Which branch should the new worktree be based on?"
- Header: "Base branch"
- Options:
  1. **`<DEFAULT_BRANCH>` (Recommended)** — "Start fresh from the main development branch"
  2. **`<CURRENT_BRANCH>`** — "Branch from your current work (includes uncommitted changes won't be included)"

Store the selected branch as `BASE_BRANCH` for use in Step 4.

### Step 4: Create worktree

Create the worktree from the selected base branch (determined in Step 3b):

```bash
git worktree add "$WORKTREE_PATH" -b "<BRANCH_NAME>" "<BASE_BRANCH>"
```

- If `BASE_BRANCH` is the current branch, you can omit it: `git worktree add "$WORKTREE_PATH" -b "<BRANCH_NAME>"`
- If the branch already exists, use `git worktree add "$WORKTREE_PATH" "<BRANCH_NAME>"` instead (without `-b`)
- If the worktree path already exists, inform the user and suggest they remove it first or choose a different name

### Step 5: Copy .certs if present

```bash
if [ -d ".certs" ]; then
    echo "Copying .certs folder to worktree..."
    cp -r .certs "$WORKTREE_PATH/.certs"
    echo "  .certs copied successfully"
else
    echo "No .certs folder found, skipping copy"
fi
```

### Step 6: Copy .env.local files

Copy all `.env.local` files from every app so secrets don't need regenerating:

```bash
echo "Copying .env.local files..."
for env_file in $(find apps -maxdepth 2 -name ".env.local" 2>/dev/null); do
    target="$WORKTREE_PATH/$env_file"
    mkdir -p "$(dirname "$target")"
    cp "$env_file" "$target"
    echo "  Copied $env_file"
done
```

If no `.env.local` files were found, try to generate them:

```bash
if [ -f "$WORKTREE_PATH/scripts/generate-env-local.sh" ]; then
    echo "No .env.local files found. Running generate-env-local.sh..."
    cd "$WORKTREE_PATH" && bash scripts/generate-env-local.sh
fi
```

### Step 7: Install dependencies

```bash
echo "Installing dependencies in worktree..."
cd "$WORKTREE_PATH" && pnpm install
```

### Step 8: Open tmux (if available)

If tmux is installed, use the smart approach:
- **Already inside tmux** (`$TMUX` is set): create a new **window** (tab) in the current session, named with the session label
- **Not inside tmux**: create a new **session** named with the session label

After creating the window/session, set up a **70/30 vertical split**:
- **Top pane (70%)**: `claude --dangerously-skip-permissions` (Claude Code in auto-accept mode)
- **Bottom pane (30%)**: plain shell (for running commands, dev server, etc.)

```bash
if command -v tmux &> /dev/null; then
    if [ -n "$TMUX" ]; then
        echo "Adding tmux window '<SESSION_LABEL>'..."
        tmux new-window -n "<SESSION_LABEL>" -c "$WORKTREE_PATH"

        # Split 70/30 vertical — bottom pane is a plain shell
        tmux split-window -v -p 30 -t "<SESSION_LABEL>" -c "$WORKTREE_PATH"

        # Select the top pane and launch Claude Code
        tmux select-pane -t "<SESSION_LABEL>".0
        tmux send-keys -t "<SESSION_LABEL>".0 "claude --dangerously-skip-permissions" Enter

        echo "  Window '<SESSION_LABEL>' added to current session"
        echo "  Top pane:    claude --dangerously-skip-permissions"
        echo "  Bottom pane: shell"
        echo "  Switch with: Ctrl+B n/p or Ctrl+B w"
    else
        echo "Starting tmux session '<SESSION_LABEL>'..."
        tmux new-session -d -s "<SESSION_LABEL>" -c "$WORKTREE_PATH"

        # Split 70/30 vertical — bottom pane is a plain shell
        tmux split-window -v -p 30 -t "<SESSION_LABEL>" -c "$WORKTREE_PATH"

        # Select the top pane and launch Claude Code
        tmux select-pane -t "<SESSION_LABEL>":0.0
        tmux send-keys -t "<SESSION_LABEL>":0.0 "claude --dangerously-skip-permissions" Enter

        echo "  tmux session '<SESSION_LABEL>' created"
        echo "  Top pane:    claude --dangerously-skip-permissions"
        echo "  Bottom pane: shell"
        echo "  Attach with: tmux attach -t <SESSION_LABEL>"
    fi
else
    echo "tmux not found, skipping"
fi
```

### Step 9: Open in Cursor

```bash
if command -v cursor &> /dev/null; then
    echo "Opening worktree in Cursor..."
    cursor "$WORKTREE_PATH"
else
    echo "Cursor not found, skipping IDE launch"
fi
```

### Step 10: Summary

Print a final summary:

```
Worktree ready!
  Branch:  <BRANCH_NAME> (based on <BASE_BRANCH>)
  Path:    <WORKTREE_PATH>
  Certs:   ✓ copied / ✗ not found
  Secrets: ✓ .env.local copied / ✓ generated / ✗ none
  Deps:    ✓ pnpm install complete
  tmux:    ✓ window '<SESSION_LABEL>' added / ✓ session '<SESSION_LABEL>' created / ✗ not available
  Layout:  ✓ 70/30 split — claude (top) + shell (bottom) / ✗ no tmux
  Cursor:  ✓ opened / ✗ not available

  In tmux:     Ctrl+B w to switch windows
  Outside:     tmux attach -t <SESSION_LABEL>
```

Also run `git worktree list` to show the full picture.
