# Atlas — Developer Toolkit for Claude Code

A productivity toolbox for shipping products faster.

Copyright (c) 2026 Geert Wille

- LinkedIn: https://www.linkedin.com/in/geertwille/
- GitHub: https://github.com/GeertWille/

## Plugins

| Plugin | Prefix | Description |
|--------|--------|-------------|
| **g** | `/g:` | Git productivity tools |

## Commands

| Command | Description |
|---------|-------------|
| `/g:worktree` | Create, list, or clean up git worktrees with full project bootstrapping |
| `/g:worktree <description>` | Create a new worktree from a feature description |
| `/g:worktree cleanup <name>` | Remove a worktree safely |
| `/g:setup` | Install tmux-worktree-switcher and configure tmux keybinding |

## tmux Worktree Switcher

The g plugin includes `tmux-worktree-switcher`, a fast fzf-based worktree navigator for tmux. It shows all worktrees in a popup and lets you quickly switch between them.

Features:
- Shows active (●) and inactive (○) worktrees
- Displays the current command/task running in each worktree
- Creates a new tmux window when switching to an inactive worktree
- Focuses the correct pane when switching to an active worktree

### Quick Setup

Run `/g:setup` in Claude Code to automatically install the switcher.

### Manual Setup

1. Copy the script to your PATH:
   ```bash
   cp /path/to/atlas/plugins/g/bin/tmux-worktree-switcher ~/.local/bin/
   chmod +x ~/.local/bin/tmux-worktree-switcher
   ```

2. Add the keybinding to `~/.tmux.conf`:
   ```
   bind g display-popup -E -w 80% -h 60% "tmux-worktree-switcher"
   ```

3. Reload tmux config:
   ```bash
   tmux source-file ~/.tmux.conf
   ```

4. Press `Ctrl+B g` to open the worktree switcher popup.

### Requirements

- tmux
- fzf
- git

## Installation

### From GitHub

1. In Claude Code, add the marketplace:

```
/plugin marketplace add GeertWille/atlas
```

2. Install the plugin:

```
/plugin install g@atlas
```

### From a local clone

If you prefer working from a local copy:

1. Clone the repository:

```bash
git clone https://github.com/GeertWille/atlas.git
```

2. In Claude Code, add the marketplace:

```
/plugin marketplace add /path/to/atlas
```

3. Install the plugin:

```
/plugin install g@atlas
```

### Choosing scope

By default plugins install to user scope (available in all projects). To install for a specific project only:

```
/plugin install g@atlas --scope project
```

### Managing plugins

You can also use the interactive plugin manager:

```
/plugin
```

This opens a UI where you can browse the **Discover** tab to find and install Atlas plugins.

## License

MIT
