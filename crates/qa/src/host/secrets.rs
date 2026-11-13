//! Per-app secrets: `apps/<id>/secrets`, outside the app's data folder, so
//! file imports can't reach it. One `key<TAB>value` line per secret, with
//! `\`, tab and newline escaped. The file is mode 0600 but not encrypted.

use qa_proto::err;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const MAX_KEY: usize = 64;
const MAX_VALUE: usize = 64 << 10;
const MAX_COUNT: usize = 100;

fn valid_key(k: &str) -> bool {
    !k.is_empty() && k.len() <= MAX_KEY && k.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

fn load(path: &Path) -> Vec<(String, String)> {
    let s = std::fs::read_to_string(path).unwrap_or_default();
    s.lines().filter_map(|l| l.split_once('\t')).map(|(k, v)| (k.to_string(), unescape(v))).collect()
}

pub fn get(path: &Path, key: &str) -> Result<String, i32> {
    if !valid_key(key) {
        return Err(err::INVALID);
    }
    load(path).into_iter().find(|(k, _)| k == key).map(|(_, v)| v).ok_or(err::NOT_FOUND)
}

/// Stores `value` under `key`; an empty value removes it.
pub fn set(path: &Path, key: &str, value: &str) -> i32 {
    if !valid_key(key) || value.len() > MAX_VALUE {
        return err::INVALID;
    }
    let mut all = load(path);
    all.retain(|(k, _)| k != key);
    if !value.is_empty() {
        if all.len() >= MAX_COUNT {
            return err::LIMIT;
        }
        all.push((key.to_string(), value.to_string()));
    }
    let mut s = String::new();
    for (k, v) in &all {
        s.push_str(k);
        s.push('\t');
        s.push_str(&escape(v));
        s.push('\n');
    }
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let ok = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| f.write_all(s.as_bytes()).and_then(|_| f.sync_all()))
        .and_then(|_| std::fs::rename(&tmp, path));
    if ok.is_ok() {
        0
    } else {
        err::IO
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let p = std::env::temp_dir().join(format!("qa-secrets-{}", std::process::id()));
        assert_eq!(super::get(&p, "token"), Err(qa_proto::err::NOT_FOUND));
        assert_eq!(super::set(&p, "token", "a\tb\\n\nc"), 0);
        assert_eq!(super::set(&p, "other", "x"), 0);
        assert_eq!(super::get(&p, "token").unwrap(), "a\tb\\n\nc");
        assert_eq!(super::set(&p, "token", ""), 0);
        assert_eq!(super::get(&p, "token"), Err(qa_proto::err::NOT_FOUND));
        assert_eq!(super::set(&p, "../x", "y"), qa_proto::err::INVALID);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_file(&p);
    }
}
