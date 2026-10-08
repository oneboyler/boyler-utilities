//! Every row / behaviour of DESIGN §3.15 against the fake Windows: ranking, groups and caps, chips, the backend choice,
//! the right-click menu, open, release. Nothing here touches the real PC.

use bu_search::fake::hit;
use bu_search::*;
use std::sync::Arc;

fn svc(os: &Arc<FakeOs>) -> SearchService {
    SearchService::new(os.clone())
}

fn q(text: &str, f: Filter) -> Query {
    Query::new(text, f)
}

fn names(g: &Group) -> Vec<&str> {
    g.items.iter().map(|i| i.name.as_str()).collect()
}

fn group(r: &SearchResults, kind: ItemKind) -> &Group {
    r.groups.iter().find(|g| g.kind == kind).unwrap_or_else(|| panic!("no {kind:?} group in {:?}", r.groups.iter().map(|g| g.title).collect::<Vec<_>>()))
}

/// The drawing's kind of data: apps, folders and files of a gamer's PC.
fn sample() -> Arc<FakeOs> {
    let os = Arc::new(FakeOs::new());
    for (n, p, path) in [
        ("Discord", "com.squirrel.Discord", Some(r"C:\Users\x\AppData\Local\Discord\Update.exe")),
        ("Steam", r"{7C5A40EF}\Steam\steam.exe", Some(r"C:\Program Files (x86)\Steam\steam.exe")),
        ("VALORANT", "VALORANT", Some(r"C:\Riot Games\Riot Client\RiotClientServices.exe")),
        ("Riot Client", "Riot", Some(r"C:\Riot Games\Riot Client\RiotClientServices.exe")),
        ("Notepad", "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App", None),
        ("Paint", "Microsoft.Paint_8wekyb3d8bbwe!App", None),
        ("OBS Studio", r"{6D809377}\obs-studio\bin\64bit\obs64.exe", Some(r"C:\Program Files\obs-studio\bin\64bit\obs64.exe")),
        ("Discord PTB", "com.squirrel.DiscordPTB", Some(r"C:\Users\x\AppData\Local\DiscordPTB\Update.exe")),
    ] {
        os.app(n, p, path);
    }
    os.state().everything = EverythingStatus::Running { version: 1 };
    os.state().everything_files = vec![
        hit("Downloads", r"C:\Users\x", true, None),
        hit("Documents", r"C:\Users\x", true, None),
        hit("Screenshots", r"C:\Users\x\Pictures", true, None),
        hit("valorant_ace_2026-10-06.mp4", r"C:\Users\x\Videos", false, Some(88_298_291)),
        hit("Screenshot 2026-10-06 214512.png", r"C:\Users\x\Pictures\Screenshots", false, Some(1_468_006)),
        hit("invoice_september.pdf", r"C:\Users\x\Documents", false, Some(217_088)),
        hit("notes.txt", r"C:\Users\x\Desktop", false, Some(3_072)),
        hit("autoexec.cfg", r"C:\Games\cfg", false, Some(2_048)),
        hit("OBS-Studio-31.0.3-Windows-Installer.exe", r"C:\Users\x\Downloads", false, Some(149_540_000)),
        hit("DiscordSetup.exe", r"C:\Users\x\Downloads", false, Some(100_759_000)),
        hit("clips_backup.zip", r"D:\Backups", false, Some(1_932_735_283)),
    ];
    os
}

// ------------------------------------------------------------------------------------------------ nothing on open

#[test]
fn empty_field_asks_nothing_and_loads_nothing() {
    let os = sample();
    let s = svc(&os);
    for text in ["", "   ", "\t"] {
        for f in Filter::CHIPS {
            let r = s.search(&q(text, f), &Cancel::new()).unwrap();
            assert!(r.groups.is_empty() && r.is_empty() && r.note.is_none());
        }
    }
    let st = os.state();
    assert_eq!(st.apps_loads, 0, "the app list is not loaded until a word is typed");
    assert!(st.queries.is_empty());
    drop(st);
    assert_eq!(EMPTY_HINT.0, "Type to find apps, folders and files on this PC.");
    assert_eq!(EMPTY_HINT.1, "Only what is on this PC — no web results, no ads.");
}

#[test]
fn the_app_list_loads_once_and_release_drops_it() {
    let os = sample();
    let s = svc(&os);
    s.search(&q("disc", Filter::Apps), &Cancel::new()).unwrap();
    s.search(&q("steam", Filter::Apps), &Cancel::new()).unwrap();
    assert_eq!(os.state().apps_loads, 1);
    s.release();
    assert_eq!(os.state().everything_released, 1);
    s.search(&q("disc", Filter::Apps), &Cancel::new()).unwrap();
    assert_eq!(os.state().apps_loads, 2, "after release the next search loads it again");
}

// ------------------------------------------------------------------------------------------------ ranking

#[test]
fn ranking_starts_with_then_word_starts_then_anywhere_then_shorter() {
    let os = Arc::new(FakeOs::new());
    for n in ["My Discord Thing", "Discord PTB", "Discord", "Xdiscordx", "Pre-Discord", "Unrelated"] {
        os.app(n, n, None);
    }
    let r = svc(&os).search(&q("discord", Filter::Apps), &Cancel::new()).unwrap();
    // 0: Discord, Discord PTB (shorter first) · 1: Pre-Discord, My Discord Thing (word start; shorter first) · 2: Xdiscordx
    assert_eq!(names(group(&r, ItemKind::App)), vec!["Discord", "Discord PTB", "Pre-Discord", "My Discord Thing", "Xdiscordx"]);
}

#[test]
fn score_levels() {
    let w = |s: &str| s.split_whitespace().map(|x| x.to_lowercase()).collect::<Vec<_>>();
    assert_eq!(score("Discord", &w("disc"), "disc"), Some(0));
    assert_eq!(score("Riot Client", &w("client"), "client"), Some(1));
    assert_eq!(score("valorant_ace_2026.mp4", &w("ace"), "ace"), Some(1), "_ . - start a word");
    assert_eq!(score("Screenshot", &w("shot"), "shot"), Some(2));
    assert_eq!(score("Riot Client", &w("zzz"), "zzz"), None);
    // every word must be in the name
    assert_eq!(score("OBS Studio", &w("obs studio"), "obs studio"), Some(0));
    assert_eq!(score("OBS Studio", &w("studio obs"), "studio obs"), Some(1));
    assert_eq!(score("OBS Studio", &w("obs cat"), "obs cat"), None);
    // only the NAME counts: a word that is only in the folder never matches (backends are asked for names)
    assert_eq!(score("notes.txt", &w("desktop"), "desktop"), None);
}

#[test]
fn matching_ignores_case_and_every_word_must_be_in_the_name() {
    let os = sample();
    let r = svc(&os).search(&q("  OBS   studio ", Filter::All), &Cancel::new()).unwrap();
    assert_eq!(names(group(&r, ItemKind::App)), vec!["OBS Studio"]);
    let f = group(&r, ItemKind::File);
    assert_eq!(names(f), vec!["OBS-Studio-31.0.3-Windows-Installer.exe"]);
}

#[test]
fn rank_order_helper() {
    use std::cmp::Ordering::*;
    assert_eq!(rank_cmp((0, "zzz"), (1, "a")), Less);
    assert_eq!(rank_cmp((1, "ab"), (1, "abc")), Less);
    assert_eq!(rank_cmp((1, "Abc"), (1, "abd")), Less);
    assert_eq!(rank_cmp((1, "abc"), (1, "ABC")), Equal);
}

#[test]
fn matches_are_marked() {
    let w = |s: &str| s.split_whitespace().map(|x| x.to_lowercase()).collect::<Vec<_>>();
    assert_eq!(match_ranges("Discord PTB", &w("disc")), vec![(0, 4)]);
    assert_eq!(match_ranges("valorant_ace_ace", &w("ace")), vec![(9, 12), (13, 16)]);
    assert_eq!(match_ranges("OBS Studio", &w("obs stu")), vec![(0, 3), (4, 7)]);
    assert_eq!(match_ranges("abcabc", &w("abc bca")), vec![(0, 6)], "overlaps merge");
    assert_eq!(match_ranges("Straße", &w("ße")), vec![(4, 6)], "ranges are in characters, not bytes");
    assert!(match_ranges("abc", &w("zzz")).is_empty());
}

// ------------------------------------------------------------------------------------------------ groups, caps, chips

#[test]
fn all_groups_in_order_with_counts() {
    let os = sample();
    let r = svc(&os).search(&q("s", Filter::All), &Cancel::new()).unwrap();
    let titles: Vec<&str> = r.groups.iter().map(|g| g.title).collect();
    assert_eq!(titles, vec!["Apps", "Folders", "Files"]);
    assert_eq!(r.files_from, FilesFrom::Everything { version: 1 });
    assert!(r.note.is_none());
    // first result = first item of the first group (selected, with its Enter key cap)
    assert_eq!(r.flat()[0].kind, ItemKind::App);
}

#[test]
fn caps_4_4_6_under_all_and_show_all() {
    let os = Arc::new(FakeOs::new());
    for i in 0..9 {
        os.app(&format!("item app {i}"), &format!("a{i}"), None);
    }
    {
        let mut st = os.state();
        st.everything = EverythingStatus::Running { version: 1 };
        for i in 0..9 {
            st.everything_files.push(hit(&format!("item dir {i}"), r"C:\d", true, None));
            st.everything_files.push(hit(&format!("item file {i}.txt"), r"C:\d", false, Some(1)));
        }
    }
    let s = svc(&os);
    let r = s.search(&q("", Filter::All), &Cancel::new()).unwrap();
    assert!(r.groups.is_empty());
    let r = s.search(&q("item", Filter::All), &Cancel::new()).unwrap();
    let (a, d, f) = (group(&r, ItemKind::App), group(&r, ItemKind::Folder), group(&r, ItemKind::File));
    assert_eq!((a.items.len(), d.items.len(), f.items.len()), (4, 4, 6));
    assert_eq!((a.total, d.total, f.total), (9, 9, 9));
    assert_eq!((a.show_all(), d.show_all(), f.show_all()), (Some(9), Some(9), Some(9)));
    // one chip: up to 40 and no "Show all"
    let r = s.search(&q("item", Filter::Files), &Cancel::new()).unwrap();
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].items.len(), 9);
    assert_eq!(r.groups[0].show_all(), None);
    assert_eq!(r.groups[0].title, "Files");
}

#[test]
fn forty_rows_under_one_chip() {
    let os = Arc::new(FakeOs::new());
    for i in 0..55 {
        os.app(&format!("tool {i:02}"), &format!("t{i}"), None);
    }
    let r = svc(&os).search(&q("tool", Filter::Apps), &Cancel::new()).unwrap();
    let g = &r.groups[0];
    assert_eq!((g.items.len(), g.total, g.show_all()), (40, 55, Some(55)));
}

#[test]
fn chips_ask_only_what_they_need() {
    let os = sample();
    let s = svc(&os);
    s.search(&q("s", Filter::Apps), &Cancel::new()).unwrap();
    assert!(os.state().queries.is_empty(), "Apps asks no file backend");
    s.search(&q("s", Filter::Folders), &Cancel::new()).unwrap();
    {
        let st = os.state();
        assert_eq!(st.queries.len(), 1);
        assert!(st.queries[0].contains("folders"));
    }
    s.search(&q("s", Filter::Files), &Cancel::new()).unwrap();
    assert_eq!(os.state().queries.len(), 2);
    assert!(os.state().queries[1].contains("files") && os.state().queries[1].contains("ext=[]"));
    s.search(&q("s", Filter::All), &Cancel::new()).unwrap();
    assert_eq!(os.state().queries.len(), 4, "All asks folders and files, one query each");
}

#[test]
fn picture_video_document_chips() {
    let os = sample();
    let s = svc(&os);
    let r = s.search(&q("2026", Filter::Pictures), &Cancel::new()).unwrap();
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].title, "Pictures");
    assert_eq!(names(&r.groups[0]), vec!["Screenshot 2026-10-06 214512.png"]);
    let r = s.search(&q("2026", Filter::Videos), &Cancel::new()).unwrap();
    assert_eq!(names(&r.groups[0]), vec!["valorant_ace_2026-10-06.mp4"]);
    // Documents include text files (the drawing's notes.txt)
    let r = s.search(&q("n", Filter::Documents), &Cancel::new()).unwrap();
    let got = names(&r.groups[0]);
    assert!(got.contains(&"invoice_september.pdf") && got.contains(&"notes.txt"), "{got:?}");
    assert!(!got.contains(&"autoexec.cfg"), "a config file is not a document");
    // the extension list the backend got
    let st = os.state();
    assert!(st.queries.iter().any(|x| x.contains("\"png\"") && x.contains("\"jpg\"")), "{:?}", st.queries);
}

#[test]
fn file_types_by_extension() {
    for (n, t) in [
        ("a.PNG", FileType::Picture),
        ("a.mp4", FileType::Video),
        ("a.pdf", FileType::Document),
        ("notes.txt", FileType::Text),
        ("a.zip", FileType::Archive),
        ("setup.exe", FileType::Program),
        ("autoexec.cfg", FileType::Config),
        ("noext", FileType::Other),
        (".gitignore", FileType::Other),
        ("weird.xyz", FileType::Other),
    ] {
        assert_eq!(FileType::of_file_name(n), t, "{n}");
    }
    assert!(Filter::Pictures.extensions().contains(&"png"));
    assert!(Filter::Documents.extensions().contains(&"txt") && Filter::Documents.extensions().contains(&"pdf"));
    assert!(Filter::All.extensions().is_empty() && Filter::Files.extensions().is_empty());
    assert_eq!(Filter::CHIPS.map(|f| f.label()), ["All", "Apps", "Folders", "Files", "Pictures", "Videos", "Documents"]);
}

#[test]
fn rows_carry_what_the_page_shows() {
    let os = sample();
    let r = svc(&os).search(&q("valorant", Filter::All), &Cancel::new()).unwrap();
    let app = &group(&r, ItemKind::App).items[0];
    assert_eq!((app.name.as_str(), app.path.as_str()), ("VALORANT", r"C:\Riot Games\Riot Client\RiotClientServices.exe"));
    assert_eq!(app.size_text(), None);
    let f = &group(&r, ItemKind::File).items[0];
    assert_eq!(f.name, "valorant_ace_2026-10-06.mp4");
    assert_eq!(f.path, r"C:\Users\x\Videos\valorant_ace_2026-10-06.mp4");
    assert_eq!(f.size_text().as_deref(), Some("84.2 MB"));
    assert_eq!(f.date_text().as_deref(), Some("6 Oct 2026"));
    assert_eq!(f.file_type, Some(FileType::Video));
    assert_eq!(f.parent_dir().as_deref(), Some(r"C:\Users\x\Videos"));
}

#[test]
fn sizes_and_counts_read_like_the_drawing() {
    assert_eq!(format_size(2_048), "2 KB");
    assert_eq!(format_size(3_072), "3 KB");
    assert_eq!(format_size(217_088), "212 KB");
    assert_eq!(format_size(1_468_006), "1.4 MB");
    assert_eq!(format_size(149_540_000), "142.6 MB");
    assert_eq!(format_size(1_932_735_283), "1.8 GB");
    assert_eq!(format_size(10), "10 B");
    assert_eq!(format_size(1_000), "1000 B");
    assert_eq!(format_size(1_100), "1 KB");
    assert_eq!(items_text(1), "1 item");
    assert_eq!(items_text(214), "214 items");
    assert_eq!(none_text("  zzz "), "Nothing on this PC matches “zzz”");
}

#[test]
fn no_hit_is_an_empty_result_not_an_error() {
    let os = sample();
    let r = svc(&os).search(&q("qqqqzzzz", Filter::All), &Cancel::new()).unwrap();
    assert!(r.is_empty() && r.note.is_none());
    assert_eq!(r.files_from, FilesFrom::Everything { version: 1 });
}

// ------------------------------------------------------------------------------------------------ the sources

#[test]
fn a_looser_index_match_is_filtered_and_duplicates_dropped() {
    let os = sample();
    {
        let mut st = os.state();
        // an index may return a looser match than the words (wildcards): the name must really contain them
        st.everything_files.push(hit("zzz.txt", r"C:\x", false, Some(1)));
        st.everything_files.push(hit("notes.txt", r"C:\Users\x\Desktop", false, Some(3_072))); // same path twice
    }
    let r = svc(&os).search(&q("notes", Filter::Files), &Cancel::new()).unwrap();
    assert_eq!(names(&r.groups[0]), vec!["notes.txt"]);
}

#[test]
fn everything_is_preferred_when_it_runs() {
    let os = sample();
    {
        let mut st = os.state();
        st.everything = EverythingStatus::Running { version: 1 };
        st.everything_files = vec![hit("everything_hit.txt", r"E:\d", false, Some(10)), hit("EvDir", r"E:\d", true, None)];
    }
    let s = svc(&os);
    let rep = s.backend_report();
    assert_eq!(rep.files_from, FilesFrom::Everything { version: 1 });
    assert_eq!(rep.scope, None, "the folder list is only for Windows Search");
    let r = s.search(&q("e", Filter::All), &Cancel::new()).unwrap();
    assert_eq!(r.files_from, FilesFrom::Everything { version: 1 });
    assert!(os.state().queries.iter().all(|x| x.starts_with("everything")), "{:?}", os.state().queries);
    assert_eq!(names(group(&r, ItemKind::File)), vec!["everything_hit.txt"]);
}

#[test]
fn no_other_engine_when_everything_is_not_ready() {
    // the owner, Oct 8: Everything is the one engine - not installed / not running / loading = no file results and a note,
    // never Windows Search
    for (st_, note) in [
        (EverythingStatus::NotInstalled, SearchError::EverythingNotInstalled),
        (EverythingStatus::Loading { building: false }, SearchError::EverythingLoading),
        (EverythingStatus::NotRunning, SearchError::Everything("Everything is not running".into())),
    ] {
        let os = sample();
        os.state().everything = st_;
        let s = svc(&os);
        assert_eq!(s.backend_report().files_from, FilesFrom::Nothing, "{st_:?}");
        let r = s.search(&q("disc", Filter::All), &Cancel::new()).unwrap();
        assert_eq!(r.files_from, FilesFrom::Nothing);
        assert_eq!(r.note, Some(note.clone()));
        assert_eq!(names(group(&r, ItemKind::App)), vec!["Discord", "Discord PTB"], "apps still show");
        assert!(r.groups.iter().all(|g| g.kind == ItemKind::App));
        assert!(os.state().queries.is_empty(), "no backend was asked");
        let r = s.search(&q("disc", Filter::Files), &Cancel::new()).unwrap();
        assert!(r.is_empty());
        assert_eq!(r.note, Some(note));
    }
}

#[test]
fn the_engine_starts_with_the_tab_and_only_ours_stops() {
    let os = sample();
    os.state().everything = EverythingStatus::NotRunning;
    os.state().everything_starts_as = EverythingStatus::Loading { building: true };
    let s = svc(&os);
    s.start_engine().unwrap();
    assert_eq!(s.engine(), EverythingStatus::Loading { building: true }, "building its file list first");
    s.start_engine().unwrap();
    assert_eq!(os.state().actions, vec!["start everything".to_string()], "started once");
    os.state().everything = EverythingStatus::Running { version: 1 };
    s.release();
    assert_eq!(s.engine(), EverythingStatus::NotRunning, "ours quits with the menu");
    assert_eq!(os.state().actions.last().unwrap(), "stop everything");
    // a copy the user runs: used, never started or stopped
    let os = sample();
    let s = svc(&os);
    s.start_engine().unwrap();
    s.release();
    assert!(os.state().actions.is_empty(), "{:?}", os.state().actions);
    assert_eq!(s.engine(), EverythingStatus::Running { version: 1 });
}

#[test]
fn not_installed_offers_the_install() {
    let os = sample();
    os.state().everything = EverythingStatus::NotInstalled;
    let s = svc(&os);
    assert_eq!(s.start_engine(), Err(SearchError::EverythingNotInstalled));
    os.state().install_result = Err(SearchError::InstallCancelled);
    assert_eq!(s.install_engine(), Err(SearchError::InstallCancelled));
    assert_eq!(s.engine(), EverythingStatus::NotInstalled, "cancelled = nothing changed");
    os.state().install_result = Ok(());
    s.install_engine().unwrap();
    s.start_engine().unwrap();
    assert_eq!(os.state().actions, vec!["install everything", "install everything", "start everything"]);
    assert_eq!(s.engine(), EverythingStatus::Running { version: 1 });
}

#[test]
fn the_type_picker_keeps_files_of_one_extension() {
    let os = sample();
    let r = svc(&os).search(&q("o", Filter::All).with_ext(Some(".TXT")), &Cancel::new()).unwrap();
    assert_eq!(r.groups.len(), 1, "files only: no apps, no folders");
    assert_eq!(names(&r.groups[0]), vec!["notes.txt"]);
    assert!(os.state().queries.iter().all(|x| x.contains("files") && x.contains("ext=[\"txt\"]")), "{:?}", os.state().queries);
    assert_eq!(q("x", Filter::All).with_ext(Some("")).ext, None);
}

#[test]
fn a_source_failing_midway_keeps_the_apps() {
    let os = sample();
    os.state().everything_error = Some(SearchError::Os { call: "Everything query".into(), code: 0x80040e14, text: "bad".into() });
    let r = svc(&os).search(&q("disc", Filter::All), &Cancel::new()).unwrap();
    assert_eq!(r.files_from, FilesFrom::Nothing);
    assert!(matches!(r.note, Some(SearchError::Os { code: 0x80040e14, .. })));
    assert_eq!(names(group(&r, ItemKind::App)), vec!["Discord", "Discord PTB"]);
}

#[test]
fn many_candidates_report_the_real_total_or_a_lower_bound() {
    let os = Arc::new(FakeOs::new());
    {
        let mut st = os.state();
        st.everything = EverythingStatus::Running { version: 1 };
        for i in 0..(MAX_CANDIDATES + 50) {
            st.everything_files.push(hit(&format!("log{i:04}.txt"), r"C:\l", false, Some(1)));
        }
    }
    let r = svc(&os).search(&q("log", Filter::Files), &Cancel::new()).unwrap();
    let g = &r.groups[0];
    assert_eq!(g.items.len(), 40);
    assert_eq!(g.total, MAX_CANDIDATES + 50, "Everything knows the full count");
    assert!(!g.total_is_lower_bound);
}

#[test]
fn a_cancelled_search_stops_between_steps() {
    let os = sample();
    let c = Cancel::new();
    c.cancel();
    assert_eq!(svc(&os).search(&q("disc", Filter::All), &c).unwrap_err(), SearchError::Cancelled);
    assert!(os.state().queries.is_empty(), "no file query after the cancel");
    assert_eq!(svc(&os).search(&q("disc", Filter::Files), &c).unwrap_err(), SearchError::Cancelled);
}

#[test]
fn the_backend_text_for_each_source() {
    let fq = FileQuery { words: vec!["valorant".into(), "ace".into()], folders: false, extensions: vec!["mp4".into(), "mkv".into()], max: 300 };
    assert_eq!(fq.everything_text(), "file: \"valorant\" \"ace\" ext:mp4;mkv");
    let fq2 = FileQuery { words: vec!["o'brien".into(), "a\"b".into()], folders: true, extensions: vec![], max: 50 };
    assert_eq!(fq2.everything_text(), "folder: \"o'brien\" \"ab\"");
    let sql = fq.windows_search_sql();
    assert!(sql.starts_with("SELECT TOP 300 System.ItemNameDisplay, System.ItemPathDisplay, System.Size, System.DateModified FROM SystemIndex"));
    assert!(sql.contains("SCOPE='file:'") && sql.contains("System.ItemType <> 'Directory'"));
    assert!(sql.contains("System.ItemNameDisplay LIKE '%valorant%' AND System.ItemNameDisplay LIKE '%ace%'"));
    assert!(sql.ends_with("AND (System.FileExtension = '.mp4' OR System.FileExtension = '.mkv')"));
    let sql2 = fq2.windows_search_sql();
    assert!(sql2.contains("System.ItemType = 'Directory'"));
    assert!(sql2.contains("LIKE '%o''brien%'"), "a quote in the text cannot break out of the string: {sql2}");
}

#[test]
fn query_words() {
    assert_eq!(q("  Foo   BAR ", Filter::All).words(), vec!["foo", "bar"]);
    assert!(q("   ", Filter::All).words().is_empty());
}

// ------------------------------------------------------------------------------------------------ the menu and open

#[test]
fn menu_entries_per_kind() {
    let os = sample();
    let s = svc(&os);
    let r = s.search(&q("s", Filter::All), &Cancel::new()).unwrap();
    let steam = &group(&r, ItemKind::App).items.iter().find(|i| i.name == "Steam").unwrap().clone();
    let folder = group(&r, ItemKind::Folder).items[0].clone();
    let file = group(&r, ItemKind::File).items[0].clone();
    assert_eq!(s.menu_for(steam), vec![MenuAction::OpenFileLocation, MenuAction::CopyPath]);
    assert_eq!(s.menu_for(&folder), vec![MenuAction::OpenFileLocation, MenuAction::CopyPath]);
    assert_eq!(s.menu_for(&file), vec![MenuAction::OpenFileLocation, MenuAction::CopyPath, MenuAction::OpenWith]);
    assert_eq!(MenuAction::OpenWith.label(), "Open with…");
    assert_eq!(s.menu_header(steam), r"C:\Program Files (x86)\Steam\steam.exe"); // the full path on top

    // a Store app has no file location
    let r = s.search(&q("notepad", Filter::Apps), &Cancel::new()).unwrap();
    let np = r.groups[0].items[0].clone();
    assert!(s.menu_for(&np).is_empty());
    assert!(matches!(s.run_menu(&np, MenuAction::CopyPath), Err(SearchError::Unsupported(_))));
    assert!(os.state().actions.is_empty());
}

#[test]
fn open_and_menu_actions_reach_windows() {
    let os = sample();
    let s = svc(&os);
    let r = s.search(&q("valorant", Filter::All), &Cancel::new()).unwrap();
    let app = group(&r, ItemKind::App).items[0].clone();
    let file = group(&r, ItemKind::File).items[0].clone();
    s.open(&app).unwrap();
    s.open(&file).unwrap();
    s.run_menu(&file, MenuAction::OpenFileLocation).unwrap();
    s.run_menu(&file, MenuAction::CopyPath).unwrap();
    s.run_menu(&file, MenuAction::OpenWith).unwrap();
    assert_eq!(
        os.state().actions,
        vec![
            "open app VALORANT".to_string(),
            r"open C:\Users\x\Videos\valorant_ace_2026-10-06.mp4".to_string(),
            r"reveal C:\Users\x\Videos\valorant_ace_2026-10-06.mp4".to_string(),
            r"copy C:\Users\x\Videos\valorant_ace_2026-10-06.mp4".to_string(),
            r"open with C:\Users\x\Videos\valorant_ace_2026-10-06.mp4".to_string(),
        ]
    );
    // Open with… is for files only
    let dir = svc(&os).search(&q("downloads", Filter::Folders), &Cancel::new()).unwrap().groups[0].items[0].clone();
    assert!(matches!(s.run_menu(&dir, MenuAction::OpenWith), Err(SearchError::Unsupported(_))));
    // an OS refusal reaches the caller
    os.state().action_error = Some(SearchError::Os { call: "ShellExecute".into(), code: 2, text: "not found".into() });
    assert!(matches!(s.open(&file), Err(SearchError::Os { code: 2, .. })));
}

#[test]
fn folder_rows_show_how_many_items() {
    let os = sample();
    os.state().counts.push((r"C:\Users\x\Downloads".into(), 214));
    let s = svc(&os);
    let r = s.search(&q("downloads", Filter::Folders), &Cancel::new()).unwrap();
    let d = &r.groups[0].items[0];
    assert_eq!(s.folder_item_count(d), Some(214));
    assert_eq!(items_text(s.folder_item_count(d).unwrap()), "214 items");
    // not asked for files / apps
    let f = s.search(&q("notes", Filter::Files), &Cancel::new()).unwrap().groups[0].items[0].clone();
    assert_eq!(s.folder_item_count(&f), None);
}

#[test]
fn typed_like_characters_are_literal_in_the_windows_search_sql() {
    assert_eq!(sql_like_word("[1080p]"), "[[]1080p]");
    assert_eq!(sql_like_word("50%_off"), "50[%][_]off");
    assert_eq!(sql_like_word("o'brien"), "o''brien");
    let fq = FileQuery { words: vec!["[1080p]".into(), "a_b".into()], folders: false, extensions: vec![], max: 300 };
    let sql = fq.windows_search_sql();
    assert!(sql.contains("LIKE '%[[]1080p]%'") && sql.contains("LIKE '%a[_]b%'"), "{sql}");
}
