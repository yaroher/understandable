---
title: Установка
sidebar_position: 1
---

# Установка

`understandable` поставляется одним Rust-бинарником. Три способа
установки; выбирай тот, что вписывается в твой обычный workflow.

## 1. Однострочный shell-инсталлятор (рекомендуется)

Самый быстрый путь. Скачивает готовый бинарник под твою платформу и
кладёт его в `PATH`.

```bash
curl -fsSL https://raw.githubusercontent.com/yaroher/understandable/main/install.sh | sh
```

Rust-тулчейн не нужен. Работает на Linux x86_64 / aarch64 и macOS
(Intel + Apple Silicon).

:::tip
В Claude Code, Cursor или любой IDE, в которой загружен скилл
`install-understandable`, просто скажи **«установи understandable»**
(или **"install understandable"**). Скилл прогонит установку,
проверит бинарник и запустит `--version`.
:::

## 2. `cargo binstall` (без тулчейна)

Если у тебя есть [`cargo-binstall`][binstall], но Rust-тулчейн на
машине ставить не хочется — этот способ тянет готовый артефакт прямо
из GitHub-релиза:

```bash
cargo binstall understandable
```

Тот же бинарник, что и shell-инсталлятор, тот же набор фич, что и в
опубликованном релизе.

## 3. `cargo install` из исходников (макс. гибкость)

Сборка из `git` нужна, когда хочешь поиграться с Cargo-фичами,
собрать неопубликованную ветку или ужать бинарник:

```bash
# Рекомендуется: все фичи включены (~80 МБ бинарник)
cargo install --git https://github.com/yaroher/understandable understandable \
  --features all-langs,local-embeddings

# Урезанные сборки (выкидывай только то, что точно не понадобится)
cargo install --git https://github.com/yaroher/understandable understandable
cargo install --git https://github.com/yaroher/understandable understandable --features all-langs
cargo install --git https://github.com/yaroher/understandable understandable --features local-embeddings
```

Матрица фич:

| Фича                | Что добавляет                                                       | Цена            |
|---------------------|---------------------------------------------------------------------|-----------------|
| (default)           | 11 tier-1 грамматик + OpenAI/Ollama-эмбеддинги по HTTP              | ~40 МБ          |
| `all-langs`         | + ~30 tier-2 грамматик (Bash, Lua, Swift, Zig, …)                   | +~25 МБ         |
| `local-embeddings`  | + fastembed-rs ONNX-рантайм + tokenizers + hf-hub                   | +~30 МБ на диск |

`local-embeddings` тянет ONNX-модель при первом запуске (~120 МБ
кешируется в `~/.cache/fastembed`).

## Заметки по платформам

### Linux

Работает из коробки на glibc-дистрибутивах (Fedora, Ubuntu, Arch,
Debian). Для Alpine / musl используй вариант 3 с `--target
x86_64-unknown-linux-musl`.

### macOS

Поддерживаются и Intel, и Apple Silicon. Если Gatekeeper ругается на
неподписанный бинарник, выполни один раз `xattr -d
com.apple.quarantine $(which understandable)`.

### Windows

Нативные сборки под Windows возможны через вариант 3, но история с
тестами и тулингом лучше всего работает на **WSL2 (Ubuntu)**.
Рекомендуем ставить внутри WSL — все примеры в этой документации
предполагают POSIX-шелл.

## Проверка установки

```bash
understandable --version
```

Если планируешь использовать локальный (офлайн) провайдер
эмбеддингов, ещё проверь, что фича вкомпилирована:

```bash
understandable embed --help | grep -q "local" && echo "local feature ON" \
  || echo "local feature OFF"
```

`local feature OFF` плюс желание гонять офлайн-ONNX-эмбеддинги
означает переустановку с `--features local-embeddings`.

## Что дальше

Иди в [Первый граф](./first-graph) — там сценарий: создаём конфиг и
строим первый граф знаний.

[binstall]: https://github.com/cargo-bins/cargo-binstall
