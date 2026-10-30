# Посты для запуска MIME-OS / EMLBox v0.16 (RU)

## Telegram-канал (короткий)

**MIME-OS / EMLBox v0.16 — сборка починена, документация перерисована**

Проект, где всё есть `.eml`: один RFC-совместимый файл — целое приложение
(view + logic + KV + бинарь), читается mmap-ом по байт-оффсетам, дельты
append-only с hash-chain, синк между устройствами по TCP, плоская tagdb вместо
папок, контейнеры путешествуют почтой.

В v0.16:
- починили сборку: `emlbox` (E0716) и `tests/sync.rs` снова компилируются —
  `cargo test` целиком зелёный, **69 тестов**;
- CI теперь гоняет полный сьют, а не только библиотеку;
- README на двух языках (EN+RU) в едином доме-стиле, честные LIMITATIONS,
  CHANGELOG по всем 28 тегам, CONTRIBUTING;
- промо-скриншоты — реальный вывод release-сборки, без фотошопа.

Формат не менялся ни на байт: контейнеры v0.1 открываются v0.16.
GitHub: github.com/kostyk348/mime-os

## Хабр (лид статьи)

«Один файл — и есть всё приложение» звучит как маркетинг, пока не видишь
механику: секции контейнера вечны (append никогда не трогает base-префикс),
чтение — envelope → head index → tail index, два маленьких JSON, payload —
zero-copy mmap-слайс; mount не зависит от размера файла. В v0.16 мы прибрали
инженерную часть: починили сборку бинаря и интеграционных тестов, выровняли
документацию на двух языках и честный список ограничений. В статье — бенчмарки
(header-scan tagdb против flat-FS против single-file контейнера), устройство
hash-chain и зачем multi-writer sync хранит цепочки по-писательски.

## Пост для X/Twitter

EMLBox v0.16: the single-file .eml container OS, rebuilt docs (EN+RU),
compile fixes, full suite green again (69 tests). Everything is .eml.
https://github.com/kostyk348/mime-os
