// Kamerton — микшер громкости в трее. Без ключей — значок в трее и выпадающее окно; с ключами —
// консольные режимы проверки ядра звука (`--list`, `--watch`).

// В release-сборке своей консоли нет (программа живёт в трее); консольные ключи подключаются к консоли
// родителя (`sys::attach_console`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod cli;
mod config;
mod contrast;
mod d3d;
mod flyout;
mod hotkeys;
mod icons;
mod mark;
mod mark_render;
mod placement;
mod profiles;
mod settings_search;
mod shell;
mod snap;
mod sys;
mod texts;
mod tray;
mod update;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use audio::{Audio, Snapshot, Target};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::core::BOOL;

const HELP: &str = "Kamerton - tray volume mixer for Windows

Usage:
  kamerton                   run in the tray (one instance)
  kamerton --flyout [l,t,r,b] [--passive]
                             open the mixer window at the tray icon rect (the tray does this on click;
                             --passive: do not take focus, for checks)
  kamerton --list            print devices and apps, then exit
  kamerton --watch [--peaks] [--for <s>]
                             print audio events until Ctrl+C or for <s> seconds
                             (--peaks: levels every 500 ms)
  kamerton --set <target> <volume>
                             set volume: 0-100, 45%, +5 or -5 (relative to the current one)
  kamerton --mute <target> [on|off|toggle]
                             mute, unmute or toggle (toggle is the default)
  kamerton --get <target>    print volume and mute state
  kamerton --help | --version

<target> is: default (default output device), input (default input device), an app name
(as shown in the window, or its exe name without .exe) or a device name. Case does not matter;
an exact name wins over a part of a name; if several apps or devices fit, nothing is changed.
Exit codes: 0 done, 1 audio error, 2 bad arguments, 3 target not found or ambiguous.

--list, --watch and --get only read; --set and --mute change volume and mute, never devices.";

static STOP: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn on_ctrl(_: u32) -> BOOL {
    STOP.store(true, Ordering::SeqCst);
    true.into()
}

fn print_snapshot(s: &Snapshot) {
    for d in &s.devices {
        println!("{d}");
        for a in &d.apps {
            println!("    {a}");
        }
    }
}

fn watch(peaks: bool, limit: Option<Duration>) -> Result<(), audio::AudioError> {
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(on_ctrl), true);
    }
    let start = Instant::now();
    let (audio, snapshot) = Audio::start(|| {})?;
    let mut names = snapshot.clone();
    print_snapshot(&snapshot);
    println!("watching, Ctrl+C to stop");
    let mut last_peaks = Instant::now();
    while !STOP.load(Ordering::SeqCst) && limit.is_none_or(|l| start.elapsed() < l) {
        if let Ok(e) = audio.events().recv_timeout(Duration::from_millis(100)) {
            names.apply(&e);
            println!("{:>9.3}s  {e}", start.elapsed().as_secs_f32());
        }
        if peaks && last_peaks.elapsed() >= Duration::from_millis(500) {
            last_peaks = Instant::now();
            let line: Vec<String> = audio
                .peaks()
                .into_iter()
                .filter(|(_, p)| *p > 0.0)
                .filter_map(|(t, p)| match t {
                    Target::App(k) => {
                        let d = names.device(&k.device)?;
                        let a = d.apps.iter().find(|a| a.key == k)?;
                        Some(format!("{} {:.2}", a.name, p))
                    }
                    Target::Device(_) => None,
                })
                .collect();
            if !line.is_empty() {
                println!("{:>9.3}s  peaks  {}", start.elapsed().as_secs_f32(), line.join(", "));
            }
        }
    }
    drop(audio);
    println!("stopped, subscriptions left: {}", audio::live_subscriptions());
    Ok(())
}

fn percent(v: f32) -> u32 {
    (v * 100.0).round() as u32
}

/// `--set`, `--mute`, `--get`: своё ядро звука на время одной команды; трей узнаёт об изменении от Windows.
fn run_command(cmd: cli::Command) -> Result<(), cli::CliError> {
    use cli::{CliError, Command, EXIT_AUDIO, EXIT_TARGET};
    let fail = |e: audio::AudioError| CliError {
        code: if e == audio::AudioError::NotFound { EXIT_TARGET } else { EXIT_AUDIO },
        message: e.to_string(),
    };
    let (audio, snapshot) = Audio::start(|| {}).map_err(fail)?;
    let control = audio.control();
    match cmd {
        Command::Get(t) => {
            let f = cli::find(&snapshot, &t)?;
            println!("{}: volume {}%, {}", f.label, percent(f.volume()), if f.muted() { "muted" } else { "not muted" });
        }
        Command::Set(t, level) => {
            let f = cli::find(&snapshot, &t)?;
            let plan = cli::volumes(&f, level);
            for (target, v) in &plan {
                control.set_volume(target.clone(), *v).map_err(fail)?;
            }
            let top = plan.iter().map(|p| p.1).fold(0.0, f32::max);
            println!("{}: volume {}%", f.label, percent(top));
        }
        Command::Mute(t, mode) => {
            let f = cli::find(&snapshot, &t)?;
            let muted = cli::muted_after(&f, mode);
            for (target, _, _) in &f.items {
                control.set_muted(target.clone(), muted).map_err(fail)?;
            }
            println!("{}: {}", f.label, if muted { "muted" } else { "not muted" });
        }
    }
    Ok(())
}

/// Строка журнала падений.
fn crash_line(unix: u64, version: &str, place: &str, message: &str) -> String {
    format!("{unix} v{version} {place}: {}\n", message.replace(['\r', '\n'], " "))
}

/// Паника — строка в `crash.log` рядом с настройками, потом обычный вывод Rust.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if let Some(dir) = config::data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
                let _ = f.write_all(crash_line(unix, env!("CARGO_PKG_VERSION"), &place, &message).as_bytes());
            }
        }
        default(info);
    }));
}

/// Трей: значок и ядро звука, без окна и OpenGL. Второй экземпляр молча выходит.
fn run_tray() -> Result<(), String> {
    // После «Перезапустить» старый процесс ещё выходит — подождать, пока он отпустит «один экземпляр».
    let patience = if std::env::var_os(update::RESTARTED).is_some() { 50 } else { 0 };
    let mut instance = sys::single_instance();
    for _ in 0..patience {
        if instance.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
        instance = sys::single_instance();
    }
    let Some(_instance) = instance else {
        return Ok(());
    };
    texts::init();
    // Окно микшера живёт в этом же процессе с самого старта (спрятано под плащом): по щелчку оно
    // только встаёт на место и проявляется.
    shell::run(
        shell::Config::popup("Kamerton", [flyout::WIDTH, 200.0]),
        Box::new(|ctx, hwnd| {
            let wake = ctx.clone();
            let (audio, snapshot) = Audio::start_quiet(move || {
                tray::refresh();
                wake.request_repaint();
            })
            .map_err(|e| e.to_string())?;
            let shared = std::sync::Arc::new(flyout::Shared::new(ctx.clone(), hwnd));
            let repaint = ctx.clone();
            let trace = std::env::var_os("KAMERTON_TRACE").is_some();
            update::init(move || {
                if trace && let Some(u) = update::get() {
                    eprintln!("trace: update {:?}", u.state());
                }
                repaint.request_repaint();
            });
            tray::start(audio.control(), shared.clone());
            Ok(Box::new(flyout::Flyout::in_tray(ctx, hwnd, audio, &snapshot, shared)) as Box<dyn shell::Content>)
        }),
    )
}

/// Окно микшера Kamerton: обычное окно со всеми устройствами. Второе — выводит первое наверх.
fn run_mixer() -> Result<(), String> {
    texts::init();
    let settings = config::Settings::load();
    texts::apply(settings.lang);
    let title = texts::T::MixerApp.get();
    let snapping = std::env::var_os("KAMERTON_SNAP").is_some();
    let Some(_instance) = sys::single_mixer() else {
        sys::raise_window(title);
        return Ok(());
    };
    shell::run(
        shell::Config {
            title,
            size: [960.0, 560.0],
            decorations: true,
            // Размер — по числу устройств и программ (`Flyout::fit_mixer`).
            resizable: false,
            always_on_top: false,
            skip_taskbar: false,
            // Снимок для проверки вида — за экранами и без фокуса, не мешая пользователю.
            position: snapping.then_some([-4000, -4000]),
            active: !snapping,
        },
        Box::new(|ctx, hwnd| flyout::Flyout::mixer_window(ctx, hwnd).map(|f| Box::new(f) as Box<dyn shell::Content>)),
    )
}

/// Окно микшера отдельным процессом — для проверки вида (`--flyout --passive`).
/// `anchor` — «left,top,right,bottom» значка в трее; без него — угол основного монитора.
fn run_flyout(anchor: Option<&str>, passive: bool) -> Result<(), String> {
    texts::init();
    let anchor = anchor.and_then(|a| {
        let v: Vec<i32> = a.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (v.len() == 4).then(|| placement::Rect { left: v[0], top: v[1], right: v[2], bottom: v[3] })
    });
    shell::run(
        shell::Config::popup("Kamerton", [flyout::WIDTH, 200.0]),
        Box::new(move |ctx, hwnd| {
            // Снимок страницы настроек показывает и блок обновлений (без сети: проверка только по кнопке).
            if std::env::var_os("KAMERTON_SNAP").is_some() {
                update::init(|| {});
            }
            flyout::Flyout::standalone(ctx, hwnd, anchor, passive).map(|f| Box::new(f) as Box<dyn shell::Content>)
        }),
    )
}

fn main() {
    install_crash_log();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        if let Err(e) = run_tray() {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    sys::attach_console();
    let has = |a: &str| args.iter().any(|x| x == a);
    if has("--mixer") {
        if let Err(e) = run_mixer() {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    if has("--flyout") {
        let anchor =
            args.iter().position(|x| x == "--flyout").and_then(|i| args.get(i + 1)).filter(|a| !a.starts_with("--"));
        if let Err(e) = run_flyout(anchor.map(|s| s.as_str()), has("--passive")) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(parsed) = cli::parse(&args) {
        match parsed.and_then(run_command) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("error: {}", e.message);
                std::process::exit(e.code);
            }
        }
    }
    let result = if has("--version") {
        println!("Kamerton {}", env!("CARGO_PKG_VERSION"));
        Ok(())
    } else if has("--list") {
        Audio::start(|| {}).map(|(_, s)| print_snapshot(&s))
    } else if has("--watch") {
        match args.iter().position(|x| x == "--for").map(|i| args.get(i + 1).and_then(|v| v.parse::<u64>().ok())) {
            Some(None) => {
                eprintln!("error: --for needs a number of seconds");
                std::process::exit(2);
            }
            limit => watch(has("--peaks"), limit.flatten().map(Duration::from_secs)),
        }
    } else {
        println!("{HELP}");
        Ok(())
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn crash_line_is_one_line() {
        let l = super::crash_line(1_700_000_000, "0.7.0", "src/x.rs:10", "boom\nsecond");
        assert_eq!(l, "1700000000 v0.7.0 src/x.rs:10: boom second\n");
    }
}
