//! Which exes count as games: an exe inside a game launcher's install folder (found read-only by [`crate::ActivityOs`]:
//! Steam libraries' `steamapps\common`, Epic manifests' install folders, `Riot Games\<game>`, Ubisoft and GOG install
//! folders from their registry keys, `XboxGames\<game>` on every fixed drive), unless the user's right-click says otherwise
//! ("Count as a game" / "Not a game" wins). Launchers themselves are not inside those folders (Steam's own exe is not in
//! `steamapps\common`; `Riot Games\Riot Client` is left out), so they don't count.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameRules {
    /// Install folders, lower case, ending in `\`.
    pub roots: Vec<String>,
    /// exe path (lower case) → the user's right-click choice.
    pub overrides: BTreeMap<String, bool>,
}

/// An exe path as a key: lower case, forward slashes turned back.
pub fn key_of(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

/// A folder as a root: lower case, with one trailing `\`.
pub fn root_of(folder: &str) -> String {
    let mut r = key_of(folder.trim());
    while r.ends_with('\\') {
        r.pop();
    }
    r.push('\\');
    r
}

impl GameRules {
    pub fn new(roots: impl IntoIterator<Item = String>) -> Self {
        let mut roots: Vec<String> = roots.into_iter().filter(|r| r.trim().len() > 3).map(|r| root_of(&r)).collect();
        roots.sort();
        roots.dedup();
        GameRules { roots, overrides: BTreeMap::new() }
    }

    /// Found by a launcher's folder (no right-click).
    pub fn auto(&self, key: &str) -> bool {
        let k = key_of(key);
        self.roots.iter().any(|r| k.starts_with(r.as_str()))
    }

    pub fn is_game(&self, key: &str) -> bool {
        self.overrides.get(&key_of(key)).copied().unwrap_or_else(|| self.auto(key))
    }
}
