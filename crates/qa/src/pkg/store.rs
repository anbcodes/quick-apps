//! Manifests, the app cache, installs, updates and `qa pack` (docs/packaging.md).

use super::json::{self, Json};
use super::sha256;
use super::zip;
use super::{data_home, AppCtx};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub bundle: String,
    pub size: u64,
    pub sha256: String,
    pub runtime: u32,
    /// The terminal command name. Empty means one made from `name`.
    pub command: String,
}

impl Manifest {
    pub fn parse(s: &str) -> Result<Manifest, String> {
        let j = json::parse(s).map_err(|e| format!("manifest: {e}"))?;
        let req = |k: &str| j.str(k).map(|s| s.to_string()).ok_or_else(|| format!("manifest: missing \"{k}\""));
        Ok(Manifest {
            name: req("name")?,
            version: req("version")?,
            description: j.str("description").unwrap_or("").to_string(),
            bundle: req("bundle")?,
            size: j.num("size").unwrap_or(0.0) as u64,
            sha256: req("sha256")?.to_ascii_lowercase(),
            runtime: j.num("runtime").unwrap_or(1.0) as u32,
            command: j.str("command").unwrap_or("").to_string(),
        })
    }

    pub fn to_json(&self) -> String {
        let mut fields = vec![
            ("name", Json::Str(self.name.clone())),
            ("version", Json::Str(self.version.clone())),
            ("description", Json::Str(self.description.clone())),
            ("bundle", Json::Str(self.bundle.clone())),
            ("size", Json::Num(self.size as f64)),
            ("sha256", Json::Str(self.sha256.clone())),
            ("runtime", Json::Num(self.runtime as f64)),
        ];
        if !self.command.is_empty() {
            fields.push(("command", Json::Str(self.command.clone())));
        }
        json::object(&fields)
    }

    /// The command to link in ~/.local/bin: `command` if it is a safe file
    /// name, else the name in lowercase with dashes ("Slack Lite" → "slack-lite").
    pub fn command_name(&self) -> Option<String> {
        let ok = |c: &str| {
            !c.is_empty() && c != "qa" && !c.starts_with(['-', '.']) && c.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        };
        if ok(&self.command) {
            return Some(self.command.clone());
        }
        let slug: String = self.name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        let slug = slug.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
        ok(&slug).then_some(slug)
    }
}

/// Dotted-number comparison: `1.10.0` > `1.9.2`; `1.0.0-beta` < `1.0.0`.
pub fn cmp_versions(a: &str, b: &str) -> Ordering {
    let split = |v: &str| -> (Vec<u64>, Option<String>) {
        let (core, pre) = match v.trim().split_once('-') {
            Some((c, p)) => (c, Some(p.to_string())),
            None => (v.trim(), None),
        };
        (core.split('.').map(|p| p.parse().unwrap_or(0)).collect(), pre)
    };
    let (ca, pa) = split(a);
    let (cb, pb) = split(b);
    for i in 0..ca.len().max(cb.len()) {
        let o = ca.get(i).unwrap_or(&0).cmp(cb.get(i).unwrap_or(&0));
        if o != Ordering::Equal {
            return o;
        }
    }
    match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(&y),
    }
}

fn is_loopback(url: &str) -> bool {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or("");
    let host = rest.split(['/', '?']).next().unwrap_or("");
    let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    host == "localhost" || host.starts_with("127.") || host == "[::1]"
}

/// Checks the URL rules: https anywhere, http only on loopback, file:// for development.
pub fn check_url(url: &str) -> Result<(), String> {
    if url.starts_with("https://") || url.starts_with("file://") || (url.starts_with("http://") && is_loopback(url)) {
        Ok(())
    } else {
        Err(format!("{url}: manifests must use https:// (http:// only for localhost)"))
    }
}

pub struct Fetched {
    pub status: u16,
    pub headers: String,
    pub body: Vec<u8>,
}

impl Fetched {
    pub fn header(&self, k: &str) -> Option<&str> {
        self.headers.lines().find_map(|l| l.split_once(':').filter(|(n, _)| n.trim().eq_ignore_ascii_case(k)).map(|(_, v)| v.trim()))
    }
}

pub fn fetch(url: &str, headers: &[(&str, &str)], timeout: Duration, max: usize) -> Result<Fetched, String> {
    check_url(url)?;
    if let Some(p) = url.strip_prefix("file://") {
        let body = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
        return Ok(Fetched { status: 200, headers: String::new(), body });
    }
    let req = crate::io::http::Req {
        method: "GET".into(),
        url: url.into(),
        headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        body: Vec::new(),
        timeout,
        max_body: max,
    };
    let r = crate::io::http::fetch(&req)?;
    Ok(Fetched { status: r.status, headers: r.headers, body: r.body })
}

pub fn join_url(manifest_url: &str, rel: &str) -> String {
    if rel.contains("://") {
        return rel.to_string();
    }
    let base = manifest_url.rsplit_once('/').map(|(b, _)| b).unwrap_or(manifest_url);
    format!("{base}/{rel}")
}

pub(super) fn bundle_dir(ctx: &AppCtx) -> PathBuf {
    ctx.home.join("bundle")
}

/// The context for a manifest URL, named from its cached manifest if there is one.
pub fn ctx_for(url: &str) -> AppCtx {
    let mut ctx = AppCtx::new(url, "", None);
    ctx.bundle = Some(bundle_dir(&ctx));
    ctx.name = cached_manifest(&ctx).map(|m| m.name).unwrap_or_else(|| "app".into());
    ctx
}

pub fn cached_manifest(ctx: &AppCtx) -> Option<Manifest> {
    Manifest::parse(&std::fs::read_to_string(ctx.home.join("manifest.json")).ok()?).ok()
}

pub fn is_installed(ctx: &AppCtx) -> bool {
    bundle_dir(ctx).join("app.wasm").exists()
}

pub fn get_manifest(url: &str, etag: Option<&str>, timeout: Duration) -> Result<Option<(Manifest, Option<String>)>, String> {
    let mut h = Vec::new();
    if let Some(e) = etag {
        h.push(("If-None-Match", e));
    }
    let f = fetch(url, &h, timeout, 1 << 20)?;
    if f.status == 304 {
        return Ok(None);
    }
    if f.status != 200 {
        return Err(format!("{url}: HTTP {}", f.status));
    }
    let m = Manifest::parse(&String::from_utf8_lossy(&f.body))?;
    if m.runtime > qa_proto::ABI_VERSION {
        return Err(format!("{} needs a newer Quick Apps runtime (ABI {}, this is {})", m.name, m.runtime, qa_proto::ABI_VERSION));
    }
    Ok(Some((m, f.header("etag").map(|s| s.to_string()))))
}

/// Downloads, verifies and extracts a bundle into `dest` (replacing it).
fn get_bundle(url: &str, m: &Manifest, dest: &Path) -> Result<(), String> {
    let burl = join_url(url, &m.bundle);
    let f = fetch(&burl, &[], Duration::from_secs(120), (zip::MAX_UNZIPPED as usize).max(m.size as usize + 1))?;
    if f.status != 200 {
        return Err(format!("{burl}: HTTP {}", f.status));
    }
    if m.size != 0 && f.body.len() as u64 != m.size {
        return Err(format!("{burl}: size {} does not match the manifest ({})", f.body.len(), m.size));
    }
    let got = sha256::hex(&sha256::digest(&f.body));
    if got != m.sha256 {
        return Err(format!("{burl}: sha256 does not match the manifest"));
    }
    let entries = zip::read(&f.body)?;
    if !entries.iter().any(|e| e.name == "app.wasm") {
        return Err("the bundle has no app.wasm".into());
    }
    let _ = std::fs::remove_dir_all(dest);
    zip::extract(&entries, dest)
}

fn write_meta(ctx: &AppCtx, m: &Manifest, etag: Option<&str>) {
    let _ = std::fs::write(ctx.home.join("source"), format!("{}\n", ctx.source));
    let _ = std::fs::write(ctx.home.join("manifest.json"), m.to_json());
    match etag {
        Some(e) => {
            let _ = std::fs::write(ctx.home.join("etag"), e);
        }
        None => {
            let _ = std::fs::remove_file(ctx.home.join("etag"));
        }
    }
}

/// A verified bundle that is downloaded but not installed yet, so `qa open`
/// can fetch while the user decides.
pub struct Staged {
    pub url: String,
    pub m: Manifest,
    etag: Option<String>,
    pub dir: PathBuf,
}

impl Drop for Staged {
    /// A download nobody committed is thrown away.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Fetches a manifest, ignoring any cached ETag.
pub fn fetch_manifest(url: &str) -> Result<(Manifest, Option<String>), String> {
    check_url(url)?;
    get_manifest(url, None, Duration::from_secs(30))?.ok_or_else(|| "no manifest".into())
}

/// Downloads and verifies the bundle into the staging folder.
pub fn stage(url: &str, m: Manifest, etag: Option<String>) -> Result<Staged, String> {
    let dir = data_home().join("staging").join(super::app_id(url));
    get_bundle(url, &m, &dir)?;
    Ok(Staged { url: url.to_string(), m, etag, dir })
}

/// Moves a staged bundle into place. The previous bundle becomes bundle.old.
pub fn commit(s: Staged) -> Result<AppCtx, String> {
    let mut ctx = ctx_for(&s.url);
    std::fs::create_dir_all(&ctx.home).map_err(|e| e.to_string())?;
    let b = bundle_dir(&ctx);
    if b.exists() {
        let old = ctx.home.join("bundle.old");
        let _ = std::fs::remove_dir_all(&old);
        let _ = std::fs::rename(&b, &old);
    }
    std::fs::rename(&s.dir, &b).map_err(|e| e.to_string())?;
    write_meta(&ctx, &s.m, s.etag.as_deref());
    ctx.name = s.m.name.clone();
    Ok(ctx)
}

/// Fetches and caches an app. Returns its context.
pub fn install(url: &str) -> Result<AppCtx, String> {
    let (m, etag) = fetch_manifest(url)?;
    commit(stage(url, m, etag)?)
}

/// A newer manifest, if the server has one the user has not skipped.
pub fn check_update(ctx: &AppCtx, timeout: Duration) -> Result<Option<Manifest>, String> {
    let cur = cached_manifest(ctx).ok_or("no cached manifest")?;
    let etag = std::fs::read_to_string(ctx.home.join("etag")).ok();
    let Some((m, _)) = get_manifest(&ctx.source, etag.as_deref(), timeout)? else { return Ok(None) };
    if cmp_versions(&m.version, &cur.version) != Ordering::Greater {
        return Ok(None);
    }
    if std::fs::read_to_string(ctx.home.join("skip")).is_ok_and(|s| s.trim() == m.version) {
        return Ok(None);
    }
    Ok(Some(m))
}

pub fn skip_version(ctx: &AppCtx, v: &str) {
    let _ = std::fs::write(ctx.home.join("skip"), v);
}

/// Downloads an update next to the current bundle.
pub fn download_update(ctx: &AppCtx, m: &Manifest) -> Result<(), String> {
    get_bundle(&ctx.source, m, &ctx.home.join("bundle.new"))?;
    std::fs::write(ctx.home.join("manifest.new"), m.to_json()).map_err(|e| e.to_string())
}

/// Swaps a downloaded update in: bundle -> bundle.old, bundle.new -> bundle.
pub fn apply_update(ctx: &AppCtx) -> Result<Manifest, String> {
    let h = &ctx.home;
    let m = Manifest::parse(&std::fs::read_to_string(h.join("manifest.new")).map_err(|e| e.to_string())?)?;
    let _ = std::fs::remove_dir_all(h.join("bundle.old"));
    std::fs::rename(h.join("bundle"), h.join("bundle.old")).map_err(|e| e.to_string())?;
    std::fs::rename(h.join("bundle.new"), h.join("bundle")).map_err(|e| e.to_string())?;
    let _ = std::fs::rename(h.join("manifest.json"), h.join("manifest.old"));
    let _ = std::fs::rename(h.join("manifest.new"), h.join("manifest.json"));
    // The next check fetches the full manifest again.
    let _ = std::fs::remove_file(h.join("etag"));
    Ok(m)
}

pub fn rollback(ctx: &AppCtx) -> Result<Manifest, String> {
    let h = &ctx.home;
    if !h.join("bundle.old").exists() {
        return Err("no previous version to roll back to".into());
    }
    let tmp = h.join("bundle.swap");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::rename(h.join("bundle"), &tmp).map_err(|e| e.to_string())?;
    std::fs::rename(h.join("bundle.old"), h.join("bundle")).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, h.join("bundle.old")).map_err(|e| e.to_string())?;
    if h.join("manifest.old").exists() {
        let _ = std::fs::rename(h.join("manifest.json"), h.join("manifest.swap"));
        let _ = std::fs::rename(h.join("manifest.old"), h.join("manifest.json"));
        let _ = std::fs::rename(h.join("manifest.swap"), h.join("manifest.old"));
    }
    let _ = std::fs::remove_file(h.join("etag"));
    cached_manifest(ctx).ok_or_else(|| "no manifest after rollback".into())
}

/// Every installed app.
pub fn installed() -> Vec<(AppCtx, Manifest)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(data_home().join("apps")) else { return out };
    for e in rd.flatten() {
        let Ok(src) = std::fs::read_to_string(e.path().join("source")) else { continue };
        let ctx = ctx_for(src.trim());
        if let Some(m) = cached_manifest(&ctx) {
            out.push((ctx, m));
        }
    }
    out.sort_by(|a, b| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()));
    out
}

/// Finds an installed app by id, name, command or manifest URL.
pub fn find(arg: &str) -> Option<AppCtx> {
    installed()
        .into_iter()
        .find(|(c, m)| c.id == arg || c.source == arg || m.name.eq_ignore_ascii_case(arg) || linked_command(c).as_deref() == Some(arg))
        .map(|(c, _)| c)
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

#[cfg(target_os = "linux")]
fn applications_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share")).join("applications")
}

#[cfg(target_os = "linux")]
pub fn desktop_path(ctx: &AppCtx) -> PathBuf {
    applications_dir().join(format!("qa-{}.desktop", ctx.id))
}

pub(super) fn exe() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "qa".into())
}

#[cfg(target_os = "linux")]
pub fn write_desktop(ctx: &AppCtx, m: &Manifest) -> Result<PathBuf, String> {
    let exe = exe();
    let clean = |s: &str| s.replace(['\n', '\r'], " ");
    let icon = bundle_dir(ctx).join("icon.png");
    let mut s = format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec={} run {}\nStartupWMClass=qa-{}\n",
        clean(&m.name),
        clean(&m.description),
        exe,
        ctx.source.replace(' ', "%20"),
        ctx.id
    );
    if icon.exists() {
        s += &format!("Icon={}\n", icon.display());
    }
    let p = desktop_path(ctx);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, s).map_err(|e| e.to_string())?;
    Ok(p)
}

/// Registers `qa open` as the handler for `qa:` links, so a web page can
/// install an app with one click. Writes qa-open.desktop and makes it the
/// default in ~/.config/mimeapps.list.
#[cfg(target_os = "linux")]
pub fn register_handler() -> Result<PathBuf, String> {
    let dir = applications_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join("qa-open.desktop");
    let s = format!(
        "[Desktop Entry]\nType=Application\nName=Quick Apps\nComment=Installs and opens apps from qa: links\nExec={} open %u\nMimeType=x-scheme-handler/qa;\nNoDisplay=true\nTerminal=false\n",
        exe()
    );
    if std::fs::read_to_string(&p).ok().as_deref() != Some(&s) {
        std::fs::write(&p, s).map_err(|e| e.to_string())?;
    }
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"));
    let list = cfg.join("mimeapps.list");
    let old = std::fs::read_to_string(&list).unwrap_or_default();
    let new = set_default(&old, "x-scheme-handler/qa", "qa-open.desktop");
    if new != old {
        let _ = std::fs::create_dir_all(&cfg);
        std::fs::write(&list, new).map_err(|e| format!("{}: {e}", list.display()))?;
    }
    Ok(p)
}

/// Sets `mime=entry` in the [Default Applications] group of a mimeapps.list.
#[cfg(target_os = "linux")]
fn set_default(list: &str, mime: &str, entry: &str) -> String {
    let line = format!("{mime}={entry}");
    let mut out = Vec::new();
    let (mut in_group, mut done) = (false, false);
    for l in list.lines() {
        if l.starts_with('[') {
            if in_group && !done {
                // Before the blank lines that end the group.
                let at = out.iter().rposition(|l: &String| !l.trim().is_empty()).map_or(0, |i| i + 1);
                out.insert(at, line.clone());
                done = true;
            }
            in_group = l.trim() == "[Default Applications]";
        } else if in_group && l.split_once('=').is_some_and(|(k, _)| k.trim() == mime) {
            if !done {
                out.push(line.clone());
                done = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !done {
        if !in_group {
            if out.last().is_some_and(|l| !l.is_empty()) {
                out.push(String::new());
            }
            out.push("[Default Applications]".into());
        }
        out.push(line);
    }
    out.join("\n") + "\n"
}

#[cfg(target_os = "macos")]
pub use super::macapp::{desktop_path, register_handler, write_desktop};

pub fn bin_dir() -> PathBuf {
    home().join(".local/bin")
}

/// The command an app is linked as, if any.
pub fn linked_command(ctx: &AppCtx) -> Option<String> {
    std::fs::read_to_string(ctx.home.join("command")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Links ~/.local/bin/<command> to the qa binary, which runs the app when
/// started under that name. Never replaces a file it didn't make: returns
/// Ok(None) when the name is taken.
pub fn link_command(ctx: &AppCtx, m: &Manifest) -> Result<Option<String>, String> {
    let Some(name) = m.command_name() else { return Ok(None) };
    let dir = bin_dir();
    let p = dir.join(&name);
    let target = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(old) = linked_command(ctx).filter(|o| *o != name) {
        unlink_command_named(&old);
    }
    if on_path_elsewhere(&name, &dir) {
        return Ok(None);
    }
    if std::fs::symlink_metadata(&p).is_ok() {
        let ours = std::fs::read_link(&p).is_ok_and(|t| t == target) && command_owner(&name).map_or(true, |c| c.id == ctx.id);
        if !ours {
            return Ok(None);
        }
    } else {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::os::unix::fs::symlink(&target, &p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    std::fs::write(ctx.home.join("command"), &name).map_err(|e| e.to_string())?;
    Ok(Some(name))
}

/// Whether another folder on PATH has a `name`, which our link would shadow.
fn on_path_elsewhere(name: &str, ours: &Path) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).filter(|d| d != ours).any(|d| d.join(name).exists())
}

/// Removes ~/.local/bin/<name> if it is a link to a qa binary.
fn unlink_command_named(name: &str) {
    let p = bin_dir().join(name);
    let is_qa = std::fs::read_link(&p).is_ok_and(|t| t.file_name().is_some_and(|f| f == "qa") || Some(t.clone()) == std::env::current_exe().ok());
    if is_qa {
        let _ = std::fs::remove_file(p);
    }
}

/// The installed app linked as `name`.
pub fn command_owner(name: &str) -> Option<AppCtx> {
    installed().into_iter().map(|(c, _)| c).find(|c| linked_command(c).as_deref() == Some(name))
}

pub fn remove(ctx: &AppCtx, keep_data: bool) -> Result<(), String> {
    // A menu entry is a file on Linux and a folder (the .app) on macOS.
    let entry = desktop_path(ctx);
    let _ = if entry.is_dir() { std::fs::remove_dir_all(&entry) } else { std::fs::remove_file(&entry) };
    if let Some(c) = linked_command(ctx) {
        unlink_command_named(&c);
    }
    if keep_data {
        for n in ["bundle", "bundle.old", "bundle.new"] {
            let _ = std::fs::remove_dir_all(ctx.home.join(n));
        }
        for n in ["manifest.json", "manifest.old", "manifest.new", "etag", "skip", "source", "command"] {
            let _ = std::fs::remove_file(ctx.home.join(n));
        }
        Ok(())
    } else {
        std::fs::remove_dir_all(&ctx.home).map_err(|e| e.to_string())
    }
}

/// `qa pack`: builds `<name>-<version>.zip` and a complete `app.json`.
pub fn pack(dir: &Path, out: &Path) -> Result<(PathBuf, PathBuf), String> {
    let meta = std::fs::read_to_string(dir.join("app.json")).map_err(|e| format!("{}: {e}", dir.join("app.json").display()))?;
    let j = json::parse(&meta)?;
    let name = j.str("name").ok_or("app.json: missing \"name\"")?.to_string();
    let version = j.str("version").ok_or("app.json: missing \"version\"")?.to_string();
    let description = j.str("description").unwrap_or("").to_string();
    let command = j.str("command").unwrap_or("").to_string();
    let wasm = std::fs::read(dir.join("app.wasm")).map_err(|e| format!("app.wasm: {e}"))?;
    let mut entries = vec![("app.wasm".to_string(), wasm)];
    if let Ok(icon) = std::fs::read(dir.join("icon.png")) {
        entries.push(("icon.png".into(), icon));
    }
    fn walk(base: &Path, p: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
        let Ok(rd) = std::fs::read_dir(p) else { return Ok(()) };
        let mut items: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        items.sort();
        for f in items {
            if f.is_dir() {
                walk(base, &f, out)?;
            } else {
                let rel = f.strip_prefix(base).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
                out.push((rel, std::fs::read(&f).map_err(|e| e.to_string())?));
            }
        }
        Ok(())
    }
    walk(dir, &dir.join("assets"), &mut entries)?;
    let zipb = zip::write(&entries);
    let slug: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let file = format!("{slug}-{version}.zip");
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let zp = out.join(&file);
    std::fs::write(&zp, &zipb).map_err(|e| e.to_string())?;
    let m = Manifest {
        name,
        version,
        description,
        bundle: file,
        size: zipb.len() as u64,
        sha256: sha256::hex(&sha256::digest(&zipb)),
        runtime: qa_proto::ABI_VERSION,
        command,
    };
    let mp = out.join("app.json");
    std::fs::write(&mp, m.to_json()).map_err(|e| e.to_string())?;
    Ok((zp, mp))
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(cmp_versions("1.10.0", "1.9.2"), Ordering::Greater);
        assert_eq!(cmp_versions("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(cmp_versions("1.0.0-beta", "1.0.0"), Ordering::Less);
        assert_eq!(cmp_versions("2.0.0", "10.0.0"), Ordering::Less);
    }

    #[test]
    fn manifest_roundtrip() {
        let m = Manifest {
            name: "Notes \"x\"".into(),
            version: "1.2.0".into(),
            description: "Plain text".into(),
            bundle: "notes-1.2.0.zip".into(),
            size: 48213,
            sha256: "ab".into(),
            runtime: 1,
            command: String::new(),
        };
        assert_eq!(Manifest::parse(&m.to_json()).unwrap(), m);
        assert!(check_url("http://example.com/app.json").is_err());
        assert!(check_url("http://127.0.0.1:8000/app.json").is_ok());
        assert_eq!(m.command_name().as_deref(), Some("notes-x"));
        assert_eq!(Manifest { command: "../evil".into(), ..m.clone() }.command_name().as_deref(), Some("notes-x"));
        assert_eq!(Manifest { command: "nt".into(), ..m.clone() }.command_name().as_deref(), Some("nt"));
        assert_eq!(join_url("https://a.b/x/app.json", "n.zip"), "https://a.b/x/n.zip");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mimeapps() {
        let e = "x-scheme-handler/qa=qa-open.desktop\n";
        assert_eq!(set_default("", "x-scheme-handler/qa", "qa-open.desktop"), format!("[Default Applications]\n{e}"));
        let l = "[Added Associations]\na=b\n\n[Default Applications]\ntext/html=firefox.desktop\nx-scheme-handler/qa=old.desktop\n";
        assert_eq!(
            set_default(l, "x-scheme-handler/qa", "qa-open.desktop"),
            format!("[Added Associations]\na=b\n\n[Default Applications]\ntext/html=firefox.desktop\n{e}")
        );
        let l = "[Default Applications]\ntext/html=firefox.desktop\n\n[Added Associations]\na=b\n";
        assert_eq!(
            set_default(l, "x-scheme-handler/qa", "qa-open.desktop"),
            format!("[Default Applications]\ntext/html=firefox.desktop\n{e}\n[Added Associations]\na=b\n")
        );
    }
}
