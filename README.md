# MIME-OS / EMLBox

**Всё есть `.eml`.** Операционная среда, в которой атомарный строительный блок — RFC-совместимый MIME-документ, а не `.exe`, `.elf` или закрытая база данных.

Один `.eml`-файл = целое приложение или сущность: код, GUI, состояние, KV-данные, бинарные ассеты и append-only лог изменений — в одном конверте, который читается через `mmap` по байт-оффсетам (без скана тела) и открывается любым текстовым редактором.

> Концепция развивается из манифеста **MIME-OS (EML-Native Architecture)**: плоский теговый диск вместо иерархии папок, IPC-шина из .eml-сообщений, единый формат для диска, памяти и сети.

---

## Что уже работает (прототип v0.2)

| Слой | Команда | Что делает |
|---|---|---|
| **Контейнер** | `create`, `mount`, `get`, `verify` | single-file .eml: mmap-ридер, two-level index, append-only дельта-лог с hash-chain |
| **KV** | `kv get/set/del/dump` | NoSQL-таблицы: base JSON + реплей дельт по seq |
| **Database/KV** (Модуль 3) | `mkdb` | контейнер-БД: таблицы = MIME-секции, byte-offset map |
| **AI/MemoryBank** (Модуль 4) | `mkmem` | память агента: LTM-вложения + turn'ы дельтами |
| **IPC-шина** | `ipc send/list` | события = .eml-сообщения в spool-директории |
| **Runner** | `run` | исполняет `logic`-секцию контейнера на событиях шины |
| **EML-FS** | `fs index/query/mkdir/dir/tag` | плоский диск + header-scan индекс + виртуальные директории |
| **eml-tag** | `tagdb insert/query/bench` | плоская теговая БД: header-only scan, corruption-proof |
| **Сеть (multi-writer sync)** | `sync export/push/pull/apply/heads` | delta-sync: per-writer chains, LWW merge, идемпотентная доставка, verify каждой цепочки |
| **Сеть (TCP transport)** | `sync serve/connect` | P2P delta-sync по TCP (чистый std::net): манифесты, инкрементальная передача блоков |
| **Клеточный реверс** | `rev <binary> <dir>`, `rev type/wave/cluster/graph/types/hash/diff/branch` | objdump → .eml-граф, волна типов (call-site dataflow), кластеры, diff, **ветки гипотез (In-Reply-To)** |
| **Compaction** | `compact <c> [--out new]` | слияние дельт в base: секции консолидируются, цепочки сбрасываются к точке схождения |
| **Подписи** | `EMLBOX_SEED` env | ed25519-подпись каждого дельта-блока, verify проверяет аутентичность |
| **Compaction** | `compact <c>` | слияние дельт в base (точка схождения после синков) |
| **Repair** | `repair <c>` | восстановление после tear-write: пересборка tail сканом блоков |
| **RGA-списки** | `kv add/list` | конфликт-free списки (лог ходов, комментарии): вставка по id, реплики сходятся |
| **X-Encoding** | `create --enc aes\|deflate` | секции и дельты: deflate-сжатие, aes-256-gcm шифрование (ключ EMLBOX_KEY/EMLBOX_PASS) |
| **SMTP-мост** | `mail pack/apply/receive` | контейнеры путешествуют как настоящие письма: MIME + Maildir (Thunderbird) |
| **Сайт-генератор** | `site new`, `site <posts> <out>` | посты = .eml-контейнеры → статический сайт (mini-markdown) |
| **GUI** | `emlbox-gui [file]` (feature `gui`) | egui-просмотрщик: секции, дельты по писателям, KV, verify |

46 тестов зелёные, спецификация: [`docs/FORMAT.md`](docs/FORMAT.md).

---

## Архитектура

```
┌──────────────┐  emlp-event   ┌──────────────┐
│ view (html)  │ ────────────► │  IPC bus     │  spool-директория .eml-сообщений
└──────────────┘   msg.eml    └──────┬───────┘
                                     │ To: == X-Entity-ID
                                     ▼
                              ┌──────────────┐   ops → дельты
                              │ EML-Runner   │ ──────────────────┐
                              │ (logic-секция)│                   ▼
                              └──────┬───────┘            ┌──────────────┐
                                     │ hal.emit()         │ game.eml     │
                                     ▼                    │ (один файл:  │
                              ┌──────────────┐            │  view+logic+ │
                              │  другие      │            │  state+kv+   │
                              │  модули      │            │  бинарь)     │
                              └──────────────┘            └──────────────┘

store/ (плоский диск)                db/ (eml-tag)
├── game.eml   [Application/Unified] ├── rec_0001.eml  X-Tag: telemetry
├── games.eml  [System/Directory]    ├── rec_0002.eml  X-Tag: status_ok
│    X-Query: X-Tag == "game"        └── ...
└── docs.eml   [System/Directory]
     X-Contains-ID: <manual@system.local>
```

### Формат контейнера (кратко)

```
[0..H)      envelope RFC 822 (From/To/X-Entity-ID/X-EML-Type/X-Index-Offset...)
[H..I)      base multipart: секции данных + head index (байт-оффсеты секций)
[I..TI)     дельта-блоки (append-only, hash-chain: X-Prev-Hash)
[TI..T)     tail index JSON (переписывается при каждом append)
[T..EOF)    trailer, фиксированные 512 байт
```

Инварианты:
1. **Стабильные оффсеты** — append никогда не трогает base-префикс; существующие секции вечны.
2. **Чтение без скана** — envelope → head index → tail index (два маленьких JSON), payload = zero-copy mmap-слайс. Mount не зависит от размера файла.
3. **Контиг-дельты** — блоки лежат подряд, tail index в конце, без мёртвых дыр.
4. **Hash-chain** — `X-Base-Hash` → `X-Prev-Hash` каждого блока → `X-Tail-Hash`; `verify` ловит любое вмешательство (tamper-тест).

---

## Сборка и быстрый старт

```bash
cargo build --release
./target/release/emlbox demo /tmp/game.eml     # однофайловое приложение (GUI+логика+KV+бинарь)
./target/release/emlbox mount /tmp/game.eml    # секции + дельты + hash-chain
./target/release/emlbox verify /tmp/game.eml   # проверка целостности
```

CLI-справка:

```
emlbox create <path> <entity> <subject> [--part id:ct:name:file ...]
emlbox mount <path>
emlbox get <path> <id> [--out file]
emlbox kv get|set|del|dump <path> <table> [key] [json]
emlbox ipc send|list <bus> [<to> <event> [json]]
emlbox run <container> [--bus <dir>] [--once]
emlbox sync export|push|pull|apply|heads <container> [--writer W] [--bus DIR] [--to ENTITY] [--since N]
emlbox sync serve <container> --addr :9001 | sync connect <container> --peer host:port
emlbox rev <binary> <dir>            # objdump -> .eml-граф функций
emlbox rev type|wave|cluster|graph|types|hash|diff <dir> [...]
emlbox mail pack|apply|receive       # SMTP-мост: письма с дельтами
emlbox site new <post.eml> --title T --tags a,b --src body.md
emlbox site <posts> <out>            # сборка статического сайта
emlbox-gui <file.eml>                # GUI (cargo build --features gui)
emlbox fs index|ls|query|mkdir|dir|tag <store> [...]
emlbox tagdb insert|query|bench <db> [...]
emlbox mkdb <path> [entity]          # X-EML-Type: Database/KV
emlbox mkmem <path> [entity]         # X-EML-Type: AI/MemoryBank
emlbox pack <dir> <out.eml> [entity] # директория -> один .eml (вместо zip/tar)
emlbox unpack <container> <out-dir>  # .eml -> файлы (защита от path traversal)
emlbox append <path> <delta-json-file>
emlbox verify <path>
emlbox demo <path> [--big]
emlbox bench <dir>
```

---

## Практические сценарии: что можно делать прямо сейчас

### 1. Однофайловое приложение (zero-install)

Игра/утилита = один `.eml`: GUI (html), логика (python), состояние (KV), спрайты (raw-бинарь). Перенос на другую машину или в мессенджер — копирование одного файла.

```bash
emlbox demo app.eml
emlbox ipc send bus app MOVE '{"dx":5,"dy":-3}'   # клик в GUI
emlbox run app.eml --bus bus --once               # логика исполняется, состояние дописывается
emlbox kv get app.eml state x                     # 47 — состояние обновлено
```

### 2. Живой AI-агент с памятью

Память агента = контейнер `AI/MemoryBank`: долгосрочные факты (вложения) + краткосрочные turn'ы (append-дельты, `X-Turn-ID` = ключ). Каждая реплика — дельта в конец файла, база не пересобирается. Готовый фундамент для агентов с персистентной памятью.

```bash
emlbox mkmem memory.eml
emlbox kv set memory.eml turns 90412 '{"role":"user","intent":"full_eml_architecture"}'
emlbox kv dump memory.eml turns
```

### 3. Телеметрия / IoT / логи (eml-tag)

Плоская теговая БД без иерархии папок и без SQLite. Каждая запись — атомарный `.eml`, поиск по заголовкам **без чтения тела** (header-only scan), вставка через tmp+rename (torn-запись невозможна), сбой питания убивает максимум одну запись.

```bash
emlbox tagdb insert db '{"temp":24.5}' --id rec_1 --tag telemetry --tag status_ok --device node_01 --ts 1786528800
emlbox tagdb query db 'X-Tag == "telemetry" AND X-Device-ID == "node_01"'
emlbox tagdb query db 'X-Timestamp >= 1786528800 AND X-Timestamp < 1786528900'
```

### 4. Виртуальные файловые системы по тегам (EML-FS)

Нет иерархии папок — есть плоский диск и директории-запросы. «Папка» — это `.eml` с `X-Query` или явным списком `X-Contains-ID`. Один и тот же файл может входить в сколько угодно «папок» без копирования.

```bash
emlbox fs tag store game game                     # тег = дельта, base не трогается
emlbox fs mkdir store games --query 'X-Tag == "game"'
emlbox fs mkdir store docs --contains manual@system.local
emlbox fs dir store games                          # → динамический список
```

### 5. Универсальный переносимый контейнер (вместо zip/apk/vpk)

Любой артефакт — проект, бэкап, конфигурация сервиса — упаковывается в один самодокументируемый `.eml` с типом, тегами, кодом и данными. Читается через 100 лет (MIME — международный стандарт), открывается текстовым редактором, верифицируется hash-chain.

```bash
emlbox pack ./project app.eml my_project   # вся папка -> один .eml (структура в путях секций)
emlbox unpack app.eml ./restored          # обратно, байт-в-байт
emlbox verify app.eml                     # целостность
```

`unpack` отвергает абсолютные пути и `..` (защита от path traversal) — контейнеры безопасно получать из сети.

### 6. IPC-шина как очередь задач между скриптами/сервисами

Локальный и сетевой IPC — один и тот же формат (позже транспорт заменится на SMTP/QUIC без смены конверта). Сейчас: spool-директория, диспетчеризация по `To:`, не-адресованные сообщения остаются владельцу.

```bash
emlbox ipc send /tmp/bus worker PROCESS '{"job":"render"}'
emlbox ipc list /tmp/bus
```

### 7. Аудит-лог с криптографической цепочкой

Любой контейнер — это append-only лог с sha256-связкой блоков (`X-Prev-Hash`). `verify` пересчитывает цепочку и ловит любое вмешательство. Готово для журналов, у которых важно доказать неизменность.

### 8. Zero-API обмен данными

Формат один для диска, памяти и «сети»: данные не меняют форму (SQL-строки → объекты → JSON → бинарь). Обмен = обмен `.eml`. Сервер — просто слушатель конвертов, ответ — конверт с дописанными секциями.

### 9. Состояние для ботов / симуляций / kaggle-агентов

Вся сущность в одном файле: конфиг, состояние, история ходов (дельты), highscore. Синк между устройствами — дописать чужие дельты в конец (механика готова, транспорт — в roadmap).

### 10. Читаемость и интероп

Любой модуль, кроме сырых бинарных ассетов, открывается `cat`/блокнотом/редактором 1995 года. Структура видна: заголовки → секции → дельта-лог.

---

## Бенчмарки (release, tmpfs, rustc 1.90)

### Контейнер: mount / random access / append

| Метрика | Значение |
|---|---|
| Mount (2000 файлов) | 13.9 µs/файл |
| Mount 20 КБ vs 4.1 МБ | 0.71 ms vs 0.81 ms — **flat**, O(индекс), не O(файл) |
| Random-access секции (zero-copy) | 6.2 ns/fetch |
| Append дельты (с fsync) | 75–149 µs (avg 95 µs) |
| 200 дельт | файл компактный (82.7 КБ) — без мёртвых дыр |

### eml-tag vs обычный FS vs single-file контейнер (n=2000, тело 64 КБ)

| Паттерн | insert | query `X-Tag=="telemetry"` | прочитано с диска |
|---|---|---|---|
| **eml-tag** (header-scan) | 43 µs/запись | **10.0 ms** | **8.2 MiB** |
| flat-FS (grep-семантика) | 36 µs/запись | 23.1 ms | 131 MiB |
| single-file контейнер | — | 66.8 ms | парс 128 МБ JSON |

Вывод: header-scan в 2.3× быстрее flat и в 6.7× быстрее контейнера, читая в 16× меньше данных. При крошечных телах (≤2 КБ) flat ≈ tagdb — сканирование выигрывает, когда тело ≫ заголовков.

### Правило выбора паттерна

| Ситуация | Паттерн |
|---|---|
| Мало/средне записей, активные правки, всё в одном файле | **контейнер (KV)** |
| Крупные тела, теговый поиск, телеметрия, много записей | **eml-tag** |
| Простые файлы без индекса | обычный FS |

---

## X-Query

Один язык запросов для контейнеров, FS и tagdb:

```
поле OP значение [AND поле OP значение ...]
OP: ==  !=  >=  <=  >  <
поле: имя заголовка (X-Tag, X-Device-ID, X-Timestamp, X-EML-Type, ...)
```

- `X-Timestamp >= 1000 AND X-Timestamp < 2000` — численное сравнение
- `X-Tag == "telemetry"` — множественные теги: матч если **любой** равен
- `X-Tag != "telemetry"` — матч если **ни один** не равен (и если поле отсутствует)
- `Subject == "arcade"` — подстрока, регистронезависимо
- Отсутствующее поле: `==` не матчит, `!=` матчит

---

## Дорожная карта

1. ~~Сетевой delta-sync~~ ✅ **сделано (v0.2)**: per-writer chains, LWW merge, идемпотентная доставка. Транспорт — .eml-шина; SMTP-меш/QUIC — замена транспорта без смены формата.
2. **Подписи** — DKIM/ed25519 поверх hash-chain (сейчас — только integrity).
3. **Sandbox** для Runner — seccomp/nsjail (сейчас logic исполняется как есть).
4. **Compaction** — слияние дельт в base; **reindex** после текстовых правок; **WAL** от tear-write.
5. X-Query: OR/NOT, вложенность.
6. **X-Encoding** секций: deflate + aes-256-gcm (сейчас — только raw).
7. **CRDT-merge** (RGA для списков) вместо чистого LWW для ключей-коллекций.

---

## Синк между устройствами (сеть)

Каждое устройство — независимый writer со своей цепочкой дельт; все цепочки
сходятся на общем `X-Base-Hash`. Merge — LWW по времени, детерминизм на всех
устройствах, `verify` проверяет каждую цепочку отдельно:

```bash
# устройство A
emlbox kv set game.eml state x 47 --writer devA
# устройство B
emlbox kv set game.eml state y 205 --writer devB

# B → шина → A
emlbox sync push devB.eml --writer devB --bus /dev/shm/bus --to game
emlbox sync pull devA.eml --bus /dev/shm/bus     # A получает чужие дельты

# обратно: A → B
emlbox sync push devA.eml --writer devA --bus /dev/shm/bus --to game
emlbox sync pull devB.eml --bus /dev/shm/bus

emlbox sync heads devA.eml    # все цепочки писателей
emlbox verify devA.eml        # чисто
```

Повторная доставка блоков идемпотентна (dedup по writer#seq); блоки вне порядка
остаются pending до прихода предшественника (pull крутится до стабилизации).

---

## Тесты

```bash
cargo test    # 39 тестов: инварианты формата, IPC+Runner, FS, tagdb, pack, сетевая синхронизация, X-Query
```

## Лицензия

Apache-2.0.

---

## Клеточный реверс (без Ghidra/IDA)

Бинарник → граф .eml-функций полным локальным конвейером: `objdump -d` (binutils)
→ парсинг → каждая функция = отдельный .eml со своим call-графом.

```bash
emlbox rev game.exe cells          # 13 функций -> cells/*.eml
emlbox rev graph cells             # main -> player_move, player_take_damage, ...
emlbox rev cluster cells player    # семантический поиск по имени+телу
emlbox rev type cells net_send arg1 void*       # пометить тип (такт 0)
emlbox rev wave cells net_send arg1 void* 3     # волна по References, call-site aware
emlbox rev types cells             # карта типов по всему графу
emlbox rev hash cells net_send     # дайджест нормализованного тела
emlbox rev diff cells_v1 cells_v2  # changed/added/removed между версиями
```

Клетка-функция:

```
From: <net_send@binary.target@system.local>
Subject: net_send
X-EML-Type: Reverse/Binary-Function
References: fire_bullet, player_move, player_take_damage   # кто вызывает
X-Type-arg0: Packet                                         # волновой тип
Body: listing секция (assembly)
```

Волна типов — **call-site aware**: мини-dataflow (mov/lea цепочки, слоты
пролога `[rbp-0x8]=спасённый параметр`, `[rip+..]`-адреса). Вызывающая функция
типизируется только если реально передаёт свой параметр в помеченный аргумент:
`player_take_damage` передаёт свой `Player*` в `net_send.arg1` → типизируется,
`main` передаёт локаль → честно пропущен.

Диффинг версий нормализует тело (адреса, hex-байты, `[rip+ADDR]`, комментарии)
→ ловит только реальные изменения кода.

**MCP-сервер** для LLM: `target/release/rev-mcp` (stdio JSON-RPC, 8 тулов:
get_function / get_callers / get_callees / cluster / wave / types / diff / graph).
Модель читает только соседей клетки, а не весь листинг. Подключение в
`opencode.json` как `"rev-mcp": { "command": "/path/rev-mcp" }`.

