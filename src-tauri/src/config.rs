//! Persistent user settings, stored as JSON next to the database.
//!
//! No secrets live here. The tracker session is owned by the worker webview's
//! own cookie jar — the app never sees a password — and only the display name
//! of the signed-in user is cached in this file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppResult;

/// RuTracker is blocked by a number of ISPs, so the host is configurable and
/// ships with the known-good mirrors.
pub const RUTRACKER_MIRRORS: &[&str] = &[
    "rutracker.org",
    "rutracker.net",
    "rutracker.nl",
    "rutracker.me",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NetworkConfig {
    /// Port the BitTorrent listener binds to. 0 = pick a free one.
    pub listen_port: u16,
    pub enable_dht: bool,
    pub enable_upnp: bool,
    /// Local Service Discovery — finds peers on the same LAN.
    pub enable_lsd: bool,
    /// KiB/s, 0 = unlimited.
    pub download_limit_kbps: u32,
    pub upload_limit_kbps: u32,
    pub max_peers_per_torrent: u32,
    /// `socks5://host:port` or `http://user:pass@host:port`, applied to tracker
    /// site requests. The BitTorrent swarm itself is not proxied.
    pub tracker_proxy: Option<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_port: 0,
            enable_dht: true,
            enable_upnp: true,
            enable_lsd: true,
            download_limit_kbps: 0,
            upload_limit_kbps: 0,
            max_peers_per_torrent: 100,
            tracker_proxy: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RutrackerConfig {
    /// Display name of the signed-in user, cached for the UI. The session
    /// itself lives in the worker webview's own cookie jar, which persists
    /// across restarts on its own.
    pub username: Option<String>,
    /// Which mirror to talk to; see [`RUTRACKER_MIRRORS`].
    pub host: String,
    /// When a tracker session was last confirmed.
    ///
    /// The session itself lives in the webview's cookie jar and survives
    /// restarts, but nothing can read it until a browser window exists. Without
    /// this the app would offer to sign in again on every launch, even though
    /// the user is still signed in.
    pub logged_in_at: Option<i64>,
}

impl Default for RutrackerConfig {
    fn default() -> Self {
        Self {
            username: None,
            host: RUTRACKER_MIRRORS[0].to_string(),
            logged_in_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdatesConfig {
    pub enabled: bool,
    /// How often the watcher polls the tracker API, in minutes.
    pub interval_minutes: u32,
    /// Check right after the app starts instead of waiting a full interval.
    pub check_on_startup: bool,
    /// Download the refreshed torrent without asking. Off by default — the
    /// user asked to be prompted before a game is replaced on disk.
    pub auto_download: bool,
    pub notify_desktop: bool,
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: 360,
            check_on_startup: true,
            auto_download: false,
            notify_desktop: true,
        }
    }
}

/// A tracker forum pinned to the home screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeaturedForum {
    pub id: i64,
    pub title: String,
}

/// The "what is new" strips on the library screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HomeConfig {
    pub enabled: bool,
    /// Forums shown as strips, in order.
    pub forums: Vec<FeaturedForum>,
    /// How many releases to show per strip.
    pub per_forum: u32,
}

impl Default for HomeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            // Sensible out-of-the-box picks, both verified to exist on the
            // live tracker; the user can change them in settings.
            forums: vec![
                FeaturedForum {
                    id: 635,
                    title: "Игры для Windows · Новинки".into(),
                },
                FeaturedForum {
                    id: 842,
                    title: "Новинки и сериалы в стадии показа".into(),
                },
            ],
            per_forum: 8,
        }
    }
}

/// Playback settings, translated into mpv options.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlayerConfig {
    /// `off` | `dynaudnorm` | `loudnorm`.
    ///
    /// Evens out the loudness gap between quiet dialogue and loud action that
    /// makes films unwatchable at night. `dynaudnorm` adapts continuously and
    /// is cheap; `loudnorm` targets broadcast loudness and is stricter.
    pub audio_normalize: String,
    /// 0–150; above 100 amplifies.
    pub volume: u32,
    /// Hardware decoding. Off is the safe choice on odd drivers.
    pub hardware_decoding: bool,
    /// Preferred subtitle and audio languages, mpv syntax, e.g. `rus,eng`.
    pub subtitle_lang: String,
    pub audio_lang: String,
    /// Show mpv's on-screen controller.
    pub on_screen_controls: bool,
    /// Extra `name=value` options passed through verbatim, for anything the
    /// settings screen does not cover.
    pub extra_options: Vec<String>,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            audio_normalize: "dynaudnorm".to_string(),
            volume: 100,
            hardware_decoding: true,
            subtitle_lang: "rus,ru,eng,en".to_string(),
            audio_lang: "rus,ru,eng,en".to_string(),
            on_screen_controls: true,
            extra_options: Vec::new(),
        }
    }
}

impl PlayerConfig {
    /// Options that have to be set before mpv initialises.
    pub fn mpv_options(&self) -> Vec<(String, String)> {
        let mut opts = vec![
            // Without an explicit video output libmpv leaves `vo` empty and
            // falls back to its render-API path: the picture may appear, but
            // the on-screen controller has no surface to draw on — which is
            // exactly what "the player has no controls" looked like.
            ("vo".into(), "gpu".into()),
            // Keyboard and mouse handling in the player window.
            ("input-default-bindings".into(), "yes".into()),
            ("input-vo-keyboard".into(), "yes".into()),
            ("osc".into(), if self.on_screen_controls { "yes" } else { "no" }.into()),
            // Leave the window up at the end of the file instead of vanishing.
            ("keep-open".into(), "yes".into()),
            (
                "hwdec".into(),
                if self.hardware_decoding { "auto-safe" } else { "no" }.into(),
            ),
            ("slang".into(), self.subtitle_lang.clone()),
            ("alang".into(), self.audio_lang.clone()),
            ("volume".into(), self.volume.clamp(0, 150).to_string()),
            // A network stream that is still downloading benefits from a cache.
            ("cache".into(), "yes".into()),
            ("demuxer-max-bytes".into(), "256MiB".into()),
            // No network timeout: a torrent read that waits is normal, and
            // treating it as a failure made playback jump to the next episode.
            // Teardown speed comes from cancelling the stream instead.
            ("network-timeout".into(), "0".into()),
            // If the connection genuinely drops, resume it rather than calling
            // the film finished.
            (
                "stream-lavf-o".into(),
                "reconnect=1,reconnect_streamed=1,reconnect_on_network_error=1,reconnect_delay_max=30"
                    .into(),
            ),
        ];
        if let Some(af) = self.audio_filter() {
            opts.push(("af".into(), af));
        }
        for extra in &self.extra_options {
            if let Some((name, value)) = extra.split_once('=') {
                opts.push((name.trim().to_string(), value.trim().to_string()));
            }
        }
        opts
    }

    /// The subset that can be changed while a film is playing.
    pub fn mpv_properties(&self) -> Vec<(String, String)> {
        vec![
            ("volume".to_string(), self.volume.clamp(0, 150).to_string()),
            (
                "af".to_string(),
                // An empty value clears the filter chain.
                self.audio_filter().unwrap_or_default(),
            ),
        ]
    }

    fn audio_filter(&self) -> Option<String> {
        match self.audio_normalize.as_str() {
            "dynaudnorm" => Some("dynaudnorm=g=5:f=250:r=0.9:p=0.5".to_string()),
            "loudnorm" => Some("loudnorm=I=-16:TP=-1.5:LRA=11".to_string()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiConfig {
    pub minimize_to_tray: bool,
    pub start_minimized: bool,
    pub autostart: bool,
    /// Grid vs list in the library view.
    pub library_view: String,
    /// `list` or `grid` for tracker search results.
    pub search_view: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            minimize_to_tray: true,
            start_minimized: false,
            autostart: false,
            library_view: "grid".to_string(),
            search_view: "list".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppConfig {
    pub download_dir: PathBuf,
    pub network: NetworkConfig,
    pub rutracker: RutrackerConfig,
    pub updates: UpdatesConfig,
    pub ui: UiConfig,
    pub home: HomeConfig,
    pub player: PlayerConfig,
    #[serde(default)]
    pub power: PowerConfig,
    #[serde(default)]
    pub seeding: SeedingConfig,
    #[serde(default)]
    pub schedule: ScheduleConfig,
    /// Ceiling on the "watch online" cache, in gigabytes. Zero means no limit.
    #[serde(default = "default_cache_limit_gb")]
    pub stream_cache_limit_gb: u32,
}

fn default_cache_limit_gb() -> u32 {
    // Enough for a film and the next episode, not enough to swallow a disk
    // while nobody is looking.
    20
}

/// Different speed limits during part of the day.
///
/// The usual reason is to get out of the way while the machine is being used
/// and let the torrents run at night.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleConfig {
    pub enabled: bool,
    /// Hour the window opens, 0–23.
    pub from_hour: u32,
    /// Hour it closes. Smaller than `from_hour` means it runs past midnight.
    pub to_hour: u32,
    /// Limits in force inside the window; zero means unlimited, as elsewhere.
    pub download_limit_kbps: u32,
    pub upload_limit_kbps: u32,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            from_hour: 9,
            to_hour: 23,
            download_limit_kbps: 0,
            upload_limit_kbps: 2048,
        }
    }
}

impl ScheduleConfig {
    /// Whether the given hour falls inside the window.
    ///
    /// A window that ends earlier than it starts runs through midnight, which
    /// is the shape most people want: quiet by day, open at night.
    pub fn covers(&self, hour: u32) -> bool {
        if !self.enabled || self.from_hour == self.to_hour {
            return false;
        }
        if self.from_hour < self.to_hour {
            hour >= self.from_hour && hour < self.to_hour
        } else {
            hour >= self.from_hour || hour < self.to_hour
        }
    }
}

/// When to stop giving a finished download back to the swarm.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeedingConfig {
    /// Stop once this much has been uploaded relative to the size. Zero means
    /// seed indefinitely, which is what a tracker would rather you did.
    pub ratio_limit: f64,
    /// Stop the moment a download completes, without giving anything back.
    #[serde(default)]
    pub stop_when_done: bool,
}

impl Default for SeedingConfig {
    fn default() -> Self {
        // Unlimited by default: stopping a distribution early is a decision
        // for the person with the ratio to protect, not a default to inherit.
        Self {
            ratio_limit: 0.0,
            stop_when_done: false,
        }
    }
}

impl SeedingConfig {
    /// Whether a finished torrent has given back enough and may be stopped.
    ///
    /// A torrent of unknown size cannot have a ratio, and a limit of zero is
    /// the way to say "keep seeding" — both mean the answer is no.
    pub fn should_stop(&self, uploaded: u64, total: u64) -> bool {
        if self.stop_when_done {
            return true;
        }
        if self.ratio_limit <= 0.0 || total == 0 {
            return false;
        }
        uploaded as f64 / total as f64 >= self.ratio_limit
    }

    /// Whether any rule is in force at all.
    pub fn is_active(&self) -> bool {
        self.stop_when_done || self.ratio_limit > 0.0
    }
}

/// Turning the computer off once there is nothing left to wait for.
///
/// Both switches are off unless asked for: nothing about a torrent client
/// should ever shut a machine down by surprise. Even switched on, the shutdown
/// is announced with a countdown that any key or click calls off.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PowerConfig {
    /// After the last episode of what is playing has finished.
    pub after_playback: bool,
    /// After every download has finished.
    pub after_downloads: bool,
    /// How long the countdown runs before it happens.
    pub delay_seconds: u32,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            after_playback: false,
            after_downloads: false,
            delay_seconds: 60,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            download_dir: default_download_dir(),
            network: NetworkConfig::default(),
            rutracker: RutrackerConfig::default(),
            updates: UpdatesConfig::default(),
            ui: UiConfig::default(),
            home: HomeConfig::default(),
            player: PlayerConfig::default(),
            power: PowerConfig::default(),
            seeding: SeedingConfig::default(),
            schedule: ScheduleConfig::default(),
            stream_cache_limit_gb: default_cache_limit_gb(),
        }
    }
}

impl AppConfig {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("config is malformed ({e}), falling back to defaults");
                AppConfig::default()
            }),
            Err(_) => AppConfig::default(),
        }
    }

    pub fn save(&self, path: &Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write-then-rename so a crash mid-write cannot leave a truncated config.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap())?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::ScheduleConfig;

    fn window(from: u32, to: u32) -> ScheduleConfig {
        ScheduleConfig {
            enabled: true,
            from_hour: from,
            to_hour: to,
            ..Default::default()
        }
    }

    #[test]
    fn a_daytime_window_covers_the_hours_between() {
        let w = window(9, 23);
        assert!(!w.covers(8));
        assert!(w.covers(9));
        assert!(w.covers(22));
        // The closing hour is outside: 9–23 means "until 23:00".
        assert!(!w.covers(23));
    }

    #[test]
    fn a_window_that_ends_earlier_runs_through_midnight() {
        // The shape most people want: quiet by day, open overnight.
        let w = window(23, 7);
        assert!(w.covers(23));
        assert!(w.covers(0));
        assert!(w.covers(6));
        assert!(!w.covers(7));
        assert!(!w.covers(12));
    }

    #[test]
    fn a_schedule_that_is_off_covers_nothing() {
        let mut w = window(0, 23);
        w.enabled = false;
        assert!(!w.covers(12));
    }

    #[test]
    fn an_empty_window_covers_nothing() {
        // Same hour both ends is not "all day"; it is a mistake, and treating
        // it as all day would throttle everything around the clock.
        assert!(!window(5, 5).covers(5));
    }
}

#[cfg(test)]
mod seeding_tests {
    use super::SeedingConfig;

    fn limit(ratio: f64) -> SeedingConfig {
        SeedingConfig {
            ratio_limit: ratio,
            stop_when_done: false,
        }
    }

    #[test]
    fn stopping_when_done_ignores_the_ratio_entirely() {
        // "Do not seed" means exactly that: nothing given back, whatever the
        // ratio setting happens to say.
        let cfg = SeedingConfig {
            ratio_limit: 5.0,
            stop_when_done: true,
        };
        assert!(cfg.should_stop(0, 1_000));
        assert!(cfg.is_active());
    }

    #[test]
    fn no_rules_means_nothing_is_ever_stopped() {
        assert!(!SeedingConfig::default().is_active());
    }

    #[test]
    fn seeding_is_unlimited_until_a_limit_is_set() {
        // The default must never stop a distribution: doing so silently is how
        // a tracker ratio quietly rots.
        let cfg = SeedingConfig::default();
        assert!(!cfg.should_stop(u64::MAX, 1_000));
    }

    #[test]
    fn stops_once_the_ratio_is_reached() {
        let cfg = limit(2.0);
        assert!(!cfg.should_stop(1_999, 1_000));
        assert!(cfg.should_stop(2_000, 1_000));
        assert!(cfg.should_stop(5_000, 1_000));
    }

    #[test]
    fn a_torrent_of_unknown_size_has_no_ratio() {
        assert!(!limit(1.0).should_stop(5_000, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_always_gets_an_explicit_video_output() {
        // Regression guard. With `vo` unset libmpv leaves it empty and the
        // on-screen controller has no surface: the film plays but there is no
        // pause, no seek bar and no title. Verified against the real libmpv
        // with `cargo run --example mpv_probe`.
        let opts = PlayerConfig::default().mpv_options();
        let vo = opts.iter().find(|(k, _)| k == "vo");
        assert_eq!(
            vo.map(|(_, v)| v.as_str()),
            Some("gpu"),
            "options were: {opts:?}"
        );
    }

    #[test]
    fn on_screen_controls_can_be_turned_off() {
        let mut cfg = PlayerConfig::default();
        cfg.on_screen_controls = false;
        let opts = cfg.mpv_options();
        assert_eq!(
            opts.iter().find(|(k, _)| k == "osc").map(|(_, v)| v.as_str()),
            Some("no")
        );
    }

    #[test]
    fn audio_normalisation_maps_to_a_filter() {
        let mut cfg = PlayerConfig::default();
        cfg.audio_normalize = "loudnorm".into();
        assert!(
            cfg.mpv_options()
                .iter()
                .any(|(k, v)| k == "af" && v.contains("loudnorm"))
        );

        cfg.audio_normalize = "off".into();
        assert!(!cfg.mpv_options().iter().any(|(k, _)| k == "af"));
        // Clearing it on a running player needs an empty value, not a missing one.
        assert_eq!(
            cfg.mpv_properties()
                .iter()
                .find(|(k, _)| k == "af")
                .map(|(_, v)| v.as_str()),
            Some("")
        );
    }

    #[test]
    fn extra_options_are_passed_through() {
        let mut cfg = PlayerConfig::default();
        cfg.extra_options = vec!["sub-font-size=48".into()];
        assert!(
            cfg.mpv_options()
                .iter()
                .any(|(k, v)| k == "sub-font-size" && v == "48")
        );
    }
}

fn default_download_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Downloads")
        .join("PandaTorrent")
}
