//! Client for the open RuTracker JSON API (`api.rutracker.cc`).
//!
//! When it is up, this is the cheapest way to detect re-uploads: no login, no
//! Cloudflare challenge, and up to 100 topics per request. A release being
//! "updated" means the topic keeps its id while the attached torrent gets a new
//! info hash and a newer `reg_time`, and both fields come straight from here.
//!
//! The tracker currently answers every method with
//! `{"error":{"code":1,"text":"Temporarily disabled"}}`. That is surfaced as
//! [`AppError::ApiUnavailable`] so [`crate::updates`] can fall back to reading
//! topic pages through the browser transport instead.

use std::collections::HashMap;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};

const API_BASE: &str = "https://api.rutracker.cc/v1";
/// The API rejects longer batches.
const MAX_BATCH: usize = 100;

/// Per-topic record as returned by `get_tor_topic_data`.
///
/// Parsed leniently from JSON: the API has changed field types before, and a
/// failed update check must never take the whole poll down with it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicData {
    pub topic_id: i64,
    pub info_hash: Option<String>,
    pub forum_id: Option<i64>,
    pub topic_title: Option<String>,
    pub size_bytes: Option<i64>,
    /// Unix time the current torrent file was registered on the tracker.
    pub reg_time: Option<i64>,
    /// Tracker moderation status; 2 = approved, 0/other = not checked or dead.
    pub tor_status: Option<i64>,
    pub seeders: Option<i64>,
    pub leechers: Option<i64>,
}

pub struct RutrackerApi {
    client: Client,
    base: String,
}

impl RutrackerApi {
    pub fn new(proxy: Option<&str>) -> AppResult<Self> {
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("PandaTorrent/", env!("CARGO_PKG_VERSION")));
        if let Some(p) = proxy.filter(|p| !p.trim().is_empty()) {
            builder = builder.proxy(
                reqwest::Proxy::all(p)
                    .map_err(|e| AppError::msg(format!("Неверный адрес прокси: {e}")))?,
            );
        }
        Ok(Self {
            client: builder.build()?,
            base: API_BASE.to_string(),
        })
    }

    /// Fetches metadata for the given topics, transparently splitting the call
    /// into batches the API accepts. Topics the API does not know about are
    /// simply absent from the result.
    pub async fn get_topic_data(&self, topic_ids: &[i64]) -> AppResult<HashMap<i64, TopicData>> {
        let mut out = HashMap::with_capacity(topic_ids.len());
        for chunk in topic_ids.chunks(MAX_BATCH) {
            let vals = chunk
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let url = format!("{}/get_tor_topic_data?by=topic_id&val={}", self.base, vals);

            let resp = self.client.get(&url).send().await.map_err(|e| {
                if e.is_timeout() || e.is_connect() {
                    AppError::TrackerUnreachable(format!("api.rutracker.cc — {e}"))
                } else {
                    AppError::Network(e)
                }
            })?;

            if !resp.status().is_success() {
                return Err(AppError::TrackerUnreachable(format!(
                    "api.rutracker.cc вернул {}",
                    resp.status()
                )));
            }

            let body: Value = resp.json().await?;
            parse_topic_data_into(&body, &mut out)?;
        }
        Ok(out)
    }

    /// Resolves a topic id from an info hash — used when importing a `.torrent`
    /// the user already had, so it can join the update watcher.
    pub async fn topic_id_by_hash(&self, info_hash: &str) -> AppResult<Option<i64>> {
        let url = format!(
            "{}/get_topic_id?by=hash&val={}",
            self.base,
            info_hash.to_uppercase()
        );
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let body: Value = resp.json().await?;
        Ok(body
            .get("result")
            .and_then(Value::as_object)
            .and_then(|m| m.values().next())
            .and_then(as_i64))
    }
}

fn parse_topic_data_into(body: &Value, out: &mut HashMap<i64, TopicData>) -> AppResult<()> {
    // The tracker can switch the public API off — it answers 200 with an error
    // object. Callers treat this as "use the fallback", not as a hard failure.
    if let Some(err) = body.get("error") {
        let text = err
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("API недоступен");
        return Err(AppError::ApiUnavailable(text.to_string()));
    }

    let result = body
        .get("result")
        .and_then(Value::as_object)
        .ok_or_else(|| AppError::Parse("в ответе API нет поля result".into()))?;

    for (key, value) in result {
        let Ok(topic_id) = key.parse::<i64>() else {
            continue;
        };
        // The API uses `null` for topics that do not exist or were removed.
        let Some(obj) = value.as_object() else {
            continue;
        };
        out.insert(
            topic_id,
            TopicData {
                topic_id,
                info_hash: obj.get("info_hash").and_then(as_hash),
                forum_id: obj.get("forum_id").and_then(as_i64),
                topic_title: obj
                    .get("topic_title")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                size_bytes: obj.get("size").and_then(as_i64),
                reg_time: obj.get("reg_time").and_then(as_i64),
                tor_status: obj.get("tor_status").and_then(as_i64),
                seeders: obj.get("seeders").and_then(as_i64),
                leechers: obj.get("leechers").and_then(as_i64),
            },
        );
    }
    Ok(())
}

/// Numbers arrive as either JSON numbers or strings depending on the field.
fn as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Info hashes are compared case-insensitively everywhere in this app, so
/// normalise them to uppercase at the boundary.
fn as_hash(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim();
    if s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(s.to_uppercase())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_batch_response() {
        let body: Value = serde_json::from_str(
            r#"{
                "result": {
                    "4530000": {
                        "info_hash": "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678",
                        "forum_id": 635,
                        "size": "12884901888",
                        "reg_time": 1700000000,
                        "tor_status": 2,
                        "seeders": 42
                    },
                    "9999999": null
                }
            }"#,
        )
        .unwrap();

        let mut out = HashMap::new();
        parse_topic_data_into(&body, &mut out).unwrap();

        assert_eq!(out.len(), 1, "null topics must be skipped");
        let t = &out[&4530000];
        assert_eq!(
            t.info_hash.as_deref(),
            Some("A1B2C3D4E5F60718293A4B5C6D7E8F9012345678")
        );
        assert_eq!(t.size_bytes, Some(12_884_901_888));
        assert_eq!(t.reg_time, Some(1_700_000_000));
        assert_eq!(t.seeders, Some(42));
    }

    #[test]
    fn rejects_a_malformed_hash() {
        let body: Value =
            serde_json::from_str(r#"{"result":{"1":{"info_hash":"not-a-hash"}}}"#).unwrap();
        let mut out = HashMap::new();
        parse_topic_data_into(&body, &mut out).unwrap();
        assert!(out[&1].info_hash.is_none());
    }

    #[test]
    fn missing_result_is_an_error() {
        let body: Value = serde_json::from_str(r#"{"nothing":1}"#).unwrap();
        let mut out = HashMap::new();
        assert!(parse_topic_data_into(&body, &mut out).is_err());
    }

    #[test]
    fn a_disabled_api_is_reported_distinctly() {
        // What the tracker actually returns while the public API is off.
        let body: Value =
            serde_json::from_str(r#"{"error":{"code":1,"text":"Temporarily disabled"}}"#).unwrap();
        let mut out = HashMap::new();
        let err = parse_topic_data_into(&body, &mut out).unwrap_err();
        assert_eq!(
            err.kind(),
            "api_unavailable",
            "update checking relies on this to fall back to page parsing"
        );
    }
}
