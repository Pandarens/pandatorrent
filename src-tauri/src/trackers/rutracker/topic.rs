//! Parsing of a `viewtopic.php` page.
//!
//! The opening post is turned into a small block tree rather than flat text,
//! because a tracker description is structured content: line breaks separate
//! cast lists and technical specs, screenshots belong in a gallery, and the
//! extra material authors hide behind spoilers should stay hidden until asked
//! for. Replies are parsed separately so the UI can put them on their own tab.
//!
//! RuTracker specifics this relies on:
//!   * post images are lazy-loaded — the real URL is in `title` on a
//!     `<var class="postImg">`, not in an `<img src>`;
//!   * spoilers are `<div class="sp-wrap"><div class="sp-head">…</div>
//!     <div class="sp-body">…</div></div>`;
//!   * each post lives in `<div class="post_wrap">` with its body in
//!     `.post_body`.

use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node, Selector};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// A piece of a post.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PostBlock {
    /// A run of text with its line breaks preserved.
    Text { text: String },
    /// An image, shown in the gallery.
    Image { url: String },
    /// Collapsed by default, exactly as on the site.
    Spoiler { title: String, blocks: Vec<PostBlock> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicComment {
    pub author: Option<String>,
    pub posted_at: Option<String>,
    pub blocks: Vec<PostBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicDetails {
    pub topic_id: i64,
    pub title: String,
    /// Structured opening post.
    pub blocks: Vec<PostBlock>,
    /// Flattened text, kept for search results and card titles.
    pub description: String,
    /// Every image in the opening post, in page order.
    pub images: Vec<String>,
    pub magnet: Option<String>,
    pub info_hash: Option<String>,
    pub size_bytes: Option<i64>,
    pub comments: Vec<TopicComment>,
}

pub fn parse_topic(html: &str, topic_id: i64) -> AppResult<TopicDetails> {
    if super::search::looks_like_login_page(html) {
        return Err(AppError::NotAuthenticated);
    }

    let doc = Html::parse_document(html);
    let sel = |s: &str| Selector::parse(s).expect("static selector");

    let title = doc
        .select(&sel("h1.maintitle, #topic-title, h1"))
        .next()
        .map(|e| collapse(&e.text().collect::<String>()))
        .unwrap_or_default();

    let post_sel = sel("div.post_body");
    let mut posts = doc.select(&post_sel);

    let first = posts.next();
    let blocks = first.map(|p| parse_blocks(*p)).unwrap_or_default();

    let mut images = Vec::new();
    collect_images(&blocks, &mut images);

    let magnet = doc
        .select(&sel("a.magnet-link, a[href^=\"magnet:\"]"))
        .next()
        .and_then(|a| a.value().attr("href"))
        .map(str::to_string);

    let info_hash = doc
        .select(&sel("#tor-hash"))
        .next()
        .map(|e| collapse(&e.text().collect::<String>()))
        .filter(|h| h.len() == 40 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|h| h.to_uppercase())
        .or_else(|| magnet.as_deref().and_then(hash_from_magnet));

    let size_bytes = doc
        .select(&sel("#tor-size-humn, span.tor-size-humn"))
        .next()
        .and_then(|e| e.value().attr("title").map(str::to_string))
        .and_then(|t| t.trim().parse().ok());

    Ok(TopicDetails {
        topic_id,
        title,
        description: flatten(&blocks),
        images,
        magnet,
        info_hash,
        size_bytes,
        comments: parse_comments(&doc),
        blocks,
    })
}

/// Replies to the topic — every post after the opening one.
fn parse_comments(doc: &Html) -> Vec<TopicComment> {
    let wrap_sel = Selector::parse("div.post_wrap").expect("static selector");
    let body_sel = Selector::parse("div.post_body").expect("static selector");
    let nick_sel = Selector::parse("p.nick, .post-user-message .nick, a.nick").expect("static");
    let date_sel = Selector::parse("a.p-link, .post_head .p-link").expect("static");

    doc.select(&wrap_sel)
        // The first post is the description, already parsed above.
        .skip(1)
        .filter_map(|wrap| {
            let body = wrap.select(&body_sel).next()?;
            let blocks = parse_blocks(*body);
            // A post with nothing readable in it is not worth a row.
            if flatten(&blocks).trim().is_empty() {
                return None;
            }
            Some(TopicComment {
                author: wrap
                    .select(&nick_sel)
                    .next()
                    .map(|n| collapse(&n.text().collect::<String>()))
                    .filter(|s| !s.is_empty()),
                posted_at: wrap
                    .select(&date_sel)
                    .next()
                    .map(|d| collapse(&d.text().collect::<String>()))
                    .filter(|s| !s.is_empty()),
                blocks,
            })
        })
        .take(200)
        .collect()
}

/// Walks a post body into blocks, keeping line breaks and spoilers intact.
fn parse_blocks(root: NodeRef<'_, Node>) -> Vec<PostBlock> {
    let mut out = Vec::new();
    let mut text = String::new();
    walk(root, &mut out, &mut text);
    flush_text(&mut out, &mut text);
    out
}

fn walk(node: NodeRef<'_, Node>, out: &mut Vec<PostBlock>, text: &mut String) {
    for child in node.children() {
        match child.value() {
            Node::Text(t) => text.push_str(t),
            Node::Element(el) => {
                let Some(element) = ElementRef::wrap(child) else {
                    continue;
                };
                let name = el.name();
                let class = el.attr("class").unwrap_or_default();

                if class.contains("sp-wrap") {
                    flush_text(out, text);
                    if let Some(spoiler) = parse_spoiler(element) {
                        out.push(spoiler);
                    }
                    continue;
                }

                // Lazy-loaded post image: the URL is the `title` attribute.
                if name == "var" && class.contains("postImg") {
                    if let Some(url) = el.attr("title").and_then(normalise_image) {
                        flush_text(out, text);
                        out.push(PostBlock::Image { url });
                    }
                    continue;
                }
                if name == "img" {
                    if let Some(url) = el.attr("src").and_then(normalise_image) {
                        flush_text(out, text);
                        out.push(PostBlock::Image { url });
                    }
                    continue;
                }

                if name == "br" {
                    text.push('\n');
                    continue;
                }
                // Block-level elements end a line; inline ones do not.
                let block_level = matches!(
                    name,
                    "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "hr" | "blockquote"
                );
                if block_level && !text.ends_with('\n') {
                    text.push('\n');
                }
                walk(child, out, text);
                if block_level && !text.ends_with('\n') {
                    text.push('\n');
                }
            }
            _ => {}
        }
    }
}

fn parse_spoiler(element: ElementRef<'_>) -> Option<PostBlock> {
    let head_sel = Selector::parse("div.sp-head, .sp-head").ok()?;
    let body_sel = Selector::parse("div.sp-body, .sp-body").ok()?;

    let title = element
        .select(&head_sel)
        .next()
        .map(|h| collapse(&h.text().collect::<String>()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Подробнее".to_string());

    let body = element.select(&body_sel).next()?;
    let blocks = parse_blocks(*body);
    if blocks.is_empty() {
        return None;
    }
    Some(PostBlock::Spoiler { title, blocks })
}

fn flush_text(out: &mut Vec<PostBlock>, text: &mut String) {
    let cleaned = tidy_lines(text);
    text.clear();
    if !cleaned.is_empty() {
        out.push(PostBlock::Text { text: cleaned });
    }
}

/// Squeezes runs of spaces inside lines and runs of blank lines between them,
/// which is what turns raw forum markup into something readable.
fn tidy_lines(raw: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in raw.replace('\r', "").split('\n') {
        let squeezed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        // Collapse consecutive blank lines to at most one.
        if squeezed.is_empty() && lines.last().map(String::is_empty).unwrap_or(true) {
            continue;
        }
        lines.push(squeezed);
    }
    while lines.last().map(String::is_empty).unwrap_or(false) {
        lines.pop();
    }
    lines.join("\n")
}

fn collect_images(blocks: &[PostBlock], out: &mut Vec<String>) {
    for block in blocks {
        match block {
            PostBlock::Image { url } => {
                if !out.contains(url) {
                    out.push(url.clone());
                }
            }
            PostBlock::Spoiler { blocks, .. } => collect_images(blocks, out),
            PostBlock::Text { .. } => {}
        }
    }
}

/// Plain-text rendering, used where a single string is needed.
fn flatten(blocks: &[PostBlock]) -> String {
    let mut parts = Vec::new();
    for block in blocks {
        match block {
            PostBlock::Text { text } => parts.push(text.clone()),
            PostBlock::Spoiler { title, blocks } => {
                parts.push(format!("{title}\n{}", flatten(blocks)));
            }
            PostBlock::Image { .. } => {}
        }
    }
    tidy_lines(&parts.join("\n"))
}

fn normalise_image(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.starts_with("data:") {
        return None;
    }
    let absolute = if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else if url.starts_with("http") {
        url.to_string()
    } else {
        return None;
    };
    // Smilies and forum chrome are not artwork.
    if absolute.contains("/smiles/") || absolute.contains("/templates/") {
        return None;
    }
    Some(absolute)
}

pub fn hash_from_magnet(magnet: &str) -> Option<String> {
    let marker = "urn:btih:";
    let pos = magnet.find(marker)? + marker.len();
    let hash: String = magnet[pos..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    (hash.len() == 40).then(|| hash.to_uppercase())
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
      <h1 class="maintitle">Half-Life 2 [RePack]</h1>
      <div class="post_wrap">
        <p class="nick">uploader</p>
        <div class="post_body">
          Жанр: Шутер<br>Разработчик: Valve
          <var class="postImg" title="https://i.example.com/shot1.jpg"></var>
          <div class="sp-wrap">
            <div class="sp-head folded">Системные требования</div>
            <div class="sp-body">ОЗУ: 4 ГБ<br>Видео: DX11
              <var class="postImg" title="https://i.example.com/shot2.jpg"></var>
            </div>
          </div>
          <img src="https://rutracker.org/templates/smiles/ok.gif">
        </div>
      </div>
      <div class="post_wrap">
        <p class="nick">someone</p>
        <a class="p-link">12-Май-24 10:00</a>
        <div class="post_body">Спасибо, работает!</div>
      </div>
      <a class="magnet-link" href="magnet:?xt=urn:btih:A1B2C3D4E5F60718293A4B5C6D7E8F9012345678">M</a>
    "#;

    #[test]
    fn keeps_line_breaks_instead_of_one_long_line() {
        let t = parse_topic(PAGE, 42).unwrap();
        let text = match &t.blocks[0] {
            PostBlock::Text { text } => text.clone(),
            other => panic!("expected text first, got {other:?}"),
        };
        assert!(
            text.contains("Жанр: Шутер\nРазработчик: Valve"),
            "a <br> must survive as a newline: {text:?}"
        );
    }

    #[test]
    fn spoilers_stay_folded_as_their_own_block() {
        let t = parse_topic(PAGE, 42).unwrap();
        let spoiler = t
            .blocks
            .iter()
            .find_map(|b| match b {
                PostBlock::Spoiler { title, blocks } => Some((title.clone(), blocks.clone())),
                _ => None,
            })
            .expect("the spoiler should be its own block");
        assert_eq!(spoiler.0, "Системные требования");
        assert!(
            spoiler.1.iter().any(|b| matches!(b, PostBlock::Image { .. })),
            "images inside a spoiler belong to that spoiler"
        );
    }

    #[test]
    fn gallery_collects_images_including_those_in_spoilers() {
        let t = parse_topic(PAGE, 42).unwrap();
        assert_eq!(
            t.images,
            vec![
                "https://i.example.com/shot1.jpg",
                "https://i.example.com/shot2.jpg"
            ],
            "smilies must be excluded, spoiler images included"
        );
    }

    #[test]
    fn replies_are_separated_from_the_description() {
        let t = parse_topic(PAGE, 42).unwrap();
        assert_eq!(t.comments.len(), 1, "only the second post is a comment");
        assert_eq!(t.comments[0].author.as_deref(), Some("someone"));
        assert_eq!(t.comments[0].posted_at.as_deref(), Some("12-Май-24 10:00"));
        assert!(!t.description.contains("Спасибо"));
    }

    #[test]
    fn reads_the_hash_from_a_magnet_link() {
        let m = "magnet:?xt=urn:btih:a1b2c3d4e5f60718293a4b5c6d7e8f9012345678&dn=name";
        assert_eq!(
            hash_from_magnet(m).as_deref(),
            Some("A1B2C3D4E5F60718293A4B5C6D7E8F9012345678")
        );
    }

    #[test]
    fn runs_of_blank_lines_collapse_to_one() {
        // One blank line survives on purpose — it is what separates paragraphs
        // in a description. Longer runs, and trailing ones, do not.
        assert_eq!(tidy_lines("a\n\n\n\nb   c\n\n"), "a\n\nb c");
        assert_eq!(tidy_lines("a\nb"), "a\nb");
    }
}
