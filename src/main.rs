#![windows_subsystem = "windows"]

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use handy_sdk::app::args::arg_value;
use handy_sdk::app::crash_guard::on_crash;
use handy_sdk::app::self_update::{UpdateStatus, update_on_startup};
use handy_sdk::app::session_reader::SessionReader;
use handy_sdk::app::single_instance::SingleInstance;
use handy_sdk::app::status_snapshot::{snapshot_allowed, status_line, wait_for};
use handy_sdk::app::updater_thread::{Updater, production_smoke_test};
use handy_sdk::roblox::http::win_http_get;
use handy_sdk::util::paths::{app_data_dir, set_app_name};
use handy_sdk::util::thread::StopThread;
use handy_sdk::util::timing::TimerResolution;
use handys_99n::app::config::{SettingsStore, load_settings};
use handys_99n::app::dashboard::{DASHBOARD_CLASS, DashboardDeps, run_dashboard};
use handys_99n::app::offset_keys::required_keys;
use handys_99n::app::view::{float_text, minigame_text, rod_text};
use handys_99n::game::fishing::{FishingReader, FishingSample};
use handys_99n::r#macro::engine::{Engine, release_inputs};

const MUTEX: &str = "Local\\handys-99n";

fn new_updater() -> Arc<Updater> {
    Arc::new(Updater::new(
        Arc::new(win_http_get),
        production_smoke_test,
        app_data_dir(),
        &required_keys(),
    ))
}

/// `--status <file>`: read-only snapshot (no input). Waits for Ready + one fishing read, or 15 s.
fn status_snapshot(out: &Path) -> i32 {
    if !snapshot_allowed(MUTEX, "handy's 99n", out) {
        return 1;
    }
    let updater = new_updater();
    let u = updater.clone();
    let thread = StopThread::spawn(move |stop| u.run(stop));
    let mut reader = SessionReader::for_updater(&updater, |s| FishingReader::new(s.clone()));
    let mut smp: Option<FishingSample> = None;
    wait_for(
        || {
            smp = reader.get().and_then(|r| r.sample(true));
            smp.is_some()
        },
        Duration::from_secs(15),
        Duration::from_millis(50),
    );
    let line_out = match smp.and_then(|s| s.line_out) {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    };
    let mut text = format!(
        "version: {}\nstatus: {}\nfishing rod: {}\nminigame: {}\nline out: {line_out}\n",
        env!("CARGO_PKG_VERSION"),
        status_line(&updater),
        rod_text(smp.as_ref()),
        minigame_text(smp.as_ref())
    );
    if let Some(s) = smp.filter(|s| s.open) {
        text += &format!(
            "bar: {} zone: {}+{}\n",
            float_text(s.bar_mid),
            float_text(s.zone_top),
            float_text(s.zone_h)
        );
    }
    if let Ok(mut f) = std::fs::File::create(out) {
        let _ = f.write_all(text.as_bytes());
    }
    drop(reader);
    thread.request_stop();
    drop(thread);
    if smp.is_some() { 0 } else { 1 }
}

fn run() -> i32 {
    set_app_name("handy's 99n");
    on_crash(release_inputs); // a crash mid-tap must not leave a key or the mouse held down in Roblox
    if let Some(out) = arg_value("--status") {
        return if out.is_empty() {
            2
        } else {
            status_snapshot(Path::new(&out))
        };
    }

    // Only builds made by release.cmd update themselves (dev builds and build.cmd never do). Skipped while another
    // copy runs: its exe is in use and it would be replaced under it.
    let update = (option_env!("HANDY_RELEASE").is_some() && !SingleInstance::is_running(MUTEX))
        .then(|| update_on_startup("handy-z", "handys-99n", env!("CARGO_PKG_VERSION")));
    if update == Some(UpdateStatus::Restarting) {
        return 0; // the new version is starting; exit before anything else runs
    }

    let mut instance = SingleInstance::new(MUTEX, DASHBOARD_CLASS);
    if !instance.acquire() {
        return 0; // brought the running copy's window up
    }
    let _timer = TimerResolution::new(); // 1 ms engine sleeps during the minigame

    let ini = app_data_dir().join("config.ini");
    let store = Arc::new(SettingsStore::new(load_settings(&ini)));
    store.save(&ini); // first run: write the defaults so config.ini exists
    let updater = new_updater();
    let engine = Arc::new(Engine::new(updater.clone(), store.clone()));
    let u = updater.clone();
    let updater_thread = StopThread::spawn(move |stop| u.run(stop));
    let e = engine.clone();
    let engine_thread = StopThread::spawn(move |stop| e.run(stop));

    let deps = DashboardDeps {
        updater,
        engine,
        settings: store.clone(),
        config_ini: ini.clone(),
        update,
    };
    let rc = run_dashboard(&deps);

    drop(engine_thread); // stop + join
    store.save(&ini);
    // The Updater join can wait out a WinHTTP timeout (up to 10 s): let a relaunch start meanwhile.
    instance.release();
    drop(updater_thread);
    rc
}

fn main() {
    std::process::exit(run());
}
