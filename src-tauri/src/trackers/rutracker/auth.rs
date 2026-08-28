//! Session state detection.
//!
//! Logging in is done by the user in the worker browser window — that is the
//! only way past the Cloudflare challenge, and it handles the captcha and any
//! future anti-bot step for free. What is left here is reading the resulting
//! state out of a page.

use scraper::{Html, Selector};

/// Decides whether a forum page belongs to a signed-in user.
///
/// Two independent signals, because relying on one markup detail is how login
/// detection silently breaks:
///
/// 1. A logout link — only a signed-in page renders one.
/// 2. The guest login block is absent while a user-panel link is present. The
///    anonymous `index.php` definitely carries `login_username` (verified
///    against the live site), so its absence is meaningful.
pub fn is_logged_in_html(html: &str) -> bool {
    if html.contains("logout=1") {
        return true;
    }
    !has_login_form(html)
        && (html.contains("profile.php?mode=viewprofile") || html.contains("pm.php"))
}

/// The guest login block, which disappears once signed in.
pub fn has_login_form(html: &str) -> bool {
    html.contains("login_username")
}

/// Extracts the full logout URL, which carries the session id the forum
/// requires. Returns a path relative to `/forum/`.
pub fn logout_path(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href*=\"logout=1\"]").ok()?;
    let href = doc.select(&sel).next()?.value().attr("href")?;
    Some(
        href.trim_start_matches("./")
            .trim_start_matches('/')
            .to_string(),
    )
}

/// Reads the signed-in user's name out of a forum page header.
///
/// Purely cosmetic — several selector spellings are tried and `None` is a
/// perfectly acceptable answer.
pub fn logged_in_username(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    for spec in [
        "#logged-in-username",
        "a.logged-in-username",
        "#log-out-and-in a[href*=\"profile.php\"]",
        "a[href*=\"profile.php?mode=viewprofile\"]",
    ] {
        let Ok(sel) = Selector::parse(spec) else { continue };
        if let Some(el) = doc.select(&sel).next() {
            let name = el.text().collect::<String>().trim().to_string();
            if !name.is_empty() && name.len() < 64 {
                return Some(name);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_logged_in_page() {
        assert!(is_logged_in_html(
            r#"<a href="./login.php?logout=1&sid=abc">Выход</a>"#
        ));
    }

    #[test]
    fn detects_an_anonymous_page() {
        assert!(!is_logged_in_html(
            r#"<form><input name="login_username"></form>"#
        ));
    }

    #[test]
    fn falls_back_to_the_absent_login_form() {
        // No logout link in this fragment, but the guest block is gone and the
        // user panel is there — that combination only happens when signed in.
        let html = r#"<div id="user-panel"><a href="pm.php?folder=inbox">Сообщения</a></div>"#;
        assert!(is_logged_in_html(html));
    }

    #[test]
    fn a_guest_page_with_profile_links_is_not_logged_in() {
        // Guests can see other users' profiles, so a profile link alone must
        // not count while the login form is still on the page.
        let html = r#"<a href="profile.php?mode=viewprofile&u=1">someone</a>
                      <form><input name="login_username"></form>"#;
        assert!(!is_logged_in_html(html));
    }

    #[test]
    fn reads_the_username_from_a_header() {
        let html = r#"<div><a id="logged-in-username" href="profile.php?mode=viewprofile&u=7">vasya</a></div>"#;
        assert_eq!(logged_in_username(html).as_deref(), Some("vasya"));
    }

    #[test]
    fn username_is_optional() {
        assert!(logged_in_username("<html></html>").is_none());
    }

    #[test]
    fn extracts_the_logout_path_with_its_sid() {
        let html = r#"<a href="./login.php?logout=1&amp;sid=deadbeef">Выход</a>"#;
        assert_eq!(
            logout_path(html).as_deref(),
            Some("login.php?logout=1&sid=deadbeef")
        );
    }
}
