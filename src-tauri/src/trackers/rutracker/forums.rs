//! The forum catalogue, for browsing without a search query.
//!
//! `index.php` carries the entire tree — 26 categories, ~150 forums and ~50
//! subforums — so one page load is enough and no per-category fetch is needed.
//! The page is readable without signing in (verified against the live site), so
//! the catalogue fills in even before the user logs in.
//!
//! Structure, as captured from the live page:
//!
//! ```html
//! <div id="c-36" class="category">
//!   <h3 class="cat_title"><a href="index.php?c=36">ОБХОД БЛОКИРОВОК</a></h3>
//!   <table id="cf-36" class="forums">
//!     <tr><td><h4 class="forumlink"><a href="viewforum.php?f=5">Игры для Windows</a></h4>
//!       <p class="subforums">
//!         <span class="sf_title"><a href="viewforum.php?f=635">Новинки</a></span>
//!       </p></td></tr>
//! ```
//!
//! Subforum lists also hold `viewtopic.php` announcement links; matching only
//! on `viewforum.php?f=` filters those out.
//!
//! A forum id from here drops straight into `tracker.php?f[]=N`, the same
//! endpoint search uses — so browsing reuses the search parser.

use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// A forum that can be browsed directly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ForumEntry {
    pub id: i64,
    pub title: String,
}

/// A forum together with its subforums, e.g. "Игры для Windows" → "Новинки".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogForum {
    pub id: i64,
    pub title: String,
    pub subforums: Vec<ForumEntry>,
}

/// A top-level section of the tracker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogCategory {
    pub id: i64,
    pub title: String,
    pub forums: Vec<CatalogForum>,
}

/// Parses the whole forum tree out of `index.php`.
pub fn parse_catalog(html: &str) -> AppResult<Vec<CatalogCategory>> {
    let doc = Html::parse_document(html);

    let category_sel = sel("div.category");
    let title_sel = sel("h3.cat_title a, h3.cat_title");
    let forum_sel = sel("h4.forumlink a[href*=\"viewforum.php?f=\"]");
    let row_sel = sel("table.forums tr");
    let sub_sel = sel("p.subforums a[href*=\"viewforum.php?f=\"]");

    let mut categories = Vec::new();
    for block in doc.select(&category_sel) {
        let title = block
            .select(&title_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();

        // The id lives in the heading link, with the container id as a backup.
        let id = block
            .select(&title_sel)
            .next()
            .and_then(|a| a.value().attr("href"))
            .and_then(|href| id_after(href, "index.php?c="))
            .or_else(|| {
                block
                    .value()
                    .attr("id")
                    .and_then(|v| v.strip_prefix("c-"))
                    .and_then(|v| v.parse().ok())
            });
        let Some(id) = id else { continue };

        let mut forums = Vec::new();
        for row in block.select(&row_sel) {
            let Some(link) = row.select(&forum_sel).next() else {
                continue;
            };
            let Some(forum_id) = link
                .value()
                .attr("href")
                .and_then(|h| id_after(h, "viewforum.php?f="))
            else {
                continue;
            };
            let forum_title = text_of(&link);
            if forum_title.is_empty() {
                continue;
            }

            let mut subforums = Vec::new();
            for sub in row.select(&sub_sel) {
                let Some(sub_id) = sub
                    .value()
                    .attr("href")
                    .and_then(|h| id_after(h, "viewforum.php?f="))
                else {
                    continue;
                };
                let sub_title = text_of(&sub);
                if sub_title.is_empty() || subforums.iter().any(|s: &ForumEntry| s.id == sub_id) {
                    continue;
                }
                subforums.push(ForumEntry {
                    id: sub_id,
                    title: sub_title,
                });
            }

            forums.push(CatalogForum {
                id: forum_id,
                title: forum_title,
                subforums,
            });
        }

        if !forums.is_empty() {
            categories.push(CatalogCategory { id, title, forums });
        }
    }

    if categories.is_empty() {
        return Err(AppError::Parse(
            "не найден каталог разделов — вероятно, изменилась вёрстка трекера".into(),
        ));
    }
    Ok(categories)
}

/// Flattens the tree into every browsable forum, for pickers and lookups.
pub fn flatten(categories: &[CatalogCategory]) -> Vec<ForumEntry> {
    let mut out = Vec::new();
    for category in categories {
        for forum in &category.forums {
            out.push(ForumEntry {
                id: forum.id,
                title: forum.title.clone(),
            });
            for sub in &forum.subforums {
                out.push(ForumEntry {
                    id: sub.id,
                    // Qualified, since "Новинки" alone says nothing out of context.
                    title: format!("{} · {}", forum.title, sub.title),
                });
            }
        }
    }
    out
}

fn sel(spec: &str) -> Selector {
    Selector::parse(spec).expect("static selector")
}

fn id_after(href: &str, marker: &str) -> Option<i64> {
    let pos = href.find(marker)? + marker.len();
    let digits: String = href[pos..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn text_of(el: &ElementRef<'_>) -> String {
    el.text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shape copied from the live `index.php`.
    const PAGE: &str = r#"
      <div id="c-2" class="category">
        <h3 class="cat_title"><a href="index.php?c=2" rel="nofollow">Игры и Софт</a></h3>
        <table id="cf-2" class="forums">
          <tr id="f-5">
            <td class="row1"><h4 class="forumlink"><a href="viewforum.php?f=5">Игры для Windows</a></h4>
              <p class="subforums">
                <span class="sf_title"><span class="dot-sf">•</span><a href="viewforum.php?f=635">Новинки</a></span>
                <span class="sf_title"><a href="viewtopic.php?t=999">Правила раздела</a></span>
              </p>
            </td>
          </tr>
          <tr id="f-548">
            <td class="row1"><h4 class="forumlink"><a href="viewforum.php?f=548">Игры для консолей</a></h4></td>
          </tr>
        </table>
      </div>"#;

    #[test]
    fn parses_categories_forums_and_subforums() {
        let cats = parse_catalog(PAGE).unwrap();
        assert_eq!(cats.len(), 1);
        assert_eq!(cats[0].id, 2);
        assert_eq!(cats[0].title, "Игры и Софт");
        assert_eq!(cats[0].forums.len(), 2);

        let windows = &cats[0].forums[0];
        assert_eq!(windows.id, 5);
        assert_eq!(
            windows.subforums,
            vec![ForumEntry {
                id: 635,
                title: "Новинки".into()
            }],
            "announcement topics in the subforum list must be ignored"
        );
        assert!(cats[0].forums[1].subforums.is_empty());
    }

    #[test]
    fn flatten_qualifies_subforum_names() {
        let flat = flatten(&parse_catalog(PAGE).unwrap());
        assert_eq!(flat.len(), 3);
        assert!(
            flat.iter().any(|f| f.id == 635 && f.title.contains("Игры для Windows")),
            "a bare «Новинки» would be meaningless in a picker: {flat:?}"
        );
    }

    #[test]
    fn unknown_markup_is_an_error_not_an_empty_catalogue() {
        // Silently returning nothing would read as "the tracker has no
        // sections", which is a much worse thing to show the user.
        assert!(parse_catalog("<html><body>nothing</body></html>").is_err());
    }
}
