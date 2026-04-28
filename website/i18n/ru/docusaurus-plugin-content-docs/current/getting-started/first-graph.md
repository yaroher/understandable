---
title: Первый граф
sidebar_position: 2
---

# Первый граф

Конкретный end-to-end сценарий: создаём конфиг, строим граф знаний,
гоним эмбеддинги, открываем панель. Пара минут на маленьком проекте.

Предусловие: бинарник `understandable` есть в `PATH`. Если ещё нет —
смотри [Установку](./install).

## 1. Выбираем реальный проект

```bash
cd /path/to/your/repo
```

Подойдёт любой исходный репозиторий. Walker уважает `.gitignore`, так
что `node_modules/`, артефакты сборки и вендоренные зависимости
отбрасываются автоматически.

## 2. Скаффолдим конфиг — `understandable init`

`init` пишет `understandable.yaml` в корень проекта и обновляет
`.gitignore` управляемым блоком. Выбери пресет под то, что у тебя
есть:

```bash
# Только эвристика — без LLM, без эмбеддингов. Самый быстрый. CI-friendly.
understandable init --preset minimal

# Ollama-эмбеддинги + host-LLM (без API-ключей, всё локально).
understandable init --preset local-full

# Anthropic-LLM + OpenAI-эмбеддинги (лучшее качество, нужны оба ключа).
understandable init --preset cloud-full
```

Превью без записи — `--dry-run`:

```bash
understandable init --dry-run --preset cloud-full
```

См. [`init`](../cli/init) — каждое поле YAML имеет свой флаг.

:::tip
В IDE с загруженным скиллом `understand-setup` просто скажи
**«настрой understandable»** / **"set up understandable"** — мастер
сам разберётся с детектом окружения, выбором пресета и первым
прогоном analyze + embed.
:::

## 3. Бутстрапим правила игнора — `scan --gen-ignore`

```bash
understandable scan --gen-ignore
```

Это посеет `.understandignore` из твоего `.gitignore` плюс набора
разумных дефолтов (`target/`, `.venv/`, `dist/`, `.idea/`, …). Этот
файл потом редактируешь руками, чтобы выкинуть шумные директории.

## 4. Строим граф — `understandable analyze`

```bash
understandable analyze
```

Вывод:

```
analysis complete: 412 files → 3,084 nodes, 5,221 edges, 7 layers, 12 tour steps
```

Что произошло:
1. Walker нашёл все исходники, уважая `.gitignore` +
   `.understandignore` + `ignore.paths` из `understandable.yaml`.
2. Tree-sitter извлёк символы, вызовы, импорты и структурную мету.
3. `GraphBuilder` собрал типизированный граф.
4. Прогнался детектор слоёв и эвристический генератор тура.
5. Всё запаковалось в `.understandable/graph.tar.zst`.

## 5. (Опционально) LLM-обогащение — `analyze --with-llm`

```bash
export ANTHROPIC_API_KEY=sk-ant-...
understandable analyze --with-llm --llm-max-files 50
```

Каждый файл получает one-shot сводку + теги + оценку сложности.
Output кешируется по blake3-фингерпринту файла, так что повторный
прогон на неизменных файлах бесплатен. См. [`analyze`](../cli/analyze)
про экономию.

## 6. Строим семантический индекс — `understandable embed`

```bash
understandable embed
```

Эмбеддит текст каждого узла (`name :: summary :: tags`) и складывает
векторы в тот же `graph.tar.zst`. Повторный прогон дешёвый — строки,
у которых хеш текста не изменился, пропускаются.

## 7. Открываем панель — `understandable dashboard`

```bash
understandable dashboard
```

Открывает `http://127.0.0.1:5173` в браузере. Получаешь:

- **Графовое представление** — пан, зум, клик по узлу показывает
  соседей.
- **Поисковую строку** — по умолчанию подстрока + fuzzy; включи
  переключатель `semantic` для ранжирования по косинусной близости.
- **Панель слоёв** — автодетектированные архитектурные слои.
- **Tour-режим** — пошаговый проход по эвристическому туру как
  guided-знакомство.

## 8. Пробуем поиск

В панели ищи **«auth»** или что-то, что точно есть в коде. Кликни
узел. Панель соседей покажет все узлы, связанные ребром — call-сайты,
импорты, ссылки.

Из терминала:

```bash
understandable search "auth"
understandable search --semantic "user authentication flow"
understandable explain src/auth.ts:login
```

## Что дальше

- [`init`](../cli/init) — каждый конфиг-параметр.
- [`analyze`](../cli/analyze) — флаги, экономия, инкрементальный режим.
- [`embed`](../cli/embed) — выбор провайдера, смена модели.
- [`dashboard`](../cli/dashboard) — мульти-граф, проброс на LAN.
- [Архитектура](../architecture) — что физически лежит внутри
  `tar.zst`.
