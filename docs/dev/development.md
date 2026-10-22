# Как разрабатывать

## Сборка

Rust 2024, только Windows. Репозиторий и `target` — на диске E:.

```bash
cargo build -j 4
```

Программа — `target\debug\kamerton.exe`.

## Проверки

По ходу — `cargo check -j 4`. Перед сдачей один раз:

```bash
python E:/dev_personal/tools/checks.py --repo . --logs E:/dev_personal/kamerton-scratch -j 4 fmt test clippy
```

Судить по коду выхода; журналы — в `E:\dev_personal\kamerton-scratch`. Тесты покрывают модель
(слияние сессий, разницу снимков, применение событий) и склейку меток; COM тестами не покрыт —
его проверяют консольные режимы.

## Живая проверка

- `kamerton --list` — сверить список с панелью звука Windows.
- `kamerton --watch` — подвигать громкость в микшере Windows, вставить наушники: должны прийти события.
- Утечки: `kamerton --watch --for 600`, снимать память процесса в начале и в конце; в конце строка
  `subscriptions left: 0`.
- Консольные режимы ничего не меняют. Проверку `set_volume`/`set_muted` вживую — только по команде
  владельца и с возвратом прежних значений.
