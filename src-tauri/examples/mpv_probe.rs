//! Diagnostic: what does the bundled libmpv actually support?
//!
//! The on-screen controller is a Lua script shipped inside mpv. If a build has
//! no Lua, or the script never starts, `osc=yes` is accepted and then quietly
//! does nothing — which looks exactly like "the player has no controls". This
//! tells those cases apart:
//!
//! ```text
//! cargo run --example mpv_probe
//! ```

use panda_torrent_lib::config::PlayerConfig;
use panda_torrent_lib::player::mpv::{self, Mpv};

fn main() {
    let dirs = mpv::search_dirs(None);
    println!("Searching for libmpv in:");
    for d in &dirs {
        println!("  {}", d.display());
    }

    println!("\n=== baseline: osc + gpu vo, set explicitly ===");
    probe("baseline", &[("osc", "yes"), ("vo", "gpu"), ("terminal", "no")]);

    println!("\n=== exactly what the app configures ===");
    let cfg = PlayerConfig::default();
    let opts: Vec<(String, String)> = cfg.mpv_options();
    let borrowed: Vec<(&str, &str)> = opts
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    probe("app", &borrowed);
}

fn probe(label: &str, options: &[(&str, &str)]) {
    let dirs = mpv::search_dirs(None);
    let mut player = match Mpv::load(&dirs) {
        Ok(p) => p,
        Err(e) => {
            println!("[{label}] FAILED to load libmpv: {e}");
            return;
        }
    };

    // Every player the app opens sets this, so the probe must too.
    if let Err(e) = player.set_option("force-window", "yes") {
        println!("[{label}] force-window REJECTED: {e}");
    }
    for (name, value) in options {
        if let Err(e) = player.set_option(name, value) {
            println!("[{label}] REJECTED {name}={value}: {e}");
        }
    }

    if let Err(e) = player.init() {
        println!("[{label}] FAILED to initialize: {e}");
        return;
    }

    for name in ["mpv-version", "vo", "osc", "osd-level", "input-default-bindings"] {
        println!("[{label}] {name} = {:?}", player.get_property(name));
    }

    // Decisive: this binding resolves only while osc.lua is actually running,
    // which separates "osc=yes was accepted" from "the OSC exists".
    match player.command(&["script-binding", "osc/visibility"]) {
        Ok(()) => println!("[{label}] osc script: LOADED"),
        Err(e) => println!("[{label}] osc script: NOT loaded — {e}"),
    }
}
