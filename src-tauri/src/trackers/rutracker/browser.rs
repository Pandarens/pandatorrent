//! Browser-backed transport for RuTracker.
//!
//! RuTracker sits behind a Cloudflare JavaScript challenge: every endpoint that
//! matters (`login.php`, `tracker.php`, `viewtopic.php`, `dl.php`) answers a
//! plain HTTP client with `403 Just a moment...`. Only `index.php` is open, and
//! it hands out no clearance cookie, so there is no HTTP-only workaround.
//!
//! So requests go through a real browser instead. A hidden WebView2 window is
//! parked on the tracker's origin; Chromium solves the challenge during the
//! first navigation and stores `cf_clearance`, after which every same-origin
//! `fetch()` from that page carries the right cookies *and* the right TLS
//! fingerprint. Results travel back over Tauri IPC, which the worker window is
//! granted through a remote-origin capability.
//!
//! The window is created on demand and torn down once idle, so an app that is
//! only seeding does not pay for a browser it is not using.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use base64::Engine as _;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tokio::sync::oneshot;

use crate::error::{AppError, AppResult};

/// Label of the worker window. Referenced by `capabilities/tracker.json`.
pub const WORKER_LABEL: &str = "rt-worker";

/// How long to wait for the agent script to report in after a navigation.
const READY_TIMEOUT: Duration = Duration::from_secs(45);
/// How long a single `fetch()` may take. Kept short enough that a wedged
/// transport surfaces as an error the user can act on rather than a dead UI.
const JOB_TIMEOUT: Duration = Duration::from_secs(30);
/// Idle time after which the worker window is closed to release memory.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(180);
/// If the challenge is still on screen after this, it needs a human click.
const CHALLENGE_PATIENCE: Duration = Duration::from_secs(12);

/// Events the frontend listens for while the browser transport works.
pub mod events {
    /// Emitted when the user has to solve a challenge or log in by hand.
    pub const ATTENTION: &str = "tracker:attention";
    /// Emitted when login state changes.
    pub const AUTH: &str = "tracker:auth";
}

/// Snapshot the injected agent reports on every page it lands on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageState {
    pub url: String,
    pub title: String,
    /// The Cloudflare interstitial is on screen.
    pub challenged: bool,
    /// The page shows a logout link, i.e. we have a valid tracker session.
    pub logged_in: bool,
    /// The page shows the guest login block — the only trustworthy sign that
    /// there is *no* session. Most pages show neither marker.
    #[serde(default)]
    pub has_login_form: bool,
}

/// Result of one `fetch()` performed inside the worker page.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobResult {
    pub id: u64,
    #[serde(default)]
    pub status: u16,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub base64: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// Tracker pages are windows-1251.
    Cp1251,
    /// The JSON API is UTF-8.
    Utf8,
}

impl Encoding {
    fn label(self) -> &'static str {
        match self {
            Encoding::Cp1251 => "windows-1251",
            Encoding::Utf8 => "utf-8",
        }
    }
}

/// Injected into every page the worker window loads, including the Cloudflare
/// interstitial — which is how we can tell that we are stuck on one.
const AGENT_SCRIPT: &str = r#"
(() => {
  if (window.__pandaAgent) return;

  const invoke = (cmd, args) => {
    const i = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
    if (!i) return;
    try { i(cmd, args); } catch (e) { /* page may be mid-unload */ }
  };

  function pageState() {
    const title = document.title || '';
    const challenged =
      /just a moment|checking your browser|attention required/i.test(title) ||
      !!document.querySelector('#challenge-form, #challenge-running, .cf-turnstile, #cf-please-wait');
    // Mirrors auth::is_logged_in_html on the Rust side: a logout link, or the
    // guest login block being gone while a user-panel link is present.
    const hasLoginForm = !!document.querySelector('input[name="login_username"]');
    const loggedIn =
      !!document.querySelector('a[href*="logout="]') ||
      (!hasLoginForm && !!document.querySelector('a[href*="pm.php"], #logged-in-username'));
    return { url: location.href, title, challenged, loggedIn, hasLoginForm };
  }

  // Argument name must stay camelCase-of-the-Rust-parameter (`page_state`).
  function report() { invoke('tracker_page_state', { pageState: pageState() }); }

  function toBase64(buf) {
    const bytes = new Uint8Array(buf);
    let binary = '';
    const CHUNK = 0x8000; // apply() blows the stack on large arrays
    for (let i = 0; i < bytes.length; i += CHUNK) {
      binary += String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK));
    }
    return btoa(binary);
  }

  window.__pandaAgent = {
    report,
    async run(job) {
      const send = (payload) => invoke('tracker_job_result', { result: payload });
      try {
        const res = await fetch(job.url, { credentials: 'include', redirect: 'follow' });
        const buf = await res.arrayBuffer();
        if (job.binary) {
          send({ id: job.id, status: res.status, base64: toBase64(buf) });
        } else {
          let text;
          try { text = new TextDecoder(job.encoding).decode(buf); }
          catch (e) { text = new TextDecoder('utf-8').decode(buf); }
          send({ id: job.id, status: res.status, text });
        }
      } catch (e) {
        send({ id: job.id, status: 0, error: String(e) });
      }
    }
  };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', report);
  } else {
    report();
  }
  // Keeps Rust informed after the user solves a challenge or logs in.
  setInterval(report, 1500);
})();
"#;

pub struct TrackerBrowser {
    app: AppHandle,
    host: RwLock<String>,
    pending: Mutex<HashMap<u64, oneshot::Sender<JobResult>>>,
    state: RwLock<PageState>,
    /// Set while the window is deliberately on screen for the user.
    interactive: RwLock<bool>,
    last_used: Mutex<Instant>,
    next_id: AtomicU64,
    /// Serialises window creation so two concurrent requests build one window.
    creating: tokio::sync::Mutex<()>,
    /// Lets window-event callbacks, which must be `'static`, reach us back.
    self_ref: RwLock<Weak<TrackerBrowser>>,
    /// Last *known* answer to "is the user signed in".
    ///
    /// Deliberately separate from the current page: most tracker pages carry
    /// neither a logout link nor a login form, and the worker window is opened
    /// and closed on demand. Deriving the answer from whatever happens to be
    /// on screen made the UI flip between signed in and signed out.
    session: RwLock<Option<bool>>,
}

impl TrackerBrowser {
    pub fn new(app: AppHandle, host: &str) -> Arc<Self> {
        let me = Arc::new(Self {
            app,
            host: RwLock::new(host.to_string()),
            pending: Mutex::new(HashMap::new()),
            state: RwLock::new(PageState::default()),
            interactive: RwLock::new(false),
            last_used: Mutex::new(Instant::now()),
            next_id: AtomicU64::new(1),
            creating: tokio::sync::Mutex::new(()),
            self_ref: RwLock::new(Weak::new()),
            session: RwLock::new(None),
        });
        *me.self_ref.write() = Arc::downgrade(&me);
        me
    }

    fn arc(&self) -> Option<Arc<Self>> {
        self.self_ref.read().upgrade()
    }

    /// The worker window went away — closed by the user, or torn down.
    ///
    /// Dropping the pending senders makes every in-flight request fail at once
    /// instead of sitting out its timeout, which is what made the app look
    /// frozen after the login window was closed by hand.
    fn on_window_gone(&self) {
        // `session` is deliberately left alone: the webview's cookie jar
        // outlives the window, so closing it does not sign anyone out.
        let stranded = {
            let mut pending = self.pending.lock();
            let n = pending.len();
            pending.clear();
            n
        };
        if stranded > 0 {
            tracing::debug!("tracker window closed with {stranded} request(s) in flight");
        }
        *self.interactive.write() = false;
        *self.state.write() = PageState::default();
    }

    /// Value of the tracker's session cookie, straight from the webview's own
    /// jar. Independent of the injected agent, so it still answers when script
    /// injection or IPC is the thing that broke.
    pub fn session_cookie(&self) -> Option<String> {
        let window = self.window()?;
        let url = url::Url::parse(&format!("https://{}/", self.host())).ok()?;
        let cookies = window.cookies_for_url(url).ok()?;
        cookies
            .into_iter()
            .find(|c| c.name() == "bb_session")
            .map(|c| c.value().to_string())
            .filter(|v| !v.is_empty())
    }

    pub fn set_host(&self, host: &str) {
        *self.host.write() = host.to_string();
        // The parked page belongs to the old origin; drop it.
        self.close();
    }

    pub fn host(&self) -> String {
        self.host.read().clone()
    }

    fn base(&self) -> String {
        format!("https://{}/forum", self.host())
    }

    pub fn page_state(&self) -> PageState {
        self.state.read().clone()
    }

    /// Called by the `tracker_page_state` command.
    pub fn on_page_state(&self, state: PageState) {
        tracing::debug!(
            challenged = state.challenged,
            logged_in = state.logged_in,
            has_login_form = state.has_login_form,
            url = %state.url,
            "tracker page state"
        );

        // A page that shows neither marker says nothing either way, so the
        // remembered answer is kept rather than being flipped to "signed out".
        let observation = if state.logged_in {
            Some(true)
        } else if state.has_login_form {
            Some(false)
        } else {
            None
        };

        if let Some(signed_in) = observation {
            let changed = {
                let mut session = self.session.write();
                let changed = *session != Some(signed_in);
                *session = Some(signed_in);
                changed
            };
            if changed {
                let _ = self.app.emit(events::AUTH, signed_in);
            }
        }

        *self.state.write() = state;
    }

    /// The remembered answer to "is the user signed in", or `None` when the
    /// app has not seen a page that settles it yet.
    pub fn session_known(&self) -> Option<bool> {
        *self.session.read()
    }

    /// Records a definite answer, e.g. after an explicit verify or logout.
    pub fn set_session(&self, signed_in: bool) {
        let changed = {
            let mut session = self.session.write();
            let changed = *session != Some(signed_in);
            *session = Some(signed_in);
            changed
        };
        if changed {
            let _ = self.app.emit(events::AUTH, signed_in);
        }
    }

    /// Called by the `tracker_job_result` command.
    pub fn on_job_result(&self, result: JobResult) {
        if let Some(tx) = self.pending.lock().remove(&result.id) {
            let _ = tx.send(result);
        }
    }

    fn window(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window(WORKER_LABEL)
    }

    /// Ensures a worker window exists and has passed the Cloudflare challenge.
    async fn ensure(&self) -> AppResult<WebviewWindow> {
        // Only one task may build the window.
        let _guard = self.creating.lock().await;
        *self.last_used.lock() = Instant::now();

        if let Some(w) = self.window() {
            // `url` is only non-empty once the injected agent has reported, and
            // running a job before that means eval-ing into a page where
            // `__pandaAgent` does not exist yet: the call silently does nothing
            // and the caller waits out the whole job timeout. So a window that
            // has not checked in yet still goes through the wait below.
            // Scoped so the lock guard cannot be held across the await below.
            let ready = {
                let state = self.state.read();
                !state.url.is_empty() && !state.challenged
            };
            if ready {
                return Ok(w);
            }
            return self.wait_for_clearance(w).await;
        }

        // Landing straight on a challenged endpoint makes Chromium solve the
        // challenge now, so later fetches already have `cf_clearance`.
        let window = self.build_window("tracker.php", false)?;
        self.wait_for_clearance(window).await
    }

    /// Creates the worker window on the given tracker path.
    fn build_window(&self, path: &str, visible: bool) -> AppResult<WebviewWindow> {
        let url = url::Url::parse(&format!("{}/{}", self.base(), path))
            .map_err(|e| AppError::msg(format!("некорректный адрес трекера: {e}")))?;

        *self.state.write() = PageState::default();

        let window = WebviewWindowBuilder::new(&self.app, WORKER_LABEL, WebviewUrl::External(url))
            .title("RuTracker — вход")
            .inner_size(1120.0, 820.0)
            .center()
            .visible(visible)
            .initialization_script(AGENT_SCRIPT)
            .build()
            .map_err(|e| AppError::msg(format!("не удалось открыть окно браузера: {e}")))?;

        if let Some(this) = self.arc() {
            window.on_window_event(move |event| {
                if matches!(event, WindowEvent::Destroyed) {
                    this.on_window_gone();
                }
            });
        }
        Ok(window)
    }

    /// Waits for the agent to report a page that is not the interstitial.
    ///
    /// A challenge that outlasts [`CHALLENGE_PATIENCE`] is almost certainly the
    /// interactive kind, so the window is shown and the user is asked to click.
    async fn wait_for_clearance(&self, window: WebviewWindow) -> AppResult<WebviewWindow> {
        let started = Instant::now();
        let mut asked_for_help = false;

        // When the window is already on screen the user is dealing with the
        // challenge themselves, so a caller polling for the session gets a
        // quick "still checking" answer instead of a long silent block.
        let interactive = *self.interactive.read();
        let deadline = if interactive {
            Duration::from_secs(5)
        } else {
            READY_TIMEOUT
        };

        loop {
            {
                let state = self.state.read();
                if !state.url.is_empty() && !state.challenged {
                    return Ok(window);
                }
            }

            if started.elapsed() > deadline {
                return Err(AppError::TrackerUnreachable(if interactive {
                    "идёт проверка Cloudflare в окне трекера".into()
                } else {
                    "трекер не ответил или проверка Cloudflare не пройдена".to_string()
                }));
            }

            // Only worth pulling the window up if the user is not already
            // looking at it.
            if !interactive
                && !asked_for_help
                && self.state.read().challenged
                && started.elapsed() > CHALLENGE_PATIENCE
            {
                asked_for_help = true;
                let _ = window.show();
                let _ = window.set_focus();
                *self.interactive.write() = true;
                let _ = self.app.emit(
                    events::ATTENTION,
                    "Cloudflare просит подтвердить, что вы не робот — сделайте это в открывшемся окне",
                );
            }

            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Runs one `fetch()` inside the worker page.
    async fn run_job(
        &self,
        path_and_query: &str,
        binary: bool,
        encoding: Encoding,
    ) -> AppResult<JobResult> {
        let window = self.ensure().await?;
        *self.last_used.lock() = Instant::now();

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id, tx);

        let url = format!(
            "{}/{}",
            self.base(),
            path_and_query.trim_start_matches('/')
        );
        let job = serde_json::json!({
            "id": id,
            "url": url,
            "binary": binary,
            "encoding": encoding.label(),
        });

        window
            .eval(format!(
                "window.__pandaAgent && window.__pandaAgent.run({job});"
            ))
            .map_err(|e| AppError::msg(format!("не удалось выполнить запрос в браузере: {e}")))?;

        match tokio::time::timeout(JOB_TIMEOUT, rx).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => {
                self.pending.lock().remove(&id);
                Err(AppError::msg("окно браузера закрылось во время запроса"))
            }
            Err(_) => {
                self.pending.lock().remove(&id);
                Err(AppError::TrackerUnreachable(
                    "браузер не ответил на запрос вовремя".into(),
                ))
            }
        }
    }

    /// Fetches a tracker page as text.
    pub async fn get_text(&self, path_and_query: &str) -> AppResult<String> {
        let r = self.run_job(path_and_query, false, Encoding::Cp1251).await?;
        if let Some(err) = r.error {
            return Err(AppError::TrackerUnreachable(err));
        }
        let text = r
            .text
            .ok_or_else(|| AppError::Parse("пустой ответ трекера".into()))?;
        if r.status == 403 && text.contains("__cf_chl") {
            return Err(AppError::TrackerUnreachable(
                "Cloudflare снова требует проверку — откройте вход в браузере".into(),
            ));
        }
        Ok(text)
    }

    /// Fetches a UTF-8 resource, such as the JSON API.
    pub async fn get_utf8(&self, path_and_query: &str) -> AppResult<String> {
        let r = self.run_job(path_and_query, false, Encoding::Utf8).await?;
        if let Some(err) = r.error {
            return Err(AppError::TrackerUnreachable(err));
        }
        r.text
            .ok_or_else(|| AppError::Parse("пустой ответ трекера".into()))
    }

    /// Fetches a binary resource, such as a `.torrent` file.
    pub async fn get_bytes(&self, path_and_query: &str) -> AppResult<(u16, Vec<u8>)> {
        let r = self.run_job(path_and_query, true, Encoding::Utf8).await?;
        if let Some(err) = r.error {
            return Err(AppError::TrackerUnreachable(err));
        }
        let b64 = r
            .base64
            .ok_or_else(|| AppError::Parse("пустой ответ трекера".into()))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .map_err(|e| AppError::Parse(format!("повреждённый ответ браузера: {e}")))?;
        Ok((r.status, bytes))
    }

    /// Brings up the tracker's own login page for the user to sign in.
    ///
    /// Deliberately does not wait for Cloudflare first: the window is visible,
    /// so the user simply watches the challenge resolve — or clicks through it —
    /// instead of staring at a frozen dialog while a hidden window works.
    pub async fn open_login(&self) -> AppResult<()> {
        *self.interactive.write() = true;
        *self.last_used.lock() = Instant::now();

        let window = match self.window() {
            Some(w) => {
                let url = url::Url::parse(&format!("{}/login.php", self.base()))
                    .map_err(|e| AppError::msg(format!("некорректный адрес: {e}")))?;
                *self.state.write() = PageState::default();
                window_navigate(&w, url)?;
                w
            }
            None => self.build_window("login.php", true)?,
        };

        window
            .show()
            .map_err(|e| AppError::msg(format!("не удалось показать окно: {e}")))?;
        let _ = window.set_focus();
        Ok(())
    }

    /// Hides the worker window once the user is done with it.
    pub fn hide(&self) {
        *self.interactive.write() = false;
        if let Some(w) = self.window() {
            let _ = w.hide();
        }
    }

    pub fn close(&self) {
        *self.interactive.write() = false;
        self.pending.lock().clear();
        *self.state.write() = PageState::default();
        if let Some(w) = self.window() {
            let _ = w.close();
        }
    }

    /// Closes the worker window when it has been idle and is not on screen.
    pub fn close_if_idle(&self) {
        if *self.interactive.read() {
            return;
        }
        if !self.pending.lock().is_empty() {
            return;
        }
        if self.last_used.lock().elapsed() < IDLE_TIMEOUT {
            return;
        }
        if self.window().is_some() {
            tracing::debug!("closing idle tracker browser window");
            self.close();
        }
    }

    pub fn is_logged_in(&self) -> bool {
        self.session_known().unwrap_or(false)
    }
}

fn window_navigate(window: &WebviewWindow, url: url::Url) -> AppResult<()> {
    window
        .navigate(url)
        .map_err(|e| AppError::msg(format!("не удалось открыть страницу входа: {e}")))
}
