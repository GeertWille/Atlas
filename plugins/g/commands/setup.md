# tmux Worktree Switcher Setup

Install the tmux-worktree-switcher tool for fast fzf-based worktree navigation.

---

## Step 1: Check dependencies

Verify required tools are installed:

```bash
echo "Checking dependencies..."
command -v tmux &>/dev/null && echo "  ✓ tmux" || echo "  ✗ tmux (required)"
command -v fzf &>/dev/null && echo "  ✓ fzf" || echo "  ✗ fzf (required)"
command -v git &>/dev/null && echo "  ✓ git" || echo "  ✗ git (required)"
```

If any are missing, inform the user they need to install them first (e.g., `brew install tmux fzf`).

---

## Step 2: Locate the script

Find the `tmux-worktree-switcher` script in the plugin cache:

```bash
SCRIPT_PATH=$(find ~/.claude/plugins/cache -name "tmux-worktree-switcher" 2>/dev/null | head -1)
echo "Found script at: $SCRIPT_PATH"
```

**If not found**, ask the user for the path to their Atlas repository clone and use `<atlas-path>/plugins/g/bin/tmux-worktree-switcher`.

---

## Step 3: Install the script

```bash
mkdir -p ~/.local/bin
cp "$SCRIPT_PATH" ~/.local/bin/tmux-worktree-switcher
chmod +x ~/.local/bin/tmux-worktree-switcher
echo "Installed to ~/.local/bin/tmux-worktree-switcher"
```

---

## Step 4: Check PATH

```bash
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    echo "Warning: ~/.local/bin is not in your PATH"
    echo "Add this to your shell config (~/.zshrc or ~/.bashrc):"
    echo '  export PATH="$HOME/.local/bin:$PATH"'
fi
```

---

## Step 5: Configure tmux keybinding

Check if the keybinding already exists in `~/.tmux.conf`:

```bash
if grep -q "tmux-worktree-switcher" ~/.tmux.conf 2>/dev/null; then
    echo "Keybinding already configured in ~/.tmux.conf"
else
    echo "Add this line to ~/.tmux.conf:"
    echo '  bind g display-popup -E -w 80% -h 60% "tmux-worktree-switcher"'
fi
```

Use `AskUserQuestion` to ask:
- **"Add keybinding automatically"** — Append the bind line to `~/.tmux.conf`
- **"I'll add it manually"** — Just show the instructions

If adding automatically:

```bash
echo '' >> ~/.tmux.conf
echo '# Worktree switcher (Ctrl+B g)' >> ~/.tmux.conf
echo 'bind g display-popup -E -w 80% -h 60% "tmux-worktree-switcher"' >> ~/.tmux.conf
echo "Added keybinding to ~/.tmux.conf"
```

---

## Step 6: Reload tmux config

If tmux is running:

```bash
if [ -n "$TMUX" ]; then
    tmux source-file ~/.tmux.conf
    echo "tmux config reloaded"
else
    echo "Reload tmux config with: tmux source-file ~/.tmux.conf"
fi
```

---

## Summary

Print a final summary:

```
tmux-worktree-switcher installed!

  Script:     ~/.local/bin/tmux-worktree-switcher
  Keybinding: Ctrl+B g (opens worktree popup)

Usage:
  1. Press Ctrl+B g in any tmux session
  2. Use arrow keys or type to filter worktrees
  3. Press Enter to switch to the selected worktree

Features:
  • Shows active (●) and inactive (○) worktrees
  • Displays current command running in each worktree
  • Creates new tmux window when switching to inactive worktree
```
