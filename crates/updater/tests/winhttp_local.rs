//! The REAL Windows HTTP client (WinHTTP) against a local server that plays GitHub (127.0.0.1, scratch only - no internet).
//! Not covered here: https/TLS and the real github.com (no internet in tests; see the report).

mod common;

use bu_updater::*;
use common::*;
use std::sync::Arc;

type Progress = Vec<(u64, Option<u64>)>;

fn get(url: &str) -> (Result<Response>, Vec<u8>, Progress) {
    let http = WinHttp::new("BoylerUtilities/9.9.9");
    let mut out = Vec::new();
    let mut prog = Vec::new();
    let r = http.get(&Request { url, accept: "application/vnd.github+json" }, &mut out, &mut |d, t| prog.push((d, t)));
    (r, out, prog)
}

#[test]
fn a_body_is_streamed_with_progress_and_the_headers_we_promise() {
    let s = TestServer::start();
    let body = fake_exe_bytes("x", 700_000);
    s.body("/file", body.clone());
    let (r, out, prog) = get(&s.url("/file"));
    let r = r.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.content_length, Some(body.len() as u64));
    assert_eq!(out, body);
    assert!(prog.len() >= 2 && prog.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(prog.last().unwrap().0, body.len() as u64);
    let hits = s.hits();
    assert_eq!(hits.len(), 1);
    let req = hits[0].1.to_ascii_lowercase();
    assert!(req.starts_with("get /file http/1.1"), "{req}");
    assert!(req.contains("user-agent: boylerutilities/9.9.9"), "GitHub rejects requests with no User-Agent: {req}");
    assert!(req.contains("accept: application/vnd.github+json"), "{req}");
}

#[test]
fn an_error_status_comes_back_as_a_status_with_no_body() {
    let s = TestServer::start();
    s.route("/gone", Route::Body { status: 404, body: b"<html>nope</html>".to_vec() });
    s.route("/limit", Route::Body { status: 403, body: b"rate".to_vec() });
    let (r, out, _) = get(&s.url("/gone"));
    assert_eq!(r.unwrap().status, 404);
    assert!(out.is_empty(), "an error page must never reach the sink");
    let (r, out, _) = get(&s.url("/limit"));
    assert_eq!(r.unwrap().status, 403);
    assert!(out.is_empty());
    let (r, _, _) = get(&s.url("/not-routed"));
    assert_eq!(r.unwrap().status, 404);
}

#[test]
fn redirects_are_followed_like_a_github_download_link() {
    let s = TestServer::start();
    s.route("/releases/download/v2/app.exe", Route::Redirect(s.url("/objects/abc")));
    s.body("/objects/abc", b"MZ-redirected".to_vec());
    let (r, out, _) = get(&s.url("/releases/download/v2/app.exe"));
    assert_eq!(r.unwrap().status, 200);
    assert_eq!(out, b"MZ-redirected");
    assert_eq!(s.paths(), vec!["/releases/download/v2/app.exe", "/objects/abc"]);
}

#[test]
fn nothing_listening_is_a_network_error() {
    // grab a free port, close it again
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let (r, out, _) = get(&format!("http://127.0.0.1:{port}/x"));
    assert!(matches!(r, Err(UpdateError::Network { .. })), "{r:?}");
    assert!(out.is_empty());
}

#[test]
fn a_connection_cut_mid_download_is_never_mistaken_for_a_whole_file() {
    let s = TestServer::start();
    let body = fake_exe_bytes("x", 100_000);
    s.route("/cut", Route::Cut { body: body.clone(), sent: 40_000 });
    let (r, out, _) = get(&s.url("/cut"));
    // WinHTTP either reports the error, or hands over fewer bytes than announced - both are caught by the updater
    println!("cut download: result = {r:?}, bytes received = {}", out.len());
    let short = match &r {
        Err(UpdateError::Network { .. }) => true,
        Ok(resp) => resp.content_length.is_some_and(|c| c != out.len() as u64),
        Err(_) => false,
    };
    assert!(short, "{r:?} with {} bytes", out.len());
    assert!(out.len() < body.len());
}

#[test]
fn bad_links_never_reach_the_network() {
    let http = WinHttp::new("t");
    let mut out = Vec::new();
    for bad in ["ftp://x/y", "https://user@host/x", "not a link"] {
        assert!(http.get(&Request { url: bad, accept: "*/*" }, &mut out, &mut |_, _| {}).is_err(), "{bad}");
    }
}

#[test]
fn the_whole_check_goes_through_the_real_client() {
    let s = TestServer::start();
    let mut cfg = UpdaterConfig::new("o/r", "1.0.0", std::env::current_exe().unwrap());
    cfg.api_base = s.base();
    let fake_installer = Arc::new(FakeInstaller::default());
    let u = Updater::new(cfg, Box::new(WinHttp::new("BoylerUtilities/1.0.0")), Box::new(fake_installer));
    assert!(s.hits().is_empty(), "building the updater makes no request");

    // no release yet
    assert_eq!(u.check().unwrap(), CheckResult::NoReleases);
    // a newer one
    let bytes = fake_exe_bytes("n", 1000);
    s.release("o/r", "v1.5.0", "BoylerUtilities.exe", "/dl/BoylerUtilities.exe", bytes.len() as u64, Some(&format!("sha256:{}", sha256_of(&bytes))));
    match u.check().unwrap() {
        CheckResult::Available(i) => {
            assert_eq!(i.version.to_string(), "1.5.0");
            assert_eq!(i.asset.url, s.url("/dl/BoylerUtilities.exe"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(s.paths(), vec!["/repos/o/r/releases/latest", "/repos/o/r/releases/latest"], "one request per check, nothing else");
}

#[test]
fn a_cut_download_over_the_real_client_is_refused_and_leaves_nothing() {
    let s = TestServer::start();
    let dir = Scratch::new("winhttp-cut");
    let exe = dir.join("BoylerUtilities.exe");
    std::fs::write(&exe, fake_exe_bytes("old", 2000)).unwrap();
    let bytes = fake_exe_bytes("new", 100_000);
    s.release("o/r", "v2.0.0", "BoylerUtilities.exe", "/dl/BoylerUtilities.exe", bytes.len() as u64, None);
    s.route("/dl/BoylerUtilities.exe", Route::Cut { body: bytes.clone(), sent: 30_000 });
    let mut cfg = UpdaterConfig::new("o/r", "1.0.0", exe.clone());
    cfg.api_base = s.base();
    let installer = Arc::new(FakeInstaller::default());
    let u = Updater::new(cfg, Box::new(WinHttp::new("BoylerUtilities/1.0.0")), Box::new(installer.clone()));
    let CheckResult::Available(info) = u.check().unwrap() else { panic!("expected an update") };
    let r = u.update(&info, &mut |_| {});
    assert!(matches!(r, Err(UpdateError::SizeMismatch { got: 30_000, .. }) | Err(UpdateError::Network { .. })), "{r:?}");
    let mut names: Vec<String> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, vec!["BoylerUtilities.exe"], "no partial file may remain");
    assert!(installer.plans.lock().unwrap().is_empty());
}
