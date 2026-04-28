# Installing understandable for OpenClaw

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
   git clone https://github.com/yaroher/understandable.git ~/.openclaw/understandable
   ```

2. **Create the skills symlinks:**
   ```bash
   mkdir -p ~/.openclaw/skills
   ln -s ~/.openclaw/understandable//skills ~/.openclaw/skills/understandable
   # Universal plugin root symlink — lets the dashboard skill find packages/dashboard/
   # Skip if already exists (e.g. another platform was installed first)
   [ -e ~/.understandable ] || [ -L ~/.understandable ] || ln -s ~/.openclaw/understandable/ ~/.understandable
   ```

   **Windows (PowerShell):**
   ```powershell
   New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.openclaw\skills"
   cmd /c mklink /J "$env:USERPROFILE\.openclaw\skills\understandable" "$env:USERPROFILE\.openclaw\understandable\skills"
   cmd /c mklink /J "$env:USERPROFILE\.understandable" "$env:USERPROFILE\.openclaw\understandable"
   ```

3. **Restart OpenClaw** to discover the skills.

## Usage

- `@understand` — Analyze the current codebase
- `@understand-chat` — Ask questions about the knowledge graph
- `@understand-dashboard` — Launch the interactive dashboard

## Updating

```bash
cd ~/.openclaw/understandable && git pull
```

## Uninstalling

```bash
rm ~/.openclaw/skills/understandable
rm ~/.understandable
rm -rf ~/.openclaw/understandable
```
