# Installing understandable for Cursor

> **Heads-up:** the markdown skills/agents in this plugin shell out to
> the Rust `understandable` binary. Install it once before completing
> the steps below:
>
> ```bash
> cargo install --git https://github.com/yaroher/understandable understandable \
>   --features all-langs,local-embeddings
> ```

## Prerequisites

- [Cursor](https://www.cursor.com/) (latest)
- Git

## Option A — Auto-discovery (recommended)

Clone this repo into your workspace. Cursor automatically discovers the
plugin via `.cursor-plugin/plugin.json` — no manual steps required.

```bash
git clone https://github.com/yaroher/understandable.git
# Open the folder in Cursor; skills appear under /
```

## Option B — Manual install (available across all projects)

1. **Clone the repository:**
   ```bash
   git clone https://github.com/yaroher/understandable.git ~/.cursor-plugin/understandable
   ```

2. **Create skill symlinks:**
   ```bash
   mkdir -p ~/.cursor/skills
   for skill in understand understand-chat understand-dashboard understand-diff \
       understand-domain understand-explain understand-knowledge understand-onboard understand-setup; do
     ln -sf ~/.cursor-plugin/understandable/skills/$skill ~/.cursor/skills/$skill
   done
   # Universal plugin root symlink — lets the dashboard skill find its assets
   [ -e ~/.understandable ] || [ -L ~/.understandable ] || \
     ln -s ~/.cursor-plugin/understandable/ ~/.understandable
   ```

   **Windows (PowerShell):**
   ```powershell
   New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.cursor\skills"
   $skills = @("understand","understand-chat","understand-dashboard","understand-diff",
               "understand-domain","understand-explain","understand-knowledge",
               "understand-onboard","understand-setup")
   foreach ($skill in $skills) {
     cmd /c mklink /J "$env:USERPROFILE\.cursor\skills\$skill" `
       "$env:USERPROFILE\.cursor-plugin\understandable\skills\$skill"
   }
   cmd /c mklink /J "$env:USERPROFILE\.understandable" `
     "$env:USERPROFILE\.cursor-plugin\understandable"
   ```

3. **Reload Cursor** (`Cmd+Shift+P` → `Reload Window`).

## Verify

Type `/` in Cursor's AI panel — you should see these skills:

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
directly by typing `/understand` in Cursor's AI panel.

## Updating

```bash
cd ~/.cursor-plugin/understandable && git pull
```

Skills update instantly through the symlinks.

## Uninstalling

```bash
for skill in understand understand-chat understand-dashboard understand-diff \
    understand-domain understand-explain understand-knowledge understand-onboard understand-setup; do
  rm -f ~/.cursor/skills/$skill
done
rm -f ~/.understandable
rm -rf ~/.cursor-plugin/understandable
```

## Issues

File issues at https://github.com/yaroher/understandable/issues
