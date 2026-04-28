# Installing understandable for Claude Code

> **Heads-up:** the markdown skills/agents in this plugin shell out to
> the Rust `understandable` binary. Install it once before completing
> the steps below:
>
> ```bash
> cargo install --git https://github.com/yaroher/understandable understandable \
>   --features all-langs,local-embeddings
> ```

## Prerequisites

- [Claude Code](https://claude.ai/code) (latest)
- Git

## Option A — Plugin marketplace (recommended)

```
/plugin marketplace add yaroher/understandable
/plugin install understandable
```

Skills and agents are discovered automatically. Restart Claude Code if
skills don't appear immediately.

## Option B — Manual install

1. **Clone the repository:**
   ```bash
   git clone https://github.com/yaroher/understandable.git ~/.claude-plugin/understandable
   ```

2. **Register the plugin:**
   ```bash
   claude plugin install ~/.claude-plugin/understandable
   ```

   Or tell Claude Code:
   ```
   Fetch and follow instructions from https://raw.githubusercontent.com/yaroher/understandable/main/.claude-plugin/INSTALL.md
   ```

3. **Restart Claude Code** to discover the plugin.

## Verify

Type `/` in Claude Code — you should see these skills:

- `/understand` — build the knowledge graph
- `/understand-chat` — ask questions about the codebase
- `/understand-dashboard` — open the interactive dashboard
- `/understand-diff` — analyze impact of current changes
- `/understand-domain` — build the domain/flow graph
- `/understand-explain` — deep-dive into a file or function
- `/understand-knowledge` — ingest a wiki into the knowledge graph
- `/understand-onboard` — generate an onboarding guide
- `/understand-setup` — guided setup wizard (interactive)

## Usage

Skills activate automatically when relevant. You can also invoke them
directly by typing `/understand` in Claude Code Chat.

## Updating

```bash
cd ~/.claude-plugin/understandable && git pull
```

Or via the marketplace:
```
/plugin update understandable
```

## Uninstalling

```bash
claude plugin uninstall understandable
rm -rf ~/.claude-plugin/understandable
```

## Issues

File issues at https://github.com/yaroher/understandable/issues
