# EMLBox v0.1 — single-file .eml container

Один `.eml`-файл = всё приложение/сущность: GUI, логика, KV-данные, бинарь,
плюс append-only дельта-хвост для состояния и памяти. Чтение — index-driven,
через mmap: никогда не сканируется всё тело файла.

## Layout

```
[0 .. H)      envelope headers (RFC 822), конец = первая пустая строка
[H .. I)      base multipart: секции данных + последняя секция = head index
[I .. TI)     дельта-блоки (MIME-сообщения), отсутствуют при создании
[TI .. T)     tail index JSON (переписывается при каждом append)
[T .. EOF)    trailer, фиксированные 512 байт, всегда в конце файла
```

## Инварианты

1. **Стабильные оффсеты.** Все смещения — абсолютные байтовые позиции от начала
   файла. Append никогда не трогает base-префикс: существующие оффсеты вечны.
2. **Контиг-дельты.** Дельта-блоки лежат подряд: блок N+1 пишется сразу после
   блока N, tail index переписывается в конце файла. Никаких дыр/мёртвых tail
   (первый дельта-блок перезаписывает пустой tail index, созданный при build).
3. **Чтение без скана.** Envelope → head index → tail index (два маленьких JSON)
   дают полную карту секций. Доступ к payload = zero-copy слайс mmap.
   Мультипарт-границы декоративны и нужны только для интероп; внутреннее чтение
   идёт по индексу — коллизии boundary не страшны.
3. **Hash-chain.** `X-Base-Hash` = sha256(base-префикс). Каждый дельта-блок
   несёт `X-Prev-Hash` (для seq=1 — base hash). Trailer хранит хэш последнего
   блока. `emlbox verify` пересчитывает всё и ловит любое вмешательство.

## Envelope (head)

```
From: <entity@system.local>
To: <kernel@system.local>
Subject: <subject>
X-Entity-ID: <entity>
X-EML-Type: Application/Unified
X-EML-Version: 0.1.0
X-Index-Offset: <20-значное число>
X-Index-Length: <20-значное число>
Content-Type: multipart/mixed; boundary="EMLBOX_v1_<hex>"
```

`X-Index-Offset/Length` указывают на payload head index — так ридер находит карту
секций без скана. Поля зафиксированы шириной 20 символов, поэтому длина envelope
детерминирована до вычисления оффсетов (single-pass сборка).

## Base-секции (multipart)

```
--BOUNDARY
Content-Type: text/html; name="view.html"
Content-ID: <view>
X-Encoding: raw

<payload>
```

`X-Encoding: raw` — payload байт-в-байт (self-hosted, без base64).
Для интероп-секций — будущий `X-Encoding: base64`.

## Head index (последняя base-секция)

```json
{"v":1,"sections":[{"id":"view","ct":"text/html","name":"view.html","off":412,"len":1044,"enc":"raw"}]}
```

## Дельта-блок (append)

```
X-EMLBox-Delta: v1
X-Entity-ID: <entity>
X-Delta-Seq: N
X-Prev-Hash: <sha256 предыдущего блока | base hash для seq=1>
Content-Type: application/x-emlbox-delta+json

{"op":"set","table":"users","key":"player_1","value":{"hp":80},"ts":1755000000}
```

Append = запись блока перед tail index + перезапись tail index + trailer.
O(размер tail index), не O(файла). EOL блока может быть LF (lenient-чтение),
оффсеты — байты, они от стиля EOL не зависят.

## Trailer (последние 512 байт)

```
X-EMLBox-Trailer: v1
X-Entity-ID: <entity>
X-Tail-Seq: N
X-Tail-Hash: <sha256 последнего дельта-блока>
X-Base-Hash: <sha256 base-префикса>
X-Tail-Index-Offset: <u64>
X-Tail-Index-Length: <u64>
<padding пробелами до 512>
```

## KV поверх контейнера

KV-таблица = JSON-секция (например `users`). Чтение: base JSON + реплей дельт
по seq. Запись: append `{"op":"set|del","table":...,"key":...,"value":...}`.
Компактизация (слияние дельт в base) — TODO: пересборка файла + сброс chain.

## Известные ограничения v0.1

- Tear-write при крахе посередине append — нет журнала (TODO: write-ahead).
- Tail index растёт с числом дельт (append O(tail), не строго O(1)).
- Текстовое редактирование файла (смена EOL и т.п.) инвалидирует оффсеты —
  нужен `reindex` (TODO).
- base64-секции не реализованы (только raw).

## EML-IPC (шина событий)

Сообщение — это .eml в spool-директории (локальная шина; транспорт заменяем
на SMTP-меш/QUIC без смены формата):

```
From: <view@system.local>
To: <game_arcade_v1@system.local>
X-Event: MOVE
X-EMLBox-Msg: v1
Content-Type: application/json

{"dx":5,"dy":-3}
```

Диспетчеризация: `To:` == `X-Entity-ID` контейнера-получателя. Обработанные
сообщения переименовываются в `.done`; не-адресованные остаются в шине для
своего владельца. `From/To` парсятся без угловых скобок.

## EML-Runner

Исполняет `logic`-секцию контейнера на событиях шины:
`on_event(hal, state, event) -> [{"op":"set|del","table","key","value"}, ...]`.

* state = merged KV-таблица `state` контейнера (перечитывается после каждого события)
* возвращённые ops применяются как дельта-блоки (append, hash-chain цела)
* `hal.emit(msg)` пишет исходящее .eml в шину (межмодульная связь)
* исполнение: python3 + генерируемый harness; НЕТ песочницы (TODO: seccomp/nsjail) — только доверенные контейнеры

## EML-FS (плоский стор + X-Query)

Диск = директория с .eml-контейнерами (плоско, без иерархии). `fs index`
открывает каждый контейнер (envelope+index+tail) и строит запись:
entity / X-EML-Type / Subject / теги / размер. Теги: статичные X-Tag-заголовки
envelope + динамические из KV-таблицы `tags` (дельта-append). Не-контейнерные
.eml пропускаются.

Виртуальная директория — контейнер с X-EML-Type: System/Directory; членство:
`X-Contains-ID: <entity>` (явное) и/или `X-Query: ...` (динамическое), объединение
без дублей.

X-Query v0.1: `поле OP значение [AND поле OP значение ...]`
  поле: X-Entity-ID | X-EML-Type | X-Tag | Subject (подстрока, CI)
  OP: == | !=  (TODO: OR, NOT, вложенность)

## Модуль 3: Database/KV (`emlbox mkdb`)

Контейнер с `X-EML-Type: Database/KV`. Таблицы = MIME-секции (users: JSON,
items: raw-бинарь); byte-offset map = head index (`mount` показывает off/len
каждой секции — ядро читает запись mmap-слайсом, без скана файла).

## Модуль 4: AI/MemoryBank (`emlbox mkmem`)

Контейнер с `X-EML-Type: AI/MemoryBank`. Вложения = LTM (long_term_facts.md)
+ эмбеддинги (vectors.idx, raw). Turn'ы — append-дельты в KV-таблицу `turns`
(X-Turn-ID = ключ). Base не пересобирается.

## eml-tag: плоская теговая БД (`emlbox tagdb`)

Запись = атомарный .eml-файл (X-Record-ID/X-Tag*/X-Device-ID/X-Timestamp +
JSON-тело), вставка через tmp+rename (не бывает torn-записи).

* **Header-only scan**: чтение первых N байт до пустой строки (лимит 4096),
  тело никогда не читается. Сломанный файл = 1 пропущенная запись, остальные
  живы (corruption-proof).
* **X-Query v0.2** (общий движок `query.rs`, работает и для контейнеров, и для
  tagdb): `поле OP значение [AND ...]`, OP: == != >= <= > <; численное
  сравнение для X-Timestamp; множественные X-Tag: == = любой, != = ни один.
* **Бенчмарк** (n=2000, body=64 KiB, tmpfs, release): query 'X-Tag ==
  "telemetry"' — tagdb 10.0 ms / 8.2 MiB прочитано, flat-FS (grep-семантика)
  23.1 ms / 131 MiB, single-file контейнер 66.8 ms. Вставка: tagdb 43 us/rec,
  flat 36 us/rec. Правило выбора: мелкие записи + активные правки → контейнер
  (KV); крупные тела + теговый поиск → tagdb.

## Сетевая фаза: multi-writer delta-sync (v0.2)

Каждый дельта-блок несёт:

```
X-EMLBox-Delta: v1
X-Entity-ID: <entity>
X-Writer-ID: devA            <- писатель
X-Delta-Seq: N               <- номер ВНУТРИ писателя (не глобальный)
X-Prev-Hash: <пред. блок ТОГО ЖЕ писателя | base_hash для seq=1>
Content-Type: application/x-emlbox-delta+json
```

Свойства:

1. **Per-writer chains.** Цепочка каждого писателя независима и сходится на
   общем `X-Base-Hash`. `verify` проверяет каждую цепочку отдельно.
2. **Merge = дописать дословно.** Чужой блок применяется verbatim (байты не
   меняются → hash-chain писателя цел). Блоки всех писателей лежат в одном
   tail index; порядок записей = порядок поступления.
3. **Валидация при apply**: `seq` должен быть ровно следующим для писателя,
   `X-Prev-Hash` — равен последнему блоку этого писателя (или base). Блок без
   предшественника остаётся pending в шине (pull крутится циклом до стабилизации).
4. **Идемпотентность**: повторно пришедший блок (та же writer#seq) — .done, не ошибка.
5. **Replay (KV)**: внутри писателя строго по seq (причинность); между
   писателями — LWW по (ts, writer) со стабильной сортировкой. Один и тот же
   набор дельт на любых устройствах сходится к одному состоянию (детерминизм).

Старый формат (v0.1): блоки без `X-Writer-ID` читаются как writer "local",
tail entry без поля writer — тоже "local" (serde default). Обратная совместимость
полная, старые контейнеры верифицируются без замечаний.

Транспорт — та же .eml-шина (spool-директория), что и EML-IPC: локальный и
сетевой обмен неотличимы. SMTP-меш/QUIC — замена транспорта без смены формата.
