//! Search against `tracker.php` and parsing of its result table.
//!
//! RuTracker has no search API, so this scrapes HTML. Every field is therefore
//! extracted defensively — a layout tweak on the site should degrade one
//! column, never blow up the whole result list. Where the markup offers a
//! machine-readable `data-ts_text` attribute we prefer it over display text.

use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};

use super::http::percent_encode_cp1251;
use crate::error::{AppError, AppResult};

/// Results per page, fixed by the site.
pub const PAGE_SIZE: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchSort {
    Registered,
    Title,
    Downloads,
    Size,
    Seeders,
    Leechers,
}

impl SearchSort {
    /// Values of the `o` query parameter used by `tracker.php`.
    fn code(self) -> u8 {
        match self {
            SearchSort::Registered => 1,
            SearchSort::Title => 2,
            SearchSort::Downloads => 4,
            SearchSort::Size => 7,
            SearchSort::Seeders => 10,
            SearchSort::Leechers => 11,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub text: String,
    #[serde(default)]
    pub forum_ids: Vec<i64>,
    pub sort: Option<SearchSort>,
    /// `true` sorts ascending; the site defaults to descending.
    #[serde(default)]
    pub ascending: bool,
    #[serde(default)]
    pub page: usize,
}

impl SearchQuery {
    pub fn to_path(&self) -> String {
        let mut q = format!("tracker.php?nm={}", percent_encode_cp1251(&self.text));
        for f in &self.forum_ids {
            // `f[]` percent-encoded; the site only accepts this exact spelling.
            q.push_str(&format!("&f%5B%5D={f}"));
        }
        if let Some(sort) = self.sort {
            q.push_str(&format!(
                "&o={}&s={}",
                sort.code(),
                if self.ascending { 1 } else { 2 }
            ));
        }
        if self.page > 0 {
            q.push_str(&format!("&start={}", self.page * PAGE_SIZE));
        }
        q
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchItem {
    pub topic_id: i64,
    pub title: String,
    pub forum_id: Option<i64>,
    pub forum_name: Option<String>,
    pub author: Option<String>,
    pub size_bytes: Option<i64>,
    pub seeders: i64,
    pub leechers: i64,
    pub downloads: i64,
    /// Unix time the torrent was registered on the tracker.
    pub registered_at: Option<i64>,
    /// `true` when the tracker marked the release as checked/approved.
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub items: Vec<SearchItem>,
    /// Total hits reported by the site, when it says so.
    pub total: Option<usize>,
    pub page: usize,
    pub page_size: usize,
}

struct Selectors {
    row: Selector,
    topic_link: Selector,
    forum_link: Selector,
    author: Selector,
    size_cell: Selector,
    seed: Selector,
    leech: Selector,
    downloads: Selector,
    ts_cells: Selector,
    status_icon: Selector,
}

impl Selectors {
    fn new() -> Self {
        // `Selector::parse` only fails on a malformed selector, and these are
        // all literals, so unwrapping is safe and keeps the call sites clean.
        let s = |sel: &str| Selector::parse(sel).expect("static selector");
        Self {
            row: s("table#tor-tbl tbody tr, tr.tCenter.hl-tr"),
            topic_link: s("a.tLink, a[href*=\"viewtopic.php?t=\"]"),
            forum_link: s("div.f-name a, a[href*=\"tracker.php?f=\"]"),
            author: s("div.u-name a, td.u-name-col a"),
            size_cell: s("td.tor-size"),
            seed: s("b.seedmed, td.seedmed"),
            leech: s("td.leechmed, b.leechmed"),
            downloads: s("td.number-format"),
            ts_cells: s("td[data-ts_text]"),
            status_icon: s("td.t-ico span, td.tor-icon span, div.t-icon"),
        }
    }
}

/// Parses a `tracker.php` results page.
///
/// Returns [`AppError::NotAuthenticated`] when the page is the login form,
/// which is how the tracker answers an expired session.
pub fn parse_search_page(html: &str, page: usize) -> AppResult<SearchPage> {
    if looks_like_login_page(html) {
        return Err(AppError::NotAuthenticated);
    }

    let doc = Html::parse_document(html);
    let sel = Selectors::new();

    let mut items = Vec::new();
    for row in doc.select(&sel.row) {
        if let Some(item) = parse_row(&row, &sel) {
            items.push(item);
        }
    }

    if items.is_empty() && !html.contains("tor-tbl") {
        return Err(AppError::Parse(
            "не найдена таблица результатов — вероятно, изменилась вёрстка трекера".into(),
        ));
    }

    Ok(SearchPage {
        items,
        total: parse_total(html),
        page,
        page_size: PAGE_SIZE,
    })
}

fn parse_row(row: &ElementRef<'_>, sel: &Selectors) -> Option<SearchItem> {
    let link = row.select(&sel.topic_link).next()?;
    let href = link.value().attr("href")?;
    let topic_id = extract_id(href, "viewtopic.php?t=")?;
    let title = text_of(&link);
    if title.is_empty() {
        return None;
    }

    let forum = row.select(&sel.forum_link).next();
    let forum_id = forum
        .and_then(|f| f.value().attr("href"))
        .and_then(|h| extract_id(h, "tracker.php?f="));
    let forum_name = forum.map(|f| text_of(&f)).filter(|s| !s.is_empty());

    let size_bytes = row
        .select(&sel.size_cell)
        .next()
        .and_then(|c| ts_value(&c))
        // Fall back to the human-readable label if the attribute is gone.
        .or_else(|| {
            row.select(&sel.size_cell)
                .next()
                .and_then(|c| parse_human_size(&text_of(&c)))
        });

    let seeders = row
        .select(&sel.seed)
        .next()
        .map(|c| parse_int(&text_of(&c)))
        .unwrap_or(0);
    let leechers = row
        .select(&sel.leech)
        .next()
        .map(|c| parse_int(&text_of(&c)))
        .unwrap_or(0);
    let downloads = row
        .select(&sel.downloads)
        .next()
        .map(|c| parse_int(&text_of(&c)))
        .unwrap_or(0);

    // The registration date is the last timestamped cell in the row.
    let registered_at = row
        .select(&sel.ts_cells)
        .filter_map(|c| ts_value(&c))
        // Sizes and counters also carry `data-ts_text`; a plausible unix time
        // is the only one past this threshold (2001-09-09).
        .filter(|v| *v > 1_000_000_000)
        .last();

    let approved = row
        .select(&sel.status_icon)
        .next()
        .map(|s| {
            let class = s.value().attr("class").unwrap_or_default();
            class.contains("tor-approved") || class.contains("tor-checked")
        })
        .unwrap_or(false);

    Some(SearchItem {
        topic_id,
        title,
        forum_id,
        forum_name,
        author: row
            .select(&sel.author)
            .next()
            .map(|a| text_of(&a))
            .filter(|s| !s.is_empty()),
        size_bytes,
        seeders,
        leechers,
        downloads,
        registered_at,
        approved,
    })
}

/// An expired session redirects to the login form rather than returning 401.
pub fn looks_like_login_page(html: &str) -> bool {
    html.contains("name=\"login_username\"") || html.contains("name='login_username'")
}

fn parse_total(html: &str) -> Option<usize> {
    // The site renders e.g. `Результатов поиска: 137`.
    let idx = html.find("Результатов поиска")?;
    let tail = &html[idx..];
    let digits: String = tail
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

fn extract_id(href: &str, marker: &str) -> Option<i64> {
    let pos = href.find(marker)? + marker.len();
    let digits: String = href[pos..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn ts_value(el: &ElementRef<'_>) -> Option<i64> {
    el.value().attr("data-ts_text")?.trim().parse().ok()
}

fn text_of(el: &ElementRef<'_>) -> String {
    el.text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_int(s: &str) -> i64 {
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    digits.parse().unwrap_or(0)
}

/// Parses labels like `1.15 GB` / `700 MB` as a byte count.
fn parse_human_size(s: &str) -> Option<i64> {
    let s = s.replace(',', ".");
    let num: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let value: f64 = num.parse().ok()?;
    let upper = s.to_uppercase();
    let mult = if upper.contains("TB") || upper.contains("ТБ") {
        1024f64.powi(4)
    } else if upper.contains("GB") || upper.contains("ГБ") {
        1024f64.powi(3)
    } else if upper.contains("MB") || upper.contains("МБ") {
        1024f64.powi(2)
    } else if upper.contains("KB") || upper.contains("КБ") {
        1024f64
    } else {
        1f64
    };
    Some((value * mult) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: &str = r#"
    <table id="tor-tbl"><tbody>
      <tr class="tCenter hl-tr">
        <td class="row1 t-ico"><span class="tor-icon tor-approved">*</span></td>
        <td class="row1 f-name-col"><div class="f-name">
          <a class="gen f ts-text" href="tracker.php?f=635">PC игры</a></div></td>
        <td class="row1 t-title-col tt"><div class="t-title">
          <a class="med tLink ts-text bold" href="viewtopic.php?t=6301531">Half-Life 2 [RePack]</a></div></td>
        <td class="row1 u-name-col"><div class="u-name">
          <a class="med ts-text" href="tracker.php?pid=42">uploader</a></div></td>
        <td class="row4 small nowrap tor-size" data-ts_text="12884901888">
          <a class="small tr-dl" href="dl.php?t=6301531">12 GB</a></td>
        <td class="row4 nowrap" data-ts_text="120"><b class="seedmed">120</b></td>
        <td class="row4 leechmed bold" data-ts_text="7">7</td>
        <td class="row4 small number-format" data-ts_text="4500">4500</td>
        <td class="row4 small nowrap" data-ts_text="1700000000">15-Ноя-23</td>
      </tr>
    </tbody></table>
    <p>Результатов поиска: 137</p>"#;

    #[test]
    fn parses_a_result_row() {
        let page = parse_search_page(ROW, 0).unwrap();
        assert_eq!(page.items.len(), 1);
        let it = &page.items[0];
        assert_eq!(it.topic_id, 6301531);
        assert_eq!(it.title, "Half-Life 2 [RePack]");
        assert_eq!(it.forum_id, Some(635));
        assert_eq!(it.forum_name.as_deref(), Some("PC игры"));
        assert_eq!(it.author.as_deref(), Some("uploader"));
        assert_eq!(it.size_bytes, Some(12_884_901_888));
        assert_eq!(it.seeders, 120);
        assert_eq!(it.leechers, 7);
        assert_eq!(it.downloads, 4500);
        assert_eq!(it.registered_at, Some(1_700_000_000));
        assert!(it.approved);
        assert_eq!(page.total, Some(137));
    }

    #[test]
    fn login_form_is_reported_as_unauthenticated() {
        let html = r#"<form><input name="login_username" /></form>"#;
        let err = parse_search_page(html, 0).unwrap_err();
        assert_eq!(err.kind(), "not_authenticated");
    }

    #[test]
    fn size_falls_back_to_the_label() {
        assert_eq!(parse_human_size("1.15 GB"), Some(1_234_803_097));
        assert_eq!(parse_human_size("700 MB"), Some(734_003_200));
    }

    #[test]
    fn query_encodes_cyrillic_and_paging() {
        let q = SearchQuery {
            text: "игра".into(),
            forum_ids: vec![635],
            sort: Some(SearchSort::Seeders),
            ascending: false,
            page: 2,
        };
        assert_eq!(
            q.to_path(),
            "tracker.php?nm=%E8%E3%F0%E0&f%5B%5D=635&o=10&s=2&start=100"
        );
    }
}
