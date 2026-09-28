//! End-to-end smoke test of the core, with no windows and no tracker.
//!
//! Everything before this was unit tests of pure decisions plus "it compiles,
//! launches and does not panic" — which is exactly how fifteen blocked commands
//! and a queue that restarted closed films got through. This runs the real
//! engine over real files: makes a release, builds a `.torrent` from it, adds
//! that torrent on top of the files it came from, and checks the things that
//! have actually broken in this project — the hash check, the piece map, the
//! stream server's byte ranges, re-checking, re-downloading, and the database
//! flags the queue and seeding rules depend on.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use panda_torrent_lib::config::AppConfig;
use panda_torrent_lib::db::Db;
use panda_torrent_lib::db::models::TorrentSource;
use panda_torrent_lib::engine::{AddOptions, AddSource, Engine};
use panda_torrent_lib::streaming::StreamServer;

/// A fresh scratch folder for one test, wiped from any earlier run.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("panda-smoke-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Deterministic file contents, so a served byte range can be checked exactly.
fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i.wrapping_mul(31).wrapping_add(seed as usize) & 0xff) as u8)
        .collect()
}

/// Polls until `check` holds, or fails with `what` after `limit` — saying
/// what the torrent looked like at the end, so a failure explains itself.
async fn wait_until(
    what: &str,
    limit: Duration,
    engine: &Engine,
    hash: &str,
    mut check: impl FnMut() -> bool,
) {
    let started = Instant::now();
    while !check() {
        if started.elapsed() >= limit {
            panic!(
                "не дождались: {what} (за {} с)
состояние: {:?}
файлы: {:?}",
                limit.as_secs(),
                engine.progress_one(hash),
                engine.details(hash).map(|d| d
                    .files
                    .iter()
                    .map(|f| (f.name.clone(), f.downloaded, f.length))
                    .collect::<Vec<_>>()),
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// An engine that talks to nobody: no DHT, no UPnP, no local discovery.
async fn quiet_engine(root: &Path) -> Arc<Engine> {
    let mut cfg = AppConfig::default();
    cfg.download_dir = root.join("downloads");
    cfg.network.enable_dht = false;
    cfg.network.enable_upnp = false;
    cfg.network.enable_lsd = false;
    Engine::start(&cfg, root.join("session"))
        .await
        .expect("движок не запустился")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_release_already_on_disk_is_recognised_streamed_and_repaired() {
    let root = scratch("core");
    let downloads = root.join("downloads");
    let release = downloads.join("Smoke.Release");
    std::fs::create_dir_all(&release).unwrap();

    // Two "episodes", big enough to span a good number of pieces.
    let episode1 = pattern(12 * 1024 * 1024, 7);
    let episode2 = pattern(4 * 1024 * 1024, 42);
    std::fs::write(release.join("episode1.bin"), &episode1).unwrap();
    std::fs::write(release.join("episode2.bin"), &episode2).unwrap();

    // ---- a torrent made from those files -------------------------------------
    let bytes = Engine::create_torrent_bytes(&release, None, Vec::new())
        .await
        .expect("торрент не собрался");
    assert!(bytes.starts_with(b"d"), "это не bencode");

    // ---- added on top of the files it describes -----------------------------
    let engine = quiet_engine(&root).await;
    let added = engine
        .add(
            AddSource::Bytes(bytes),
            AddOptions {
                output_folder: Some(downloads.to_string_lossy().to_string()),
                overwrite: true,
                ..Default::default()
            },
        )
        .await
        .expect("торрент не добавился");
    let hash = added.info_hash.clone();
    assert_eq!(added.files.len(), 2);

    // The engine must be looking exactly where the release lies. A torrent
    // added onto an explicit folder used to land loose in that folder, with
    // no folder of its own — and every hash check then found nothing.
    assert_eq!(
        Path::new(&added.output_folder),
        release.as_path(),
        "раздача должна лежать в своей папке"
    );
    for f in &added.files {
        let mut path = PathBuf::from(&added.output_folder);
        for part in &f.components {
            path.push(part);
        }
        assert!(path.exists(), "движок ищет файл не там: {}", path.display());
    }

    // The hash check must find everything: this is the "existing files are
    // counted, not downloaded again" behaviour that was missing for weeks.
    wait_until("раздача засчитана как готовая", Duration::from_secs(60), &engine, &hash, || {
        engine.progress_one(&hash).map(|p| p.finished).unwrap_or(false)
    })
    .await;
    let progress = engine.progress_one(&hash).unwrap();
    assert_eq!(progress.progress_bytes, progress.total_bytes);
    assert_eq!(progress.total_bytes as usize, episode1.len() + episode2.len());

    let details = engine.details(&hash).unwrap();
    for f in &details.files {
        assert_eq!(f.downloaded, f.length, "файл {} не засчитан целиком", f.name);
        assert!(f.included);
    }
    let idx1 = details.files.iter().find(|f| f.name == "episode1.bin").unwrap().index;
    let idx2 = details.files.iter().find(|f| f.name == "episode2.bin").unwrap().index;

    // ---- the piece map says "all of it, everywhere" ----------------------------
    let map = engine.pieces(&hash).expect("карта кусков не отдана");
    assert!(map.total_pieces > 1, "слишком крупные куски для проверки карты");
    assert_eq!(map.have_pieces, map.total_pieces);
    assert!(map.buckets.iter().all(|&b| b == 100), "не все корзины полны: {:?}", map.buckets);
    assert_eq!(map.files.len(), 2);
    for f in &map.files {
        assert_eq!(f.have_pieces, f.total_pieces);
    }
    // Pieces do not respect file boundaries: the one straddling the two
    // files belongs to both, and deleting either file empties it.
    let piece_len: u64 =
        (progress.total_bytes + map.total_pieces as u64 - 1) / map.total_pieces as u64;

    // ---- the stream server serves exactly the bytes asked for -----------------
    let server = StreamServer::start(engine.clone())
        .await
        .expect("сервер потока не запустился");
    let url = server.url_for(&hash, idx1);
    let client = reqwest::Client::new();

    let resp = client
        .get(&url)
        .header("Range", "bytes=1024-2047")
        .send()
        .await
        .expect("запрос к серверу потока не прошёл");
    assert_eq!(resp.status().as_u16(), 206, "ожидали 206 Partial Content");
    let body = resp.bytes().await.unwrap();
    assert_eq!(body.len(), 1024);
    assert_eq!(&body[..], &episode1[1024..2048], "сервер отдал не те байты");

    // An open-ended range runs to the end of the file — the bug that stopped
    // films five minutes in was a cap right here.
    let tail = episode1.len() - 4096;
    let resp = client
        .get(&url)
        .header("Range", format!("bytes={tail}-"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 206);
    let body = resp.bytes().await.unwrap();
    assert_eq!(body.len(), 4096);
    assert_eq!(&body[..], &episode1[tail..]);

    // A range past the end is refused, not answered with the whole file.
    let resp = client
        .get(&url)
        .header("Range", "bytes=99999999-")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 416);

    // ---- a re-check leaves a complete torrent complete --------------------------
    engine.recheck(&hash).await.expect("перепроверка не удалась");
    wait_until("после перепроверки снова готово", Duration::from_secs(60), &engine, &hash, || {
        engine.progress_one(&hash).map(|p| p.finished).unwrap_or(false)
    })
    .await;

    // ---- re-downloading one file empties exactly that file -------------------
    engine
        .redownload_file(&hash, idx2)
        .await
        .expect("перекачка файла не удалась");
    wait_until("второй файл пуст, первый цел, кроме куска, общего с ним", Duration::from_secs(60), &engine, &hash, || {
            engine
                .details(&hash)
                .map(|d| {
                    let f1 = d.files.iter().find(|f| f.index == idx1).unwrap();
                    let f2 = d.files.iter().find(|f| f.index == idx2).unwrap();
                    f2.downloaded == 0
                        && f1.downloaded > 0
                        && f1.downloaded + piece_len >= f1.length
                })
                .unwrap_or(false)
        },
    )
    .await;
    assert!(
        !engine.progress_one(&hash).unwrap().finished,
        "раздача с выброшенным файлом не может быть готовой"
    );
    let map = engine.pieces(&hash).unwrap();
    let f2 = map.files.iter().find(|f| f.index == idx2).unwrap();
    assert_eq!(f2.have_pieces, 0, "карта кусков не заметила пропажу файла");

    // ---- pause, resume, delete ----------------------------------------------------
    engine.pause(&hash).await.unwrap();
    wait_until("пауза", Duration::from_secs(10), &engine, &hash, || {
        engine.progress_one(&hash).map(|p| p.state == "paused").unwrap_or(false)
    })
    .await;
    engine.resume(&hash).await.unwrap();
    wait_until("снова в работе", Duration::from_secs(10), &engine, &hash, || {
        engine.progress_one(&hash).map(|p| p.state != "paused").unwrap_or(false)
    })
    .await;

    engine.delete(&hash).await.unwrap();
    assert!(engine.progress_one(&hash).is_err(), "удалённая раздача всё ещё в сессии");
    wait_until("файлы удалены с диска", Duration::from_secs(10), &engine, &hash, || {
        !release.join("episode1.bin").exists()
    })
    .await;

    engine.shutdown().await;
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_database_migrates_and_keeps_every_flag() {
    let root = scratch("db");
    let db = Db::open(&root.join("panda.db")).expect("база не открылась");

    db.upsert_torrent(
        "ABCDEF0123456789ABCDEF0123456789ABCDEF01",
        "Смок-раздача",
        root.to_string_lossy().as_ref(),
        4096,
        TorrentSource::File,
        Some(777),
        None,
    )
    .unwrap();

    // The three flags the queue and seeding rules read every second. Each is
    // written by its own command; a write that silently failed would leave a
    // ticked box that changes nothing.
    db.set_forced("ABCDEF0123456789ABCDEF0123456789ABCDEF01", true).unwrap();
    db.set_no_seeding("ABCDEF0123456789ABCDEF0123456789ABCDEF01", true).unwrap();
    db.set_user_paused("ABCDEF0123456789ABCDEF0123456789ABCDEF01", true).unwrap();

    let rows = db.list_torrents().unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.name, "Смок-раздача");
    assert_eq!(row.topic_id, Some(777));
    assert!(row.no_seeding);
    // The pause came last, so it wins: "force" and "paused by hand" contradict,
    // and whichever a person did most recently is what they meant.
    assert!(row.user_paused);
    assert!(!row.forced, "ручная пауза должна снимать принудительный запуск");

    // And the other way round.
    db.set_forced("ABCDEF0123456789ABCDEF0123456789ABCDEF01", true).unwrap();
    let row = &db.list_torrents().unwrap()[0];
    assert!(row.forced);
    assert!(!row.user_paused, "принудительный запуск должен снимать ручную паузу");

    db.set_forced("ABCDEF0123456789ABCDEF0123456789ABCDEF01", false).unwrap();
    assert!(!db.list_torrents().unwrap()[0].forced);

    // Viewing position round-trips, and the thresholds that make "resume"
    // meaningful are honoured.
    db.history_add(Some(777), None, "Смок-фильм", Some("film.mkv"), None, false)
        .unwrap();
    db.history_set_position(Some(777), Some("film.mkv"), 1500.0, Some(6000.0))
        .unwrap();
    assert_eq!(
        db.history_position(Some(777), Some("film.mkv")).unwrap(),
        Some(1500.0)
    );
    db.history_set_position(Some(777), Some("film.mkv"), 5980.0, Some(6000.0))
        .unwrap();
    assert_eq!(
        db.history_position(Some(777), Some("film.mkv")).unwrap(),
        None,
        "последняя минута считается досмотренной"
    );

    db.delete_torrent("ABCDEF0123456789ABCDEF0123456789ABCDEF01").unwrap();
    assert!(db.list_torrents().unwrap().is_empty());

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
}
