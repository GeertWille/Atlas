---
name: setup
prefix: g
description: Install tmux-worktree-switcher to ~/.local/bin and configure tmux keybinding
---

# /g:setup — Install tmux-worktree-switcher

You are setting up the tmux-worktree-switcher tool from the Atlas g plugin.

## Steps

1. **Locate the script** in the plugin cache:
   - Look in `~/.claude/plugins/cache/` for the Atlas plugin
   - The script is at `plugins/g/bin/tmux-worktree-switcher` within the plugin directory
   - Use `find ~/.claude/plugins/cache -name "tmux-worktree-switcher" 2>/dev/null` to locate it

2. **Create the target directory** if it doesn't exist:
   ```bash
   mkdir -p ~/.local/bin
   ```

3. **Copy the script** to `~/.local/bin/tmux-worktree-switcher`

4. **Make it executable**:
   ```bash
   chmod +x ~/.local/bin/tmux-worktree-switcher
   ```

5. **Check if ~/.local/bin is in PATH**:
   - If not, suggest adding `export PATH="$HOME/.local/bin:$PATH"` to their shell config

6. **Print the tmux configuration**:
   Tell the user to add this line to `~/.tmux.conf`:
   ```
   bind g display-popup -E -w 80% -h 60% "tmux-worktree-switcher"
   ```
   Then reload tmux config with `tmux source-file ~/.tmux.conf` or restart tmux.

## Output

Print a summary showing:
- Where the script was installed
- The tmux keybinding to add
- How to reload the tmux config
- Note that `Ctrl+B g` will open the worktree switcher popup

## Requirements

The switcher requires:
- tmux (for popup windows)
- fzf (for fuzzy selection)
- git (for worktree operations)

If any are missing, note which ones need to be installed.

## Fallback

If the script cannot be found in the plugin cache:
1. Ask the user for the path to their Atlas repository clone
2. Copy from `<atlas-path>/plugins/g/bin/tmux-worktree-switcher`
