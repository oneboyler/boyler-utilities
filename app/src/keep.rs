//! What the pages remember while the app runs (the owner Oct 8, feedback F2: "it should work in the background until its done
//! then remember the last result" - closing the window or leaving a tab must not stop or wipe work).
//!
//! The menu window and its pages come and go (`Page::close` drops a page's service, the menu drops every page); this store
//! does not - the app holds ONE for its whole life and hands it to every page as `env.keep` (`Env::default()` = a fresh,
//! empty one, so each unit test has its own).
//!
//! How a page uses it (the F2 pattern):
//! - slow work (a scan, a speed test, pings) runs as a JOB (`cx.start_job(key, work)`, from the user's click): the job
//!   runner lives as long as the app, so the work goes on with the tab left or the window closed. The job's closure owns
//!   what it needs (its own crate service / OS layer - never the page's, which goes with the page) and a clone of
//!   `env.keep`; it writes its result there (`keep.put(key, value)`, also partial results while it runs) and returns;
//! - live data that the page and the job share while it runs: put an `Arc<Mutex<..>>` in (`get` hands back a clone of
//!   the Arc);
//! - when the tab is shown again: `cx.job(key)` (still running -> its progress) and `keep.get_at(key)` (the last result +
//!   when it was made: "Scanned 5 min ago" via [`ago`]). The page shows it at once - nothing is redone on its own.
//!
//! Memory only (not written to disk): the app's quit forgets it. Values are small results, not raw data.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

type Slot = (Box<dyn Any + Send + Sync>, SystemTime);

/// The store (cheap to clone: one shared map; Send + Sync, so a job's thread can hold it).
#[derive(Clone, Default)]
pub struct Keep(Arc<Mutex<HashMap<String, Slot>>>);

impl std::fmt::Debug for Keep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.0.lock().map(|m| m.len()).unwrap_or(0);
        write!(f, "Keep({n} kept)")
    }
}

/// The app's one store (the menu's pages and the pages' background parts share it).
pub fn app() -> Keep {
    static APP: OnceLock<Keep> = OnceLock::new();
    APP.get_or_init(Keep::default).clone()
}

impl Keep {
    /// Remember `value` under `key` (replaces the old one), stamped now.
    pub fn put<T: Any + Send + Sync + Clone>(&self, key: &str, value: T) {
        self.put_at(key, value, SystemTime::now());
    }

    /// Remember `value`, stamped `at` (e.g. when the scan STARTED, if that is the time to show).
    pub fn put_at<T: Any + Send + Sync + Clone>(&self, key: &str, value: T, at: SystemTime) {
        if let Ok(mut m) = self.0.lock() {
            m.insert(key.to_string(), (Box::new(value), at));
        }
    }

    /// The value under `key` (None = nothing kept, or kept as another type).
    pub fn get<T: Any + Send + Sync + Clone>(&self, key: &str) -> Option<T> {
        self.get_at(key).map(|(v, _)| v)
    }

    /// The value and when it was put.
    pub fn get_at<T: Any + Send + Sync + Clone>(&self, key: &str) -> Option<(T, SystemTime)> {
        let m = self.0.lock().ok()?;
        let (b, at) = m.get(key)?;
        b.downcast_ref::<T>().map(|v| (v.clone(), *at))
    }

    /// When `key` was last put.
    pub fn at(&self, key: &str) -> Option<SystemTime> {
        self.0.lock().ok()?.get(key).map(|s| s.1)
    }

    /// Change the kept value in place (nothing kept = `f` is not called); the stamp stays.
    pub fn update<T: Any + Send + Sync + Clone>(&self, key: &str, f: impl FnOnce(&mut T)) -> bool {
        let Ok(mut m) = self.0.lock() else { return false };
        match m.get_mut(key).and_then(|s| s.0.downcast_mut::<T>()) {
            Some(v) => {
                f(v);
                true
            }
            None => false,
        }
    }

    pub fn has(&self, key: &str) -> bool {
        self.0.lock().map(|m| m.contains_key(key)).unwrap_or(false)
    }

    /// Forget `key` (e.g. the user cleared the result).
    pub fn remove(&self, key: &str) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(key);
        }
    }
}

/// How long ago `at` was, as the pages write it: "just now", "1 min ago", "25 min ago", "3 h ago", "2 days ago".
pub fn ago(at: SystemTime) -> String {
    ago_from(at, SystemTime::now())
}

pub fn ago_from(at: SystemTime, now: SystemTime) -> String {
    let s = now.duration_since(at).unwrap_or(Duration::ZERO).as_secs();
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        86_400..=172_799 => "1 day ago".into(),
        _ => format!("{} days ago", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_survives_its_page_and_a_job_thread_can_write_it() {
        let k = Keep::default();
        assert_eq!(k.get::<u32>("sto.scan"), None);
        let k2 = k.clone();
        std::thread::spawn(move || k2.put("sto.scan", 42u32)).join().unwrap();
        assert_eq!(k.get::<u32>("sto.scan"), Some(42));
        // another type under the same key: not mixed up
        assert_eq!(k.get::<String>("sto.scan"), None);
        assert!(k.update::<u32>("sto.scan", |v| *v += 1));
        assert_eq!(k.get::<u32>("sto.scan"), Some(43));
        k.remove("sto.scan");
        assert!(!k.has("sto.scan"));
        // shared live data: the Arc comes back
        let live = Arc::new(Mutex::new(vec![1]));
        k.put("net.pings", live.clone());
        live.lock().unwrap().push(2);
        assert_eq!(*k.get::<Arc<Mutex<Vec<i32>>>>("net.pings").unwrap().lock().unwrap(), vec![1, 2]);
    }

    #[test]
    fn each_default_store_is_its_own_and_the_app_store_is_one() {
        Keep::default().put("x", 1u8);
        assert!(!Keep::default().has("x"));
        app().put("keep.test.app", 1u8);
        assert!(app().has("keep.test.app"));
        app().remove("keep.test.app");
    }

    #[test]
    fn ago_reads_like_the_pages_write_it() {
        let now = SystemTime::now();
        let t = |s: u64| ago_from(now - Duration::from_secs(s), now);
        assert_eq!(t(5), "just now");
        assert_eq!(t(60), "1 min ago");
        assert_eq!(t(25 * 60 + 10), "25 min ago");
        assert_eq!(t(3 * 3600), "3 h ago");
        assert_eq!(t(86_400 + 5), "1 day ago");
        assert_eq!(t(3 * 86_400), "3 days ago");
        assert_eq!(ago_from(now + Duration::from_secs(9), now), "just now");
    }
}
