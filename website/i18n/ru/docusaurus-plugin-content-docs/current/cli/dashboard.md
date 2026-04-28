---
title: dashboard
sidebar_position: 2
---

# `understandable dashboard`

Поднимает встроенный axum-сервер и отдаёт React-UI. Панель встраивается
в бинарник на этапе компиляции через `rust-embed` — отдельного
фронтенда устанавливать не надо.

## Синопсис

```bash
understandable dashboard [--kind {codebase,domain,knowledge}] \
                         [--port <N>] [--host <ip>] \
                         [--open | --no-open]
```

## Флаги

| Флаг             | По умолчанию                  | Заметки                                                                          |
|------------------|-------------------------------|----------------------------------------------------------------------------------|
| `--port <N>`     | `dashboard.port` / 5173       | Порт привязки.                                                                   |
| `--host <ip>`    | `dashboard.host` / 127.0.0.1  | Адрес привязки. Используй `0.0.0.0` для доступа из LAN.                          |
| `--open`         | —                             | Принудительно открыть вкладку браузера, игнорируя YAML.                          |
| `--no-open`      | —                             | Принудительно не открывать. Взаимоисключим с `--open`.                           |
| `--kind <kind>`  | `codebase`                    | Какой граф отдавать: `codebase`, `domain`, `knowledge`.                          |

`--open` и `--no-open` вместе — ошибка. Без обоих побеждает
YAML-дефолт (`dashboard.auto_open`, по умолчанию `true`).

## Мульти-граф

Один и тот же бинарник держит до трёх независимых графов рядом:

- **`codebase`** — дефолт, пишется через `understandable analyze`.
  Файлы, символы, вызовы, импорты.
- **`domain`** — пишется через `understandable domain`. Domain / flow
  / step-субстрат, выведенный из codebase-графа.
- **`knowledge`** — пишется через `understandable knowledge <wiki>`.
  Граф статей/тем в стиле Karpathy-вики.

Каждый живёт в своём архиве (`graph.tar.zst`,
`graph.domain.tar.zst`, `graph.knowledge.tar.zst`). Запускай несколько
панелей на разных портах для бок-о-бок просмотра:

```bash
understandable dashboard --kind codebase --port 5173 &
understandable dashboard --kind domain   --port 5174 &
```

Если выбрать kind, чьего архива ещё нет, бинарник падает с точным
указанием, какую подкоманду запустить первой.

## API-эндпоинты

axum-сервер отдаёт JSON API на том же порту. React-приложение его
потребляет; можно дёргать напрямую `curl`-ом или построить свой
клиент. Полный список эндпоинтов и формат хранения — в
[Архитектуре](../architecture).

## По умолчанию — только loopback

Сервер привязывается к `127.0.0.1`, если не переопределить. В сеть
ничего не торчит без явного opt-in.

Чтобы выставить в LAN (например, чтобы коллега посмотрел твой
локальный граф):

```bash
understandable dashboard --host 0.0.0.0 --no-open
```

:::caution
В панели нет аутентификации. Не привязывай `0.0.0.0` в недоверенной
сети. Для реальных мульти-юзер-деплоев терминируй TLS + аутентификацию
на reverse proxy, а саму панель оставь на loopback.
:::

## Примеры

### По умолчанию

```bash
understandable dashboard
# → http://127.0.0.1:5173/
```

### Свой порт, без открытия браузера

```bash
understandable dashboard --port 8080 --no-open
```

### Domain-граф рядом с codebase-графом

```bash
understandable domain
understandable dashboard --kind domain --port 5174
```

### Доступно по LAN

```bash
understandable dashboard --host 0.0.0.0 --port 5173 --no-open
```

## См. также

- [`analyze`](./analyze) — заполняет codebase-граф.
- [`embed`](./embed) — семантический поиск, который панель использует.
- [Архитектура](../architecture) — эндпоинты и формат хранения.
