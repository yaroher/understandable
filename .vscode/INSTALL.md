# Installing understandable for VS Code + GitHub Copilot

> **Heads-up:** the markdown skills/agents in this plugin shell out to
> the Rust `understandable` binary. Install it once before completing
> the steps below:
>
> ```bash
> cargo install --git https://github.com/yaroher/understandable understandable
> ```

## Prerequisites

- [VS Code](https://code.visualstudio.com/) with the [GitHub Copilot](https://marketplace.visualstudio.com/items?itemName=GitHub.copilot) extension (v1.108+)
- Git

## Option A — Auto-discovery (recommended)

Clone this repo and open it in VS Code. GitHub Copilot automatically discovers the plugin via `.copilot-plugin/plugin.json` — no manual steps required.

```bash
git clone https://github.com/yaroher/understandable.git
code understandable
```

Skills will appear when you type `/` in GitHub Copilot Chat.

## Option B — Personal skills (available across all projects)

1. **Clone the repository** (to any location you prefer):
   ```bash
   git clone https://github.com/yaroher/understandable.git ~/understandable
   ```

2. **Create a symlink for each skill** into `~/.copilot/skills/`:
   ```bash
   mkdir -p ~/.copilot/skills
   SKILLS_DIR=~/understandable/skills
   for skill in "$SKILLS_DIR"/*/; do
     ln -sf "$skill" ~/.copilot/skills/$(basename "$skill")
   done
   # Universal plugin root symlink — lets the dashboard skill find packages/dashboard/
   # Skip if already exists (e.g. another platform was installed first)
   [ -e ~/.understandable ] || [ -L ~/.understandable ] || \
     ln -s ~/understandable/ ~/.understandable
   ```

   **Windows (PowerShell):**
   ```powershell
   New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.copilot\skills"
   $skillsDir = "$env:USERPROFILE\understandable\skills"
   Get-ChildItem $skillsDir -Directory | ForEach-Object {
     cmd /c mklink /J "$env:USERPROFILE\.copilot\skills\$($_.Name)" $_.FullName
   }
   cmd /c mklink /J "$env:USERPROFILE\.understandable" "$env:USERPROFILE\understandable"
   ```

3. **Reload VS Code** (`Cmd+Shift+P` → `Developer: Reload Window`) so GitHub Copilot discovers the skills.

## Verify

Type `/` in GitHub Copilot Chat — you should see all six skills listed:

- `understand` — build the knowledge graph
- `understand-chat` — ask questions about the codebase
- `understand-dashboard` — open the interactive dashboard
- `understand-diff` — analyze impact of current changes
- `understand-explain` — deep-dive into a file or function
- `understand-onboard` — generate an onboarding guide

## Usage

Skills activate automatically when relevant. You can also invoke them directly by typing `/` in Copilot Chat and selecting a skill.

## Updating

```bash
cd ~/understandable && git pull
```

Skills update instantly through the symlinks.

## Uninstalling

```bash
for skill in understand understand-chat understand-dashboard understand-diff understand-domain understand-explain understand-knowledge understand-onboard; do
  rm -f ~/.copilot/skills/$skill
done
rm -f ~/.understandable
rm -rf ~/understandable
```
