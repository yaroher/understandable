# Installing understandable for Gemini CLI

> **Heads-up:** the markdown skills/agents in this plugin shell out to
> the Rust `understandable` binary. Install it once before completing
> the steps below:
>
> ```bash
> cargo install --git https://github.com/yaroher/understandable understandable
> ```

## Prerequisites

- Git
- [Gemini CLI](https://github.com/google-gemini/gemini-cli) installed

## Installation

1. **Clone the repository:**
   ```bash
   git clone https://github.com/yaroher/understandable.git ~/.gemini/understandable
   ```

2. **Create the skills symlinks:**
   ```bash
   mkdir -p ~/.agents/skills
   # Note: if another platform's understandable is already installed, these symlinks
   # already exist and the ln commands will safely fail — that is fine, the
   # existing symlinks work for Gemini CLI too.
   for skill in understand understand-chat understand-dashboard understand-diff understand-domain understand-explain understand-knowledge understand-onboard understand-setup; do
     ln -sf ~/.gemini/understandable/skills/$skill ~/.agents/skills/$skill
   done
   # Universal plugin root symlink — lets the dashboard skill find packages/dashboard/
   # Skip if already exists (e.g. another platform was installed first)
   [ -e ~/.understandable ] || [ -L ~/.understandable ] || ln -s ~/.gemini/understandable/ ~/.understandable
   ```

   **Windows (PowerShell):**
   ```powershell
   New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.agents\skills"
   $skills = @("understand","understand-chat","understand-dashboard","understand-diff","understand-domain","understand-explain","understand-knowledge","understand-onboard","understand-setup")
   foreach ($skill in $skills) {
     cmd /c mklink /J "$env:USERPROFILE\.agents\skills\$skill" "$env:USERPROFILE\.gemini\understandable\skills\$skill"
   }
   # Universal plugin root symlink
   cmd /c mklink /J "$env:USERPROFILE\.understandable" "$env:USERPROFILE\.gemini\understandable"
   ```

3. **Restart Gemini CLI** to discover the skills.

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
cd ~/.gemini/understandable && git pull
```

Skills update instantly through the symlinks.

## Uninstalling

```bash
for skill in understand understand-chat understand-dashboard understand-diff understand-domain understand-explain understand-knowledge understand-onboard understand-setup; do
  rm -f ~/.agents/skills/$skill
done
rm ~/.understandable
rm -rf ~/.gemini/understandable
```
