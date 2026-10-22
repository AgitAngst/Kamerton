// Kamerton — микшер громкости в трее. Пока есть только ядро звука и консольные режимы проверки
// (`--list`, `--watch`); трей и окно — этап 0.2 в SPEC.md.

mod audio;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use audio::{Audio, Snapshot, Target};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::core::BOOL;

const HELP: &str = "Kamerton - tray volume mixer for Windows

Usage:
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |a: &str| args.iter().any(|x| x == a);
    let result = if has("--version") {
        println!("Kamerton {}", env!("CARGO_PKG_VERSION"));
        Ok(())
    } else if has("--list") {
        Audio::start(|| {}).map(|(_, s)| print_snapshot(&s))
    } else if has("--watch") {
        let limit = args
            .iter()
            .position(|x| x == "--for")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs);
        watch(has("--peaks"), limit)
    } else {
        println!("{HELP}");
        Ok(())
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
