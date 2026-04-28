---
title: Архитектура
sidebar_position: 99
---

# Архитектура

`understandable` — это Rust-воркспейс из 8 крейтов плюс React-панель,
склеенные в один бинарник и один формат архива. На этой странице —
движущиеся части и как они связаны.

## Layout воркспейса

```
crates/
  ua-core      — типы, схема, валидация
  ua-extract   — tree-sitter (tier1 + tier2) + строковые парсеры
  ua-analyzer  — graph builder, детектор слоёв, генератор тура,
                 нормалайзер, domain-экстрактор, wiki-импорт
  ua-search    — nucleo fuzzy + chat-context builder
  ua-persist   — IndraDB + usearch + tar.zst + blake3-фингерпринты
  ua-llm       — Anthropic + OpenAI-совместимые эмбеддинги + Ollama
  ua-server    — axum-сервер с React-бандлом, встроенным rust-embed
  ua-cli       — clap-биндинг бинарника `understandable`
dashboard/     — React 19 + xyflow + zustand + tailwind v4
```

**`ua-core`** держит канонические типы (`KnowledgeGraph`,
`GraphNode`, `GraphEdge`, `ProjectSettings`) плюс валидацию схемы.
Все остальные крейты потребляют эти типы — схема и есть контракт.

**`ua-extract`** гоняет tree-sitter и строковые парсеры. 11 tier-1
грамматик (TypeScript/TSX, JavaScript, Python, Go, Rust, Java, Ruby,
PHP, C, C++, C#) включены всегда; ~30 tier-2 грамматик (Bash, Lua,
Zig, Swift, OCaml, Elixir, …) сидят за `--features all-langs`.
Кастомные строковые парсеры обрабатывают Dockerfile, Makefile,
`.env`, INI.

**`ua-analyzer`** — graph builder. Берёт per-file метаданные из
`ua-extract` плюс опциональные LLM-сводки и производит типизированный
граф, назначения слоёв и эвристический тур.

**`ua-search`** оборачивает [`nucleo`][nucleo] для fuzzy-матчинга
поверх UTF-32 кеша свойств вершин (`name_lower`, `summary_lower`,
`tags_text`). chat-context builder превращает запрос в срез графа,
по которому LLM может ответить.

**`ua-persist`** — слой хранения: IndraDB MemoryDatastore + usearch
ANN-индекс + tar.zst-архив + blake3-фингерпринты + walker через крейт
`ignore`.

**`ua-llm`** держит Anthropic-клиент, OpenAI-совместимый клиент
эмбеддингов (используется и для OpenAI, и для Ollama) и мост к
fastembed-rs ONNX (gated на `--features local-embeddings`). Prompt
caching автоматически включён для системного промпта Anthropic.

**`ua-server`** — axum-приложение. React-бандл встраивается
[`rust-embed`][rust-embed] на этапе компиляции, так что бинарник
несёт в себе всю панель.

**`ua-cli`** — clap-based точка входа. Каждая подкоманда (`analyze`,
`embed`, `dashboard`, `init`, …) живёт в
`crates/ua-cli/src/commands/`.

## Формат хранения

Per-project состояние лежит в `<project_root>/.understandable/`:

```
graph.tar.zst              — codebase-граф (по умолчанию)
graph.domain.tar.zst       — опциональный, пишет `understandable domain`
graph.knowledge.tar.zst    — опциональный, пишет `understandable knowledge`
config.json                — {autoUpdate}
```

Каждый архив **аддитивный** — все артефакты в одном tarball, так что
граф едет одним файлом, а не россыпью JSON. Содержимое:

```text
meta.json            — версия схемы, штамп project_root, фингерпринты,
                       слои, данные тура, мета эмбеддингов (model → dim).
id_map.bincode       — бизнес-ключ → UUID v5 (детерминированный).
graph.msgpack        — IndraDB MemoryDatastore msgpack-снапшот.
embeddings.bin       — сырые f32-векторы с маленьким bincode-заголовком.
```

ID вершин выводятся через `Uuid::new_v5(<fixed namespace>, <business
key>)` — стабильны между пересборками. Повторный `analyze` на тех же
входах даёт те же UUID.

Сырые f32-векторы в `embeddings.bin` — источник истины; HNSW-индекс
[`usearch`][usearch] сидит сверху и lazy перестраивается после
мутаций. Индекс одноразовый — будущий major-bump usearch не лишит
тебя данных.

## Покрытие tree-sitter

- **Tier 1** (default, полная экстракция + call graph): TypeScript /
  TSX, JavaScript, Python, Go, Rust, Java, Ruby, PHP, C, C++, C#.
- **Tier 2** (`--features all-langs`): Bash, Lua, Zig, Dart, Swift,
  Scala, Haskell, OCaml, Elixir, Erlang, Elm, Julia, Scheme, Solidity,
  Perl, Fortran, D, F#, Groovy, Objective-C, CUDA, GLSL, HLSL,
  Verilog, VHDL, CMake, Make, Nix, Vim script, Fish, jq, HCL.
- **Tier 3** (только метаданные): HTML, CSS, JSON, YAML, TOML, XML,
  Markdown, regex.

## LLM и эмбеддинги

| Слой        | Провайдеры                                          |
|-------------|------------------------------------------------------|
| LLM         | Anthropic (по умолчанию), `host` (делегат IDE)      |
| Эмбеддинги  | OpenAI (по умолчанию), Ollama, локальный fastembed-rs ONNX |

OpenAI и Ollama переиспользуют один и тот же HTTP-клиент
`OpenAiEmbeddings` — отличаются только base URL и auth.

### Экономия

| Knob               | Что делает                                                                                       |
|--------------------|---------------------------------------------------------------------------------------------------|
| Prompt caching     | Авто-вкл для системного промпта Anthropic. Первый вызов пишет (1.25×), последующие читают ~0.1×. |
| Batch API          | `ua_llm::BatchClient` — скидка 50 %, SLA 24 ч. Используй для офлайн-обогащения.                  |
| Output cache       | `analyze --with-llm` ключует ответы по per-file blake3. Повторы по неизменным файлам = бесплатно. |
| Concurrency caps   | `llm.concurrency` (по умолчанию 4), `embeddings.concurrency` (по умолчанию 2). Ограничены `Semaphore`. |

## Поиск

Два пути по одному и тому же графу:

- **`search "<query>"`** — token-AND scan по подстрокам в колонках
  `name_lower` / `summary_lower` / `tags_text`, переранжирование
  через [`nucleo`][nucleo] поверх UTF-32 кеша.
- **`search --semantic "<query>"`** — косинусная близость поверх
  HNSW-индекса usearch. Требует заполненный `embeddings.bin`.

Оба варианта in-memory и завершаются за миллисекунды на 10k-узловом
графе.

## Панель (Dashboard)

`ua-server` поднимает axum-приложение на `127.0.0.1:5173`
(конфигурируется). Фронтенд React 19 + xyflow + zustand + tailwind
v4 живёт в `dashboard/`; `pnpm build` производит статический бандл,
который `rust-embed` всасывает в бинарник на этапе компиляции.
Отдельного фронтенд-деплоя нет.

## Plug-in поверхность

Бинарник — это субстрат. IDE-side опыт целиком markdown-овый:

- **9 IDE-манифестов** — `.claude-plugin/`, `.cursor-plugin/`,
  `.copilot-plugin/`, `.codex/`, `.opencode/`, `.openclaw/`,
  `.gemini/`, `.pi/`, `.antigravity/`, `.vscode/`. Каждый адаптирует
  один и тот же набор скилов/агентов/хуков под манифест-схему своей
  IDE.
- **`agents/`** — 9 markdown-агентов (file-analyzer,
  architecture-analyzer, …), которых IDE зовёт, когда пользователь
  задаёт вопрос.
- **`skills/`** — 8 markdown slash-команд (`/understand`,
  `/understand-setup`, `/install-understandable`, …).
- **`hooks/`** — post-commit хук + промпт, который гоняет
  `analyze --incremental --plan-only` и решает, звать ли LLM.

## Поток данных

```
                      ┌──────────────────────────┐
                      │  source repo (cwd)       │
                      └────────────┬─────────────┘
                                   │
                                   ▼
            ┌──────────────────────────────────────────┐
            │  ua-extract  (tree-sitter, line parsers) │
            └────────────────────┬─────────────────────┘
                                 │ FileMeta
                                 ▼
            ┌──────────────────────────────────────────┐
            │  ua-analyzer  (GraphBuilder, layers,     │
            │                tour, optional LLM enrich)│
            └────────────────────┬─────────────────────┘
                                 │ KnowledgeGraph
                                 ▼
            ┌──────────────────────────────────────────┐
            │  ua-persist  (IndraDB + usearch HNSW)    │
            └────────────────────┬─────────────────────┘
                                 │
                                 ▼
                    ┌────────────────────────┐
                    │ .understandable/       │
                    │   graph.tar.zst        │
                    │   graph.domain.tar.zst │
                    │   graph.knowledge…     │
                    └──────┬──────────┬──────┘
                           │          │
                ┌──────────▼──┐    ┌──▼─────────────────────┐
                │ ua-server   │    │ IDE-агенты / скилы     │
                │ (dashboard) │    │ (file-analyzer, …)     │
                └─────────────┘    └────────────────────────┘
```

Архив — это контракт. Панель его читает; IDE-side агенты его читают.
Кто угодно может написать нового потребителя, который открывает
`graph.tar.zst` и обходит граф — это просто msgpack + bincode +
сырые f32.

[nucleo]: https://crates.io/crates/nucleo-matcher
[usearch]: https://crates.io/crates/usearch
[rust-embed]: https://crates.io/crates/rust-embed
