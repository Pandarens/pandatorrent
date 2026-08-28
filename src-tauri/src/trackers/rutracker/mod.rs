//! RuTracker integration.
//!
//! Every request to the forum goes through [`browser`], because Cloudflare
//! rejects plain HTTP clients. The separate JSON API on `api.rutracker.cc` is
//! not behind the challenge, so [`api`] still uses a normal HTTP client —
//! though the tracker currently has that API switched off, which is why
//! [`crate::updates`] can fall back to reading topic pages.

pub mod api;
pub mod auth;
pub mod browser;
pub mod forums;
pub mod http;
pub mod search;
pub mod topic;

use std::sync::Arc;

use parking_lot::RwLock;
use tauri::AppHandle;

use crate::config::RutrackerConfig;
use crate::error::{AppError, AppResult};

pub use api::{RutrackerApi, TopicData};
pub use browser::{PageState, TrackerBrowser};
pub use forums::{CatalogCategory, CatalogForum, ForumEntry};
pub use search::{SearchItem, SearchPage, SearchQuery, SearchSort};
pub use topic::TopicDetails;

pub struct RutrackerClient {
    browser: Arc<TrackerBrowser>,
    api: RwLock<Arc<RutrackerApi>>,
}

impl RutrackerClient {
    pub fn new(app: AppHandle, cfg: &RutrackerConfig, proxy: Option<&str>) -> AppResult<Self> {
        Ok(Self {
            browser: TrackerBrowser::new(app, &cfg.host),
            api: RwLock::new(Arc::new(RutrackerApi::new(proxy)?)),
        })
    }

    pub fn browser(&self) -> Arc<TrackerBrowser> {
        self.browser.clone()
    }

    pub fn api(&self) -> Arc<RutrackerApi> {
        self.api.read().clone()
    }

    pub fn host(&self) -> String {
        self.browser.host()
    }

    /// Applies a mirror or proxy change. The worker window is dropped, since
    /// its parked page belongs to the previous origin.
    pub fn reconfigure(&self, cfg: &RutrackerConfig, proxy: Option<&str>) -> AppResult<()> {
        self.browser.set_host(&cfg.host);
        *self.api.write() = Arc::new(RutrackerApi::new(proxy)?);
        Ok(())
    }

    /// Opens the tracker's own login page for the user to sign in.
    pub async fn open_login(&self) -> AppResult<()> {
        self.browser.open_login().await
    }

    pub fn hide_login(&self) {
        self.browser.hide();
    }

    /// Last known login state, as reported by the worker page. Cheap.
    pub fn cached_logged_in(&self) -> bool {
        self.browser.is_logged_in()
    }

    /// Confirms the session by actually loading a forum page.
    pub async fn verify_session(&self) -> AppResult<bool> {
        let html = self.browser.get_text("index.php").await?;
        Ok(auth::is_logged_in_html(&html))
    }

    pub async fn logout(&self) -> AppResult<()> {
        // The logout link carries the session id, so it has to be read off a
        // live page rather than constructed.
        if let Ok(html) = self.browser.get_text("index.php").await {
            if let Some(path) = auth::logout_path(&html) {
                let _ = self.browser.get_text(&path).await;
            }
        }
        // Drop the window so the webview's cookie jar stops being consulted
        // for the rest of this session.
        self.browser.close();
        Ok(())
    }

    pub async fn search(&self, query: &SearchQuery) -> AppResult<SearchPage> {
        let html = self.browser.get_text(&query.to_path()).await?;
        search::parse_search_page(&html, query.page)
    }

    /// The whole forum tree. One page load covers every category, forum and
    /// subforum, and it is readable without signing in.
    pub async fn catalog(&self) -> AppResult<Vec<CatalogCategory>> {
        let html = self.browser.get_text("index.php").await?;
        forums::parse_catalog(&html)
    }

    pub async fn topic(&self, topic_id: i64) -> AppResult<TopicDetails> {
        let html = self
            .browser
            .get_text(&format!("viewtopic.php?t={topic_id}"))
            .await?;
        topic::parse_topic(&html, topic_id)
    }

    /// Downloads a topic's `.torrent`.
    ///
    /// `dl.php` answers with an HTML page rather than an error status when the
    /// session has expired, so the payload is validated as bencode here.
    pub async fn download_torrent(&self, topic_id: i64) -> AppResult<Vec<u8>> {
        let (status, bytes) = self
            .browser
            .get_bytes(&format!("dl.php?t={topic_id}"))
            .await?;

        if !(200..300).contains(&status) {
            return Err(AppError::TrackerUnreachable(format!(
                "dl.php вернул {status}"
            )));
        }
        if !is_bencoded_torrent(&bytes) {
            let text = http::decode_cp1251(&bytes);
            if search::looks_like_login_page(&text) {
                return Err(AppError::NotAuthenticated);
            }
            return Err(AppError::Parse(
                "вместо торрент-файла трекер вернул страницу — возможно, раздача удалена".into(),
            ));
        }
        Ok(bytes)
    }
}

/// A `.torrent` is a bencoded dictionary, so it starts with `d` and contains an
/// `info` dictionary.
fn is_bencoded_torrent(bytes: &[u8]) -> bool {
    bytes.first() == Some(&b'd') && bytes.windows(6).any(|w| w == b"4:info")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_bencoded_torrent() {
        assert!(is_bencoded_torrent(b"d8:announce3:foo4:infod4:name1:xee"));
    }

    #[test]
    fn rejects_an_html_page() {
        assert!(!is_bencoded_torrent(b"<!DOCTYPE html><html>"));
    }
}
