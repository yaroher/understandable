# Installing understandable for OpenCode

> **Heads-up:** the markdown skills/agents in this plugin shell out to
> the Rust `understandable` binary. Install it once before completing
> the steps below:
>
> ```bash
> cargo install --git https://github.com/yaroher/understandable understandable
> ```

## Prerequisites

- Git
- [OpenCode](https://opencode.ai) installed

## Installation

1. **Clone the repository:**
   ```bash
   git clone https://github.com/yaroher/understandable.git ~/.opencode/understandable
   ```

2. **Create the skills symlinks:**
   ```bash
   mkdir -p ~/.agents/skills
   # Note: if Codex's understandable is already installed, these symlinks
   # already exist and the ln commands will safely fail — that is fine, the
   # existing symlinks work for OpenCode too.
   for skill in understand understand-chat understand-dashboard understand-diff understand-domain understand-explain understand-knowledge understand-onboard understand-setup; do
     ln -sf ~/.opencode/understandable/skills/$skill ~/.agents/skills/$skill
   done
   # Universal plugin root symlink — lets the dashboard skill find packages/dashboard/
   # Skip if already exists (e.g. another platform was installed first)
   [ -e ~/.understandable ] || [ -L ~/.understandable ] || ln -s ~/.opencode/understandable/ ~/.understandable
   ```

   **Windows (PowerShell):**
   ```powershell
   New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.agents\skills"
   $skills = @("understand","understand-chat","understand-dashboard","understand-diff","understand-domain","understand-explain","understand-knowledge","understand-onboard","understand-setup")
   foreach ($skill in $skills) {
     cmd /c mklink /J "$env:USERPROFILE\.agents\skills\$skill" "$env:USERPROFILE\.opencode\understandable\skills\$skill"
   }
   # Universal plugin root symlink
   cmd /c mklink /J "$env:USERPROFILE\.understandable" "$env:USERPROFILE\.opencode\understandable"
   ```

3. **Restart OpenCode** to discover the skills.

## Verify

```bash
ls -la ~/.agents/skills/ | grep understand
```

You should see symlinks for each skill pointing into the cloned repository.

## Usage

Skills activate automatically when relevant. You can also invoke directly:

```
use skill tool to load understand
```

Or just ask: "Analyze this codebase and build a knowledge graph"

## Updating

```bash
cd ~/.opencode/understandable && git pull
```

Skills update instantly through the symlinks.

## Uninstalling

```bash
for skill in understand understand-chat understand-dashboard understand-diff understand-domain understand-explain understand-knowledge understand-onboard understand-setup; do
  rm -f ~/.agents/skills/$skill
done
rm ~/.understandable
rm -rf ~/.opencode/understandable
```

## Troubleshooting

### Skills not found

1. Check that the symlinks exist: `ls -la ~/.agents/skills/ | grep understand`
2. Verify the clone succeeded: `ls ~/.opencode/understandable/skills/`
3. Restart OpenCode

### Tool mapping

When skills reference Claude Code tools:
- `TodoWrite` → `todowrite`
- `Task` with subagents → `@mention` syntax
- `Skill` tool → OpenCode's native `skill` tool
- File operations → your native tools
