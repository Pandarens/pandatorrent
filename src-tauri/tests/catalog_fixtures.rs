//! Catalogue parsing checked against markup captured from the live tracker.
//!
//! The fixture holds two real `div.category` blocks from `index.php`, including
//! a subforum list that mixes a real subforum with an announcement topic — the
//! exact case that has to be filtered. A layout change on the site shows up
//! here rather than as a half-empty catalogue in the UI.

use panda_torrent_lib::trackers::rutracker::forums::{flatten, parse_catalog};

const INDEX: &str = include_str!("fixtures/rutracker-index.html");

#[test]
fn parses_the_real_forum_tree() {
    let cats = parse_catalog(INDEX).expect("index page should yield a catalogue");

    assert_eq!(cats.len(), 2, "fixture holds two category blocks");
    assert!(
        cats.iter().any(|c| c.title.contains("Игры")),
        "expected a games category, got: {:?}",
        cats.iter().map(|c| &c.title).collect::<Vec<_>>()
    );

    let forums: usize = cats.iter().map(|c| c.forums.len()).sum();
    assert_eq!(forums, 11, "every h4.forumlink should become a forum");
}

#[test]
fn finds_the_new_releases_subforum() {
    // f=635 «Новинки» sits under f=5 «Игры для Windows» and is only reachable
    // through the subforum list — the case the first catalogue version missed.
    let cats = parse_catalog(INDEX).unwrap();

    let windows = cats
        .iter()
        .flat_map(|c| &c.forums)
        .find(|f| f.id == 5)
        .expect("forum 5 should be present");

    assert!(
        windows.subforums.iter().any(|s| s.id == 635),
        "subforum 635 should hang off forum 5: {:?}",
        windows.subforums
    );
    assert!(
        windows
            .subforums
            .iter()
            .all(|s| !s.title.contains("Правила") && s.id != 0),
        "announcement topics must not be treated as subforums"
    );
}

#[test]
fn flatten_exposes_subforums_with_context() {
    let flat = flatten(&parse_catalog(INDEX).unwrap());
    let entry = flat
        .iter()
        .find(|f| f.id == 635)
        .expect("subforum should be selectable in a picker");
    assert!(
        entry.title.contains("Игры для Windows"),
        "a bare «Новинки» would be meaningless in the settings picker: {}",
        entry.title
    );
}
