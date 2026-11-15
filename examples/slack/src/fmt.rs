//! Slack mrkdwn → rich text, the reverse for outgoing messages, and
//! timestamps.

use alloc::collections::BTreeMap;
use qa_sdk::*;

pub type Names = BTreeMap<String, String>;

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// A message as rich text, plus the URL of each link span (by span index).
pub struct Text {
    pub rich: Rich,
    pub links: Vec<(u32, String)>,
}

/// Resolves `<…>` references and entities, styles `*bold*` and `` `code` ``,
/// and turns links into link spans.
pub fn render(raw: &str, users: &Names, chans: &Names) -> Text {
    let mut t = Text { rich: Rich::new(), links: Vec::new() };
    let mut rest = raw;
    while let Some(i) = rest.find('<') {
        plain(&mut t.rich, &unescape(&rest[..i]));
        let Some(j) = rest[i..].find('>') else {
            rest = &rest[i..];
            break;
        };
        let inner = &rest[i + 1..i + j];
        rest = &rest[i + j + 1..];
        let (target, label) = match inner.split_once('|') {
            Some((t, l)) => (t, Some(unescape(l))),
            None => (inner, None),
        };
        if let Some(id) = target.strip_prefix('@') {
            let name = label.unwrap_or_else(|| users.get(id).cloned().unwrap_or_else(|| String::from(id)));
            t.rich.styled(&cat(&["@", name.trim_start_matches('@')]), span::BOLD, "accent");
        } else if let Some(id) = target.strip_prefix('#') {
            let name = label.unwrap_or_else(|| chans.get(id).cloned().unwrap_or_else(|| String::from(id)));
            t.rich.styled(&cat(&["#", &name]), span::BOLD, "accent");
        } else if let Some(cmd) = target.strip_prefix('!') {
            let cmd = cmd.split('^').next().unwrap_or(cmd);
            let s = label.unwrap_or_else(|| cat(&["@", cmd.trim_start_matches("subteam")]));
            t.rich.styled(&s, span::BOLD, "accent");
        } else {
            let url = unescape(target);
            let shown = label.filter(|l| !l.is_empty()).unwrap_or_else(|| url.clone());
            let i = t.rich.styled(&shown, span::LINK, "");
            t.links.push((i, url));
        }
    }
    plain(&mut t.rich, &unescape(rest));
    t
}

/// Appends text, styling `code` and *bold* runs and swapping emoji shortcodes.
fn plain(r: &mut Rich, s: &str) {
    let mut rest = s;
    loop {
        // The next marker that has a closing partner on the same line.
        let next = rest.char_indices().find_map(|(i, c)| {
            if c != '`' && c != '*' {
                return None;
            }
            let after = &rest[i + 1..];
            let end = after.find(c).filter(|&e| e > 0 && !after[..e].contains('\n'))?;
            let boundary = |b: Option<char>| b.map_or(true, |b| !b.is_alphanumeric());
            if c == '*' && !(boundary(rest[..i].chars().last()) && boundary(after[end + 1..].chars().next())) {
                return None;
            }
            Some((i, c, end))
        });
        let Some((i, c, end)) = next else { break };
        r.push(&emoji(&rest[..i]));
        let inner = &rest[i + 1..i + 1 + end];
        if c == '`' {
            r.styled(inner, span::CODE, "");
        } else {
            r.styled(&emoji(inner), span::BOLD, "");
        }
        rest = &rest[i + 2 + end..];
    }
    r.push(&emoji(rest));
}

/// Outgoing text: escape &<>, and turn @user / #channel / @here into Slack
/// references when they match a known name (keys are lowercase).
pub fn encode(text: &str, handles: &Names, chans: &Names) -> String {
    let esc = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut out = String::with_capacity(esc.len());
    let mut it = esc.char_indices().peekable();
    let mut prev = ' ';
    while let Some((i, c)) = it.next() {
        if (c == '@' || c == '#') && !prev.is_alphanumeric() {
            let rest = &esc[i + 1..];
            let len = rest.find(|ch: char| !(ch.is_alphanumeric() || "._-'".contains(ch))).unwrap_or(rest.len());
            let word = rest[..len].trim_end_matches(['.', '\'']);
            let key = word.to_lowercase();
            let rep = match c {
                '@' if matches!(key.as_str(), "here" | "channel" | "everyone") => Some(cat(&["<!", &key, ">"])),
                '@' => handles.get(&key).map(|id| cat(&["<@", id, ">"])),
                _ => chans.get(&key).map(|id| cat(&["<#", id, ">"])),
            };
            if let (Some(rep), false) = (rep, word.is_empty()) {
                out.push_str(&rep);
                while it.peek().is_some_and(|&(j, _)| j < i + 1 + word.len()) {
                    it.next();
                }
                prev = 'x';
                continue;
            }
        }
        out.push(c);
        prev = c;
    }
    out
}

/// The emoji for a Slack shortcode (without colons), if it is one.
pub fn lookup(name: &str) -> Option<&'static str> {
    const TONES: [&str; 5] = ["\u{1f3fb}", "\u{1f3fc}", "\u{1f3fd}", "\u{1f3fe}", "\u{1f3ff}"];
    if let Some(n) = name.strip_prefix("skin-tone-") {
        return n.parse::<usize>().ok().filter(|n| (2..=6).contains(n)).map(|n| TONES[n - 2]);
    }
    let t = crate::emoji::EMOJI;
    t.binary_search_by(|(n, _)| (*n).cmp(name)).ok().map(|i| t[i].1)
}

pub fn emoji(s: &str) -> String {
    if !s.contains(':') {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(':') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || "_+-".contains(c)));
        let hit = end.filter(|&e| e > 0 && after.as_bytes()[e] == b':').and_then(|e| lookup(&after[..e]).map(|ch| (e, ch)));
        match hit {
            Some((e, ch)) => {
                out.push_str(ch);
                rest = &after[e + 1..];
            }
            None => {
                out.push(':');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A reaction chip label: the emoji (with its skin tone) if it is a
/// standard one, else `:name:`.
pub fn reaction(name: &str) -> String {
    let mut parts = name.split("::");
    let base = parts.next().unwrap_or(name);
    match lookup(base) {
        Some(ch) => {
            let mut s = String::from(ch);
            for p in parts {
                s.push_str(lookup(p).unwrap_or(""));
            }
            s
        }
        None => cat(&[":", base, ":"]),
    }
}

/// A Slack ts ("1700000000.123456") as microseconds, for ordering.
pub fn ts_us(ts: &str) -> u64 {
    let (s, f) = ts.split_once('.').unwrap_or((ts, ""));
    let mut us: u64 = s.parse().unwrap_or(0) * 1_000_000;
    let mut scale = 100_000;
    for d in f.bytes().take(6) {
        us += (d.wrapping_sub(b'0') as u64 % 10) * scale;
        scale /= 10;
    }
    us
}

/// A Slack ts in local seconds (see `time`).
pub fn local_secs(ts: &str) -> i64 {
    time::local((ts_us(ts) / 1_000_000) as i64)
}
