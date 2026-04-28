# Installing understandable for Copilot CLI

> **Heads-up:** the markdown skills/agents in this plugin shell out to
> the Rust `understandable` binary. Install it once before completing
> the steps below:
>
> ```bash
> cargo install --git https://github.com/yaroher/understandable understandable \
>   --features all-langs,local-embeddings
> ```

## Prerequisites

- [GitHub Copilot CLI](https://docs.github.com/en/copilot/github-copilot-in-the-cli) (`gh extension install github/gh-copilot`)
- Git

## Installation

1. **Clone the repository:**
   ```bash
   git clone https://github.com/yaroher/understandable.git ~/.copilot-plugin/understandable
   ```

2. **Create skill symlinks:**
   ```bash
   mkdir -p ~/.copilot/skills
   for skill in understand understand-chat understand-dashboard understand-diff \
       understand-domain understand-explain understand-knowledge understand-onboard understand-setup; do
     ln -sf ~/.copilot-plugin/understandable/skills/$skill ~/.copilot/skills/$skill
   done
   # Universal plugin root symlink — lets the dashboard skill find its assets
   [ -e ~/.understandable ] || [ -L ~/.understandable ] || \
     ln -s ~/.copilot-plugin/understandable/ ~/.understandable
   ```

   **Windows (PowerShell):**
   ```powershell
   New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.copilot\skills"
   $skills = @("understand","understand-chat","understand-dashboard","understand-diff",
               "understand-domain","understand-explain","understand-knowledge",
               "understand-onboard","understand-setup")
   foreach ($skill in $skills) {
     cmd /c mklink /J "$env:USERPROFILE\.copilot\skills\$skill" `
       "$env:USERPROFILE\.copilot-plugin\understandable\skills\$skill"
   }
   cmd /c mklink /J "$env:USERPROFILE\.understandable" `
     "$env:USERPROFILE\.copilot-plugin\understandable"
   ```

3. **Restart your terminal** (or `source ~/.bashrc` / `source ~/.zshrc`).

## Verify

```bash
ls -la ~/.copilot/skills/ | grep understand
```

You should see symlinks for each skill pointing into the cloned repository.

## Usage

Skills activate when you ask Copilot CLI to analyze or explain a codebase.
You can also invoke them explicitly in your shell conversations.

## Updating

```bash
cd ~/.copilot-plugin/understandable && git pull
```

Skills update instantly through the symlinks.

## Uninstalling

```bash
for skill in understand understand-chat understand-dashboard understand-diff \
    understand-domain understand-explain understand-knowledge understand-onboard understand-setup; do
  rm -f ~/.copilot/skills/$skill
done
rm -f ~/.understandable
rm -rf ~/.copilot-plugin/understandable
```

## Issues

File issues at https://github.com/yaroher/understandable/issues
