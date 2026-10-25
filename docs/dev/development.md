# Как разрабатывать

## Сборка

Rust 2024, только Windows. Репозиторий и `target` — на диске E:.

```bash
cargo build -j 4
```

Программа — `target\debug\kamerton.exe` (с консолью для журналов) или `target\release\kamerton.exe`
(`cargo build --release -j 4`; без своей консоли, консольные ключи подключаются к консоли родителя).
Окно рисует свой цикл (`src/shell.rs`: winit + egui-winit) и свой рисовальщик egui на Direct3D 11
(`src/d3d.rs`); eframe в программе только ради типов egui для набора anvil-ui. Замер 22.10.2026, своя
память окна: D3D11 ~26 МБ, OpenGL через eframe ~110 МБ, wgpu 430–560 МБ.

## Проверки

По ходу — `cargo check -j 4`. Перед сдачей один раз:

```bash
python E:/dev_personal/tools/checks.py --repo . --logs E:/dev_personal/kamerton-scratch -j 4 fmt test clippy
```

Судить по коду выхода; журналы — в `E:\dev_personal\kamerton-scratch`. Тесты покрывают модель
(слияние сессий, разницу снимков, применение событий, шаг громкости), склейку меток, место окна
(`placement`), высоту окна, ход значков (сверка с эталоном) и их кадры (сверка с образцами RGBA), `HICON`
из кадра, шаг колеса, строки RU/EN, запись PNG. COM, трей и окно тестами не покрыты — их проверяют
консольные режимы и живая проверка.

## Переменные для проверки

| Переменная | Что делает |
|---|---|
| `KAMERTON_LANG=ru\|en` | язык вместо системного |
| `KAMERTON_THEME=light\|dark` | тема вместо системной |
| `KAMERTON_OPEN=1` | трей при запуске открывает окно у значка: без фокуса, не закрывается само |
| `KAMERTON_OPEN=2` | то же через 2 с и ещё раз через 4 с — замер открытия после прогрева |
| `KAMERTON_SNAP` с `--mixer` | окно микшера за экранами и без фокуса — снимок не мешает работе |
| `KAMERTON_SNAP=<файл.png>` | окно через 10 кадров после открытия сохраняет свой снимок и закрывается |
| `KAMERTON_EXPANDED=0\|1` | вид окна (свёрнуто / развёрнуто) для снимков; `settings.txt` не меняется |
| `KAMERTON_PAGE=hotkeys\|profiles\|settings` | окно `--flyout` сразу на странице настроек |
| `KAMERTON_UPDATE_DEMO=uptodate\|checking\|available\|downloading\|ready\|failed\|ratelimit` | шапка «Настроек» показывает выдуманное состояние обновления (снимки), GitHub не трогается |
| `KAMERTON_PAGE=osd` с `KAMERTON_OSD=<проценты>[m]` | окно `--flyout` сразу плашкой громкости (`m` — «без звука»), держится час; со `KAMERTON_SNAP` — снимок плашки |
| `KAMERTON_OSD_DEMO=1` | трей через 2 с и ещё раз через 0,8 с показывает плашку с нынешней громкостью устройства по умолчанию, ничего не меняя — живая проверка плашки |
| `KAMERTON_SEARCH=<запрос>` | с `KAMERTON_PAGE=settings` — страница «Настройки» с запросом в поиске |
| `KAMERTON_CONTRAST=black\|white` | вид при высоком контрасте на образцах цветов (без включения темы в Windows); без неё — по настройке Windows |
| `KAMERTON_CYCLE=N` | трей через 2 с открывает и закрывает окно N раз под плащом (невидимо) и печатает свою память в начале, после 20-го и последнего раза — прогон на утечки |
| `KAMERTON_SNAP` с `--flyout` | на странице настроек виден и блок обновлений (проверка — только по кнопке) |
| `KAMERTON_SHEET=<файл.png>` | `cargo test tray_sheet -- --ignored` пишет лист значков трея: знак и колонка, пурпур и лазурь, 16–32 px на тёмной и светлой панели, громкость, «без звука», ошибка, кадры движения |
| `KAMERTON_TRACE=1` | печатать место окна, прямоугольник значка, время просьбы открыть и проявления, смену состояния обновления |

Снимок окна в нужной теме и языке, не трогая курсор (окно — в углу основного монитора):

```bash
KAMERTON_THEME=dark KAMERTON_LANG=en KAMERTON_SNAP=E:/dev_personal/kamerton-scratch/snap-en-dark.png target/debug/kamerton.exe --flyout --passive
```

То же у значка в трее: запустить трей с `KAMERTON_OPEN=1` и теми же переменными.

Закрыть запущенный Kamerton как «Выход» в меню (значок убирается):

```bash
powershell -ExecutionPolicy Bypass -File scripts/quit.ps1
```

Перед проверкой в трее — убедиться, что Kamerton не запущен (`Get-Process kamerton`): второй экземпляр
сразу выходит, а `scripts/quit.ps1` закроет уже работающий — возможно, экземпляр владельца. Снимки на
выдуманных настройках — `APPDATA=E:/dev_personal/kamerton-scratch/demo-appdata` (там `Kamerton/settings.txt`
с демо-сочетаниями и профилями), чтобы не трогать настоящий `settings.txt`. Трей на демо-настройках
запускать с `demo-safe` (`remember=0`, без громкости новых программ): иначе он сам поменяет громкость
появившихся программ.

Не убивать процесс `taskkill`: значок останется «призраком» в трее до наведения мыши.

## Упаковка и выпуск

- `powershell -File scripts\package.ps1` — release-сборка и `target\package\kamerton-X.Y.Z-windows-x64.zip`
  с `SHA256SUMS`, как у выпуска; ничего не публикует. `-NoBuild` — упаковать готовый exe.
- Выпуск — тег `vX.Y.Z` на коммите с той же версией в `Cargo.toml` (только по команде владельца):
  `.github/workflows/release.yml` вызывает общий workflow Anvil, публикует архив и `SHA256SUMS`, подписывает
  exe, если в репозитории есть секреты `SIGN_PFX`/`SIGN_PASSWORD`. Заметки — из аннотации тега
  (`git tag -a vX.Y.Z --cleanup=verbatim -F notes.md`).

## Живая проверка

- `kamerton --list` — сверить список с панелью звука Windows.
- `kamerton --watch` — подвигать громкость в микшере Windows, вставить наушники: должны прийти события.
- Трей: щелчок, колесо над значком, меню, закрытие окна щелчком мимо и Esc — руками; курсор
  пользователя самому не двигать.
- Утечки: запустить `kamerton --watch --for 900`, снять память процесса (`PrivateMemorySize64`), затем
  `powershell -ExecutionPolicy Bypass -File scripts\leak-burst.ps1 -n 40` несколько раз, снимая память
  после каждого. Параметры: `-n` — сколько процессов (60), `-pause` — пауза между ними в мс (1000;
  вместе с запуском выходит около 2 с на процесс), `-dir` — папка для `silence.wav`
  (`E:\dev_personal\kamerton-scratch\leak`). Норма: после первого прогрева память стоит
  (22.10.2026: 2,18 → 2,45 → 2,39 → 2,39 МБ на 0/40/80/120 процессах). В конце `--watch` — строка
  `subscriptions left: 0`. Тест грузит ПК — только с согласия владельца.
- Консольные режимы ничего не меняют. Проверку громкости из окна, смены устройства по умолчанию и
  переноса программ вживую — только по команде владельца и с возвратом прежних значений: это его
  настройки звука. Тест `policy_reads_without_changing` только читает.

## Знак и значки

Файлы знака — `assets/icon` (что где — `assets/icon/README.md`). Поправили SVG — пересобрать PNG, `.ico` и
образцы RGBA для тестов растеризатора одной командой (нужен `resvg`):

```bash
powershell -ExecutionPolicy Bypass -File scripts/render-icon.ps1
```

Повторный запуск без правок SVG даёт те же байты. Образцы `kamerton-{16,32,64}.rgba` — SVG, отрисованный в
8 раз крупнее и усреднённый (истинное покрытие): resvg в 16–32 px сглаживает грубее, сверка с его PNG
ловила бы не наши ошибки. Значок exe ставит `build.rs` (`winresource`); без `rc.exe` сборка не падает, exe
выходит без значка. Ход значков сверяется с эталон `reference/mark-motion.html` изменения OpenSpec `stage-0-9-mark-motion` (`openspec/changes/`, после архивации — `openspec/changes/archive/`) — тесты `mark::tests`, числа — строка
внизу эталона (`?scenario=…&at=…`).

## CI на GitHub

`.github/workflows/ci.yml` — только по кнопке (Actions → CI → Run workflow), Windows, Rust той же версии, что на
ПК: `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`,
`cargo test --locked --all-features --no-fail-fast`. Правило владельца — запускать после каждой новой версии
(`CLAUDE.md`, «Сборка на GitHub»); перед кнопкой — те же команды на ПК. Состояние —
помощник `ghci.py status` (скил `github-ci`) в папке репозитория или страница Actions на GitHub.
