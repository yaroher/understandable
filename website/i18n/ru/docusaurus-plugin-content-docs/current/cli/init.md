---
title: init
sidebar_position: 4
---

# `understandable init`

Скаффолдит (или обновляет) `understandable.yaml` в корне проекта.
Каждое поле конфига доступно через флаг, чтобы LLM-led мастер
`understand-setup` мог собрать любую комбинацию детерминированно.

## Синопсис

```bash
understandable init [--preset {minimal,local-full,cloud-full}] \
                    [--dry-run] [--force] [--no-merge] [--no-backup] \
                    [--no-gitignore] \
                    [--<любое-поле>=<значение>...]
```

## Пресеты

Три опиниатед-набора закрывают типичные сценарии деплоя:

| Пресет       | Эмбеддинги                          | LLM                              | Когда выбирать                                  |
|--------------|-------------------------------------|----------------------------------|------------------------------------------------|
| `minimal`    | нет                                 | нет                              | CI, eval-прогоны, маленькие репы. Только эвристика. |
| `local-full` | Ollama (`nomic-embed-text`)         | Host-LLM (без API-ключа)         | Air-gapped команды; дёшево гонять постоянно.   |
| `cloud-full` | OpenAI (`text-embedding-3-small`)   | Anthropic (`claude-opus-4-7`)    | Лучшее качество. Нужны оба API-ключа.          |

Порядок применения: **`recommended() → preset → individual flags`**,
так что любой явный флаг побеждает пресет.

```bash
understandable init --preset cloud-full
understandable init --preset local-full --embed-model bge-m3   # мультиязычное переопределение
```

## Каждое поле YAML — это флаг

У каждой секции `ProjectSettings` есть свой флаг. Маппинг:

| YAML-путь                          | Флаг                                   |
|------------------------------------|----------------------------------------|
| `project.name`                     | `--project-name <s>`                   |
| `project.description`              | `--project-description <s>`            |
| `storage.dir`                      | `--storage-dir <path>`                 |
| `storage.db_name`                  | `--db-name <stem>`                     |
| `embeddings.provider`              | `--embed-provider {openai,ollama,local}` |
| `embeddings.model`                 | `--embed-model <id>`                   |
| `embeddings.endpoint`              | `--embed-endpoint <url>`               |
| `embeddings.batch_size`            | `--embed-batch-size <N>`               |
| `embeddings.embed_on_analyze`      | `--embed-on-analyze {true,false}`      |
| `embeddings.concurrency`           | `--embed-concurrency <N>`              |
| `llm.provider`                     | `--llm-provider {anthropic,host}`      |
| `llm.model`                        | `--llm-model <id>`                     |
| `llm.max_files`                    | `--llm-max-files <N>`                  |
| `llm.temperature`                  | `--llm-temperature <f32>`              |
| `llm.run_on_analyze`               | `--llm-run-on-analyze {true,false}`    |
| `llm.concurrency`                  | `--llm-concurrency <N>`                |
| `ignore.paths` (повторяемо)        | `--ignore-path <prefix>`               |
| `incremental.full_threshold`       | `--incremental-full-threshold <N>`     |
| `incremental.big_graph_threshold`  | `--incremental-big-graph-threshold <N>`|
| `dashboard.host`                   | `--dashboard-host <ip>`                |
| `dashboard.port`                   | `--dashboard-port <N>`                 |
| `dashboard.auto_open`              | `--dashboard-auto-open {true,false}`   |
| `git.commit_db`                    | `--git-commit-db {true,false}`         |
| `git.commit_embeddings`            | `--git-commit-embeddings {true,false}` |

`--embed-provider` — value-enum. Опечатки (`olama` vs `ollama`)
отвергаются clap'ом на этапе парсинга и в YAML не попадают.

## `--dry-run` (только превью)

Печатает результирующий YAML в stdout. **Никаких побочных эффектов
на ФС** — никакого `create_dir_all`, никакого обновления
`.gitignore`, никакой канонизации.

```bash
understandable init --dry-run --preset cloud-full --embed-model bge-m3
```

LLM-led мастер сначала запускает это, чтобы показать пользователю
план до записи.

## `--force` (мердж с бэкапом)

`init` по умолчанию отказывается перетирать существующий
`understandable.yaml`. С `--force` он:

1. Сохраняет старый файл как `understandable.yaml.bak` (выкл. через
   `--no-backup`).
2. **Мерджит** существующий YAML с пресетом и CLI-переопределениями.
   Поля, набитые руками и не переданные в командной строке,
   выживают.
3. Пишет новый YAML.

Чтобы пропустить мердж — `--no-merge` (чистая перезапись, выживают
только пресет + CLI-флаги).

:::caution
У `serde_yaml_ng` нет режима с сохранением комментариев, так что
комментарии из старого файла во время мерджа теряются. `init` об
этом предупреждает, а `.bak` позволяет восстановить, что важно.
:::

## Авто-управляемый блок в gitignore

Если не передать `--no-gitignore`, `init` пишет (или переписывает на
месте) управляемый блок в `<project>/.gitignore` между маркерами
`# >>> understandable >>>` и `# <<< understandable <<<`.

Содержимое блока зависит от `git.commit_db`:

**`commit_db: true` (по умолчанию, рекомендуется):**

```gitignore
# >>> understandable >>>
# managed by `understandable init` — leave the DB tracked.
.understandable/intermediate/
.understandable/tmp/
# <<< understandable <<<
```

**`commit_db: false`:**

```gitignore
# >>> understandable >>>
# managed by `understandable init` — DB stays out of git.
.understandable/
# <<< understandable <<<
```

Повторный прогон `init` обновляет блок in place. Hand-written записи
снаружи маркеров не трогаются.

## Русский / мультиязычный контент

Когда в проекте доки или строки на не-английских языках — выбирай
мультиязычную модель эмбеддингов. `bge-m3` на Ollama — безопасный
дефолт:

```bash
ollama pull bge-m3
understandable init --preset local-full --embed-model bge-m3
```

Для OpenAI хорошо тянет 100+ языков `text-embedding-3-large`; для
локального провайдера — `paraphrase-multilingual-mpnet-base-v2`.

## Примеры

### Превью перед записью

```bash
understandable init --dry-run --preset cloud-full
```

### CI-friendly, только эвристика

```bash
understandable init --preset minimal --no-gitignore
```

### Обновить существующий конфиг — привязать панель к LAN

```bash
understandable init --force \
  --dashboard-host 0.0.0.0 --dashboard-auto-open false
```

### Air-gapped команда, мультиязычный контент

```bash
understandable init --preset local-full --embed-model bge-m3 \
  --git-commit-db true
```

## См. также

- [`analyze`](./analyze) — первое место, где потребляется каждое
  поле YAML.
- [`embed`](./embed) — выбор провайдера в рантайме.
- [Архитектура](../architecture) — что значит выбранный layout
  хранения.
