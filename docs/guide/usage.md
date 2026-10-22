# Как пользоваться

Окна и трея пока нет; ядро звука проверяется из консоли. Консольные режимы только читают — громкость
и устройства не меняют.

## Снимок: `kamerton --list`

Печатает устройства и под каждым — его программы, затем выходит:

```
[out] Speakers (Demo Audio) 100% default
    Web Browser 100% (pid 4100,4120)
    System sounds 100% (pid 0)
[in] Microphone (Demo Audio) 50% default
```

- `[out]` / `[in]` — вывод или ввод; `default` — устройство по умолчанию; `muted` — без звука.
- Процессы одного exe на одном устройстве — одна строка, их pid через запятую.
- Имя программы: из сессии Windows, иначе описание файла («Web Browser» вместо «browser»), иначе имя exe.

## Поток событий: `kamerton --watch [--peaks] [--for <s>]`

Печатает снимок, затем каждое изменение строкой со временем от запуска:

```
    5.023s  app added      Music Player 100% (pid 5200)
    7.410s  device changed [out] Speakers (Demo Audio) 64% default
```

- События: `device added/removed/changed`, `app added/removed/changed`.
- `--peaks` — раз в 500 мс уровни (0–1) программ, у которых сейчас есть звук.
- `--for <s>` — остановиться через `<s>` секунд; иначе — по Ctrl+C.
- В конце: `stopped, subscriptions left: 0` — сколько подписок на события Windows осталось
  неснятыми (должно быть 0).

`kamerton --help` — подсказка, `--version` — версия.
