//! Finding the program inside a command line: a quoted first part, or — for an unquoted path with spaces (e.g.
//! `C:\Riot Games\Riot Client\RiotClientServices.exe --launch-background-mode`) — the shortest run of words that names an existing
//! file, trying `.exe` too. That is how CreateProcessW resolves an unquoted path with spaces ("c:\program.exe", "c:\program
//! files\sub.exe", … — Microsoft Learn, CreateProcessW, lpCommandLine remarks).

use std::path::PathBuf;

/// `(program, arguments)`. `exists` says whether a path names a file; `expand` expands `%VAR%`.
pub fn split_command(cmd: &str, exists: &dyn Fn(&str) -> bool, expand: &dyn Fn(&str) -> String) -> Option<(PathBuf, String)> {
    let cmd = expand(cmd.trim());
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return None;
    }
    if let Some(rest) = cmd.strip_prefix('"') {
        let end = rest.find('"')?;
        let prog = &rest[..end];
        if prog.is_empty() {
            return None;
        }
        return Some((PathBuf::from(prog), rest[end + 1..].trim().to_string()));
    }
    let words: Vec<&str> = cmd.split(' ').collect();
    for i in 1..=words.len() {
        let candidate = words[..i].join(" ");
        let with_exe = format!("{candidate}.exe");
        let args = words[i..].join(" ").trim().to_string();
        if exists(&candidate) {
            return Some((PathBuf::from(candidate), args));
        }
        if !candidate.to_ascii_lowercase().ends_with(".exe") && exists(&with_exe) {
            return Some((PathBuf::from(with_exe), args));
        }
        // Own rule (not CreateProcess's): a word ending in .exe ends the program even if that file is missing, so a stale entry
        // still shows the path it names.
        if candidate.to_ascii_lowercase().ends_with(".exe") {
            return Some((PathBuf::from(candidate), args));
        }
    }
    // Nothing exists: the first word is the program.
    let first = words[0];
    Some((PathBuf::from(first), words[1..].join(" ").trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exists_in(list: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |p: &str| list.iter().any(|x| x.eq_ignore_ascii_case(p))
    }
    fn no_expand(s: &str) -> String {
        s.to_string()
    }

    #[test]
    fn quoted() {
        let e = exists_in(&[]);
        let (p, a) = split_command(r#""C:\Program Files (x86)\Steam\steam.exe" -silent"#, &e, &no_expand).unwrap();
        assert_eq!(p, PathBuf::from(r"C:\Program Files (x86)\Steam\steam.exe"));
        assert_eq!(a, "-silent");
    }

    #[test]
    fn unquoted_with_spaces() {
        let e = exists_in(&[r"C:\Riot Games\Riot Client\RiotClientServices.exe"]);
        let (p, a) =
            split_command(r"C:\Riot Games\Riot Client\RiotClientServices.exe --launch-background-mode", &e, &no_expand).unwrap();
        assert_eq!(p, PathBuf::from(r"C:\Riot Games\Riot Client\RiotClientServices.exe"));
        assert_eq!(a, "--launch-background-mode");
        // Missing file: still stops at the word ending in .exe.
        let none = exists_in(&[]);
        let (p, _) = split_command(r"C:\Riot Games\Riot Client\RiotClientServices.exe -x", &none, &no_expand).unwrap();
        assert_eq!(p, PathBuf::from(r"C:\Riot Games\Riot Client\RiotClientServices.exe"));
    }

    #[test]
    fn exe_suffix_added_and_env_expanded() {
        let e = exists_in(&[r"C:\Windows\system32\svchost.exe"]);
        let expand = |s: &str| s.replace("%SystemRoot%", r"C:\Windows");
        let (p, a) = split_command(r"%SystemRoot%\system32\svchost -k netsvcs -p", &e, &expand).unwrap();
        assert_eq!(p, PathBuf::from(r"C:\Windows\system32\svchost.exe"));
        assert_eq!(a, "-k netsvcs -p");
    }

    #[test]
    fn empty_is_none() {
        let e = exists_in(&[]);
        assert!(split_command("", &e, &no_expand).is_none());
        assert!(split_command("   ", &e, &no_expand).is_none());
        assert!(split_command("\"\"", &e, &no_expand).is_none());
    }
}
