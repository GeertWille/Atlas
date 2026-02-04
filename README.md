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
