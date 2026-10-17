//! Shell-style wildcards for configuration includes (`Include conf.d/*.conf`,
//! nginx `include sites-enabled/*`) and log archive patterns
//! (`/var/log/httpd/example.com-access_log-*`).
//!
//! `*` and `?` never cross a `/`, `[abc]`, `[a-z]` and `[!x]` / `[^x]` are
//! character classes, and a wildcard never matches a leading `.` (hidden
//! files) unless the pattern component itself starts with one. Matches are
//! returned sorted, which is the order Apache and nginx include files in.

use std::path::{Component, Path, PathBuf};

/// Upper bound of the paths one pattern may expand to.
pub const MAX_MATCHES: usize = 10_000;

/// True when `s` contains a wildcard character.
pub fn has_magic(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

/// `fnmatch()` of one path component.
pub fn fnmatch(pat: &str, name: &str) -> bool {
    let (p, n) = (pat.as_bytes(), name.as_bytes());
    if n.first() == Some(&b'.') && p.first() != Some(&b'.') {
        return false;
    }
    matches(p, n)
}

fn matches(p: &[u8], n: &[u8]) -> bool {
    let (mut pi, mut ni) = (0usize, 0usize);
    // backtracking point of the last `*`: (pattern index after it, name index)
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() {
            match p[pi] {
                b'*' => {
                    star = Some((pi + 1, ni));
                    pi += 1;
                    continue;
                }
                b'?' => {
                    pi += 1;
                    ni += 1;
                    continue;
                }
                b'[' => match class(&p[pi..], n[ni]) {
                    Some((true, len)) => {
                        pi += len;
                        ni += 1;
                        continue;
                    }
                    Some((false, _)) => {}
                    // an unterminated `[` is a literal
                    None if n[ni] == b'[' => {
                        pi += 1;
                        ni += 1;
                        continue;
                    }
                    None => {}
                },
                b'\\' if pi + 1 < p.len() && p[pi + 1] == n[ni] => {
                    pi += 2;
                    ni += 1;
                    continue;
                }
                c if c == n[ni] => {
                    pi += 1;
                    ni += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star {
            Some((sp, sn)) => {
                pi = sp;
                ni = sn + 1;
                star = Some((sp, sn + 1));
            }
            None => return false,
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

/// Matches `c` against the class at the start of `p` (`[...]`); `(matched,
/// length of the class)`, `None` when the class is not terminated.
fn class(p: &[u8], c: u8) -> Option<(bool, usize)> {
    let mut i = 1;
    let neg = matches!(p.get(i), Some(b'!') | Some(b'^'));
    if neg {
        i += 1;
    }
    let mut ok = false;
    let mut first = true;
    while i < p.len() {
        let b = p[i];
        if b == b']' && !first {
            return Some((ok != neg, i + 1));
        }
        first = false;
        if i + 2 < p.len() && p[i + 1] == b'-' && p[i + 2] != b']' {
            if b <= c && c <= p[i + 2] {
                ok = true;
            }
            i += 3;
        } else {
            if b == c {
                ok = true;
            }
            i += 1;
        }
    }
    None
}

/// Expands a pattern into the existing paths it names, sorted; a pattern without
/// wildcards yields itself when it exists. Relative patterns are resolved against
/// `base`.
pub fn expand(pattern: &Path, base: &Path) -> Vec<PathBuf> {
    let full = if pattern.is_absolute() {
        pattern.to_path_buf()
    } else {
        base.join(pattern)
    };
    let mut cur: Vec<PathBuf> = vec![PathBuf::new()];
    for comp in full.components() {
        let part = match comp {
            Component::RootDir => {
                cur.iter_mut().for_each(|p| p.push("/"));
                continue;
            }
            Component::CurDir => continue,
            Component::ParentDir => "..".to_string(),
            Component::Normal(s) => s.to_string_lossy().into_owned(),
            Component::Prefix(_) => continue,
        };
        let mut next = Vec::new();
        for dir in &cur {
            if !has_magic(&part) {
                next.push(dir.join(&part));
                continue;
            }
            let d = if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir.as_path()
            };
            for name in crate::util::fs::scandir(d) {
                if fnmatch(&part, &name) {
                    next.push(dir.join(&name));
                    if next.len() >= MAX_MATCHES {
                        break;
                    }
                }
            }
        }
        cur = next;
        if cur.is_empty() {
            return cur;
        }
    }
    let mut out: Vec<PathBuf> = cur
        .into_iter()
        .filter(|p| std::fs::symlink_metadata(p).is_ok())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert!(fnmatch("*.conf", "ssl.conf"));
        assert!(!fnmatch("*.conf", "ssl.conf.bak"));
        assert!(!fnmatch("*.conf", ".hidden.conf"));
        assert!(fnmatch(".*", ".hidden"));
        assert!(fnmatch("access_log-*", "access_log-20261017"));
        assert!(fnmatch("access_log.?", "access_log.1"));
        assert!(!fnmatch("access_log.?", "access_log.10"));
        assert!(fnmatch("a[0-9]b", "a5b"));
        assert!(!fnmatch("a[!0-9]b", "a5b"));
        assert!(fnmatch("a[!0-9]b", "axb"));
        assert!(fnmatch("*", "x"));
        assert!(fnmatch("a*b*c", "aXbYbZc"));
        assert!(!fnmatch("a*b*c", "aXbYbZ"));
        assert!(fnmatch("[", "["));
        assert!(has_magic("conf.d/*.conf") && !has_magic("conf/httpd.conf"));
    }

    #[test]
    fn expansion() {
        let dir = std::env::temp_dir().join(format!("wp-glob-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("conf.d")).unwrap();
        for f in ["b.conf", "a.conf", "c.txt", ".x.conf"] {
            std::fs::write(dir.join("conf.d").join(f), "").unwrap();
        }
        let got = expand(Path::new("conf.d/*.conf"), &dir);
        assert_eq!(
            got,
            vec![dir.join("conf.d/a.conf"), dir.join("conf.d/b.conf")]
        );
        assert_eq!(expand(&dir.join("conf.d/c.txt"), Path::new("/")).len(), 1);
        assert!(expand(&dir.join("nope/*.conf"), Path::new("/")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
