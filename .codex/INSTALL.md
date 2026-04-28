# Installing understandable for Codex

> **Heads-up:** the markdown skills/agents in this plugin shell out to
> the Rust `understandable` binary. Install it once before completing
> the steps below:
>
> ```bash
> cargo install --git https://github.com/yaroher/understandable understandable
> ```

## Prerequisites

- Git

## Installation

1. **Clone the repository:**
   ```bash
   git clone https://github.com/yaroher/understandable.git ~/.codex/understandable
   ```

2. **Create the skills symlinks:**
   ```bash
   mkdir -p ~/.agents/skills
   # Note: if OpenCode's understandable is already installed, these symlinks
   # already exist and the ln commands will safely fail — that is fine, the
   # existing symlinks work for Codex too.
   for skill in understand understand-chat understand-dashboard understand-diff understand-domain understand-explain understand-knowledge understand-onboard understand-setup; do
     ln -sf ~/.codex/understandable/skills/$skill ~/.agents/skills/$skill
   done
   # Universal plugin root symlink — lets the dashboard skill find packages/dashboard/
   # Skip if already exists (e.g. another platform was installed first)
   [ -e ~/.understandable ] || [ -L ~/.understandable ] || ln -s ~/.codex/understandable/ ~/.understandable
   ```

   **Windows (PowerShell):**
   ```powershell
   New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.agents\skills"
   $skills = @("understand","understand-chat","understand-dashboard","understand-diff","understand-domain","understand-explain","understand-knowledge","understand-onboard","understand-setup")
   foreach ($skill in $skills) {
     cmd /c mklink /J "$env:USERPROFILE\.agents\skills\$skill" "$env:USERPROFILE\.codex\understandable\skills\$skill"
   }
   # Universal plugin root symlink
   cmd /c mklink /J "$env:USERPROFILE\.understandable" "$env:USERPROFILE\.codex\understandable"
   ```

3. **Restart Codex** to discover the skills.

## Verify

```bash
ls -la ~/.agents/skills/ | grep understand
```

You should see symlinks for each skill pointing into the cloned repository.

## Usage

Skills activate automatically when relevant. You can also invoke directly:
- "Analyze this codebase and build a knowledge graph"
- "Help me understand this project's architecture"

## Updating

```bash
cd ~/.codex/understandable && git pull
```

Skills update instantly through the symlinks.

## Uninstalling

```bash
for skill in understand understand-chat understand-dashboard understand-diff understand-domain understand-explain understand-knowledge understand-onboard understand-setup; do
  rm -f ~/.agents/skills/$skill
done
rm ~/.understandable
rm -rf ~/.codex/understandable
```
