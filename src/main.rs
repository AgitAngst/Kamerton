// Kamerton — микшер громкости в трее. Без ключей — значок в трее и выпадающее окно; с ключами —
// консольные режимы проверки ядра звука (`--list`, `--watch`).

// В release-сборке своей консоли нет (программа живёт в трее); консольные ключи подключаются к консоли
// родителя (`sys::attach_console`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod flyout;
mod icons;
mod placement;
mod snap;
mod sys;
mod texts;
mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use audio::{Audio, Snapshot, Target};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::core::BOOL;

const HELP: &str = "Kamerton - tray volume mixer for Windows

Usage:
  kamerton                   run in the tray (one instance)
  kamerton --list            print devices and apps, then exit
  kamerton --watch [--peaks] [--for <s>]
                             print audio events until Ctrl+C or for <s> seconds
                             (--peaks: levels every 500 ms)
  kamerton --help | --version

Console modes only read; they never change volume or devices.";

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

/// Трей и окно. Второй экземпляр молча выходит.
fn run_tray() -> Result<(), String> {
    let Some(_instance) = sys::single_instance() else {
        return Ok(());
    };
    texts::init();
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Kamerton")
        .with_decorations(false)
        .with_always_on_top()
        .with_taskbar(false)
        .with_resizable(false)
        .with_visible(false)
        // eframe сам показывает окно после первого кадра — пусть это будет за пределами экранов;
        // на втором кадре окно прячется (`Flyout::ui`).
        .with_position([-32000.0, -32000.0])
        .with_inner_size([flyout::WIDTH, 200.0]);
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    let result = eframe::run_native(
        "Kamerton",
        options,
        Box::new(|cc| match flyout::Flyout::new(cc) {
            Ok(app) => Ok(Box::new(app) as Box<dyn eframe::App>),
            Err(e) => Err(e.into()),
        }),
    );
    tray::stop();
    result.map_err(|e| e.to_string())
}

fn main() {
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
