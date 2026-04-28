# Installing understandable for Antigravity

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
   git clone https://github.com/yaroher/understandable.git ~/.antigravity/understandable
   ```

2. **Create the skills symlinks:**
   ```bash
   mkdir -p ~/.gemini/antigravity/skills
   ln -s ~/.antigravity/understandable//skills ~/.gemini/antigravity/skills/understandable
   # Universal plugin root symlink — lets the dashboard skill find packages/dashboard/
   # Skip if already exists (e.g. another platform was installed first)
   [ -e ~/.understandable ] || [ -L ~/.understandable ] || ln -s ~/.antigravity/understandable/ ~/.understandable
   ```

   **Windows (PowerShell):**
   ```powershell
   New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.gemini\antigravity\skills"
   cmd /c mklink /J "$env:USERPROFILE\.gemini\antigravity\skills\understandable" "$env:USERPROFILE\.antigravity\understandable\skills"
   cmd /c mklink /J "$env:USERPROFILE\.understandable" "$env:USERPROFILE\.antigravity\understandable"
   ```

3. **Restart the chat or IDE** so Antigravity can discover the skills.

## Verify

```bash
ls -la ~/.gemini/antigravity/skills/understandable
```

You should see a symlink pointing to the skills directory in the cloned repo.

## Usage

Skills activate automatically when relevant. You can also invoke directly by saying:
- "Run the understand skill to analyze this codebase"
- "Use the understand-dashboard skill to view the architecture map"
- "Use understand-chat to answer a question about the graph"

## Updating

```bash
cd ~/.antigravity/understandable && git pull
```

Skills update instantly through the symlink.

## Uninstalling

```bash
rm ~/.gemini/antigravity/skills/understandable
rm ~/.understandable
rm -rf ~/.antigravity/understandable
```
