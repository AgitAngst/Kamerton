## Why

Этап «Выпуск» по SPEC.md: Kamerton должен скачиваться готовым — переносной сборкой по соглашению выпусков
семьи Anvil (`docs/RELEASES.md` в Anvil), с контрольной суммой, подписью, когда будет сертификат, и
понятным местом настроек. Без этого программой может пользоваться только тот, кто собирает её сам.

## What Changes

- Переносной режим: если рядом с exe лежит `settings.txt`, настройки и `crash.log` живут там, а не в
  `%APPDATA%\Kamerton`.
- Раздел «О программе» на странице «Настройки»: версия и пометка переносной копии.
- Workflow выпуска `.github/workflows/release.yml`: только по тегу `vX.Y.Z`, общий workflow Anvil
  (`kit-v0.6.1`) — `kamerton-X.Y.Z-windows-x64.zip`, `SHA256SUMS`, подпись exe по секретам `SIGN_PFX`/`SIGN_PASSWORD`.
- `scripts/package.ps1` — та же упаковка на ПК, без публикации.
- Версия 0.8.0. Обновления изнутри программы (`anvil-update`) — отдельным решением владельца: это новая
  зависимость с сетью.

## Capabilities

### New Capabilities

- `release`: переносной режим, «О программе», упаковка и выпуск по тегу.

### Modified Capabilities

<!-- нет -->

## Impact

- Код: `src/config.rs` (папка данных), `src/main.rs` (`crash.log`), `src/flyout.rs`, `src/texts.rs`.
- Файлы: `.github/workflows/release.yml`, `scripts/package.ps1`.
- Тег, выпуск, сертификат подписи, значок exe — владелец.
