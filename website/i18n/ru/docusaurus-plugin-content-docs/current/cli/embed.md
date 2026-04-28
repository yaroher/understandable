---
title: embed
sidebar_position: 3
---

# `understandable embed`

Массово эмбеддит каждый узел персистентного графа и складывает f32-
векторы в тот же `graph.tar.zst`. Когда индекс заполнен,
`search --semantic` ранжирует узлы по косинусной близости, не
переэмбеддя корпус на каждый запрос.

## Синопсис

```bash
understandable embed [--embed-provider {openai,ollama,local}] \
                     [--embed-model <id>] [--embed-endpoint <url>] \
                     [--reset] [--force] [--batch-size <N>]
```

## Три провайдера

| Провайдер | Модель по умолчанию          | Аутентификация    | Заметки                                                                                                                         |
|-----------|------------------------------|-------------------|---------------------------------------------------------------------------------------------------------------------------------|
| `openai`  | `text-embedding-3-small`     | `OPENAI_API_KEY`  | Дефолт. Бьёт в `api.openai.com`. Через `--embed-endpoint` можно говорить с любым OpenAI-совместимым сервером.                  |
| `ollama`  | `nomic-embed-text`           | нет               | Бьёт в `http://127.0.0.1:11434`. Один раз сделай `ollama pull nomic-embed-text`.                                               |
| `local`   | `bge-small-en-v1.5`          | нет               | ONNX in-process через fastembed-rs. **Требует `--features local-embeddings` на этапе компиляции.** Тянет модель при первом запуске. |

Cloud- и Ollama-пути переиспользуют один и тот же HTTP-клиент
`OpenAiEmbeddings` под капотом — отличаются только base URL и auth.

## Выбор провайдера

```bash
# OpenAI (дефолт, если нет флага и нет YAML-настройки)
understandable embed

# Ollama
understandable embed --embed-provider ollama --embed-model bge-m3

# Локальный ONNX (офлайн)
understandable embed --embed-provider local --embed-model bge-small
```

Порядок разрешения, как и везде в бинарнике:

1. CLI-флаг (`--embed-provider`).
2. `embeddings.provider` в `understandable.yaml`.
3. Дефолт: `openai`.

## Повторный прогон дешёвый

Каждая строка ключуется blake3-хешем текста узла (`name :: summary
:: tags`). Когда хеш совпадает с сохранённым — строка пропускается.
Повторный прогон на неизменном графе стоит ноль API-запросов.

`analyze --incremental` автоматически инвалидирует эмбеддинги
изменённых/удалённых узлов, так что следующий `embed`-вызов
обновляет только затронутые строки.

## Смена модели — `--reset`

У каждой модели фиксированная размерность вектора. Переключение на
модель с другой размерностью — это hard reset; предыдущие векторы
надо сбросить:

```bash
understandable embed --reset --embed-model text-embedding-3-large
```

`--reset` дропает все существующие эмбеддинги для названной модели
перед прогоном. `--force` похож, но не дропает (просто пере-эмбеддит);
используй, когда поведение модели изменилось за стабильным id.

:::tip
Для мультиязычного контента (русские доки, китайские комментарии,
смешанные кодовые базы) бери мультиязычную модель: `bge-m3` на
Ollama или `paraphrase-multilingual-mpnet-base-v2` на локальном
провайдере.
:::

## Конкурентность

`embeddings.concurrency` (по умолчанию 2) контролирует, сколько
запросов к провайдеру идут параллельно. Каждая задача делает только
I/O — upsert-ы в storage остаются на главной задаче, чтобы не
конкурировать за async-mutex'ом. Настраивается в YAML:

```yaml
embeddings:
  concurrency: 4
  batch_size: 32
```

`batch_size` — текстов-на-запрос. Дефолт 32 консервативный; OpenAI
принимает до 2048, Ollama обычно медленнее на батче.

## Примеры

### Первый прогон embed

```bash
understandable embed
# embedded 3084/3084 node(s) into `text-embedding-3-small` (dim=1536)
```

### Перейти с OpenAI на мультиязычную Ollama-модель

```bash
ollama pull bge-m3
understandable embed --reset --embed-provider ollama --embed-model bge-m3
```

### OpenAI-совместимый сервер (vLLM, LiteLLM и т. п.)

```bash
understandable embed --embed-provider openai \
  --embed-endpoint https://my-llm-gateway.example.com/v1
```

### Принудительно пере-эмбеддить всё

```bash
understandable embed --force
```

## См. также

- [`analyze`](./analyze) — сначала собери граф.
- [`init`](./init) — задай провайдер, модель и конкурентность в YAML.
- [Архитектура](../architecture) — формат хранения векторов.
