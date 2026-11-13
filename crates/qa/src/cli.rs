//! The `qa` commands that manage installed apps (docs/packaging.md).

use crate::host::grants::Grants;
use crate::pkg::store::{self, Manifest};
use crate::pkg::AppCtx;
use std::path::Path;

fn die(msg: impl std::fmt::Display) -> i32 {
    eprintln!("qa: {msg}");
    1
}

pub fn find(arg: &str) -> Result<AppCtx, String> {
    store::find(arg).ok_or_else(|| format!("{arg}: no such installed app (see `qa list`)"))
}

fn manifest_url(arg: &str) -> Option<String> {
    if arg.contains("://") {
        return Some(arg.to_string());
    }
    let p = Path::new(arg);
    if p.extension().is_some_and(|e| e == "json") && p.is_file() {
        return std::fs::canonicalize(p).ok().map(|a| format!("file://{}", a.display()));
    }
    None
}

pub fn run(arg: &str) -> i32 {
    if let Some(url) = manifest_url(arg) {
        return run_url(&url);
    }
    if Path::new(arg).exists() {
        return crate::run_path(arg);
    }
    match find(arg) {
        Ok(ctx) => run_url(&ctx.source),
        Err(e) => die(e),
    }
}

fn run_url(url: &str) -> i32 {
    let mut ctx = store::ctx_for(url);
    if crate::io::activate_running(&ctx.id) {
        return 0;
    }
    let fresh = !store::is_installed(&ctx);
    if fresh {
        eprintln!("qa: downloading {url}");
        match store::install(url) {
            Ok(c) => ctx = c,
            Err(e) => return die(e),
        }
    }
    let wasm = match std::fs::read(ctx.home.join("bundle/app.wasm")) {
        Ok(w) => w,
        Err(e) => return die(format!("{}: {e}", ctx.home.join("bundle/app.wasm").display())),
    };
    crate::run_app_with(wasm, ctx, !fresh)
}

pub fn install(url: &str) -> i32 {
    let Some(url) = manifest_url(url) else { return die(format!("{url}: expected a manifest URL")) };
    let _ = store::register_handler();
    match store::install(&url) {
        Ok(ctx) => {
            let m = store::cached_manifest(&ctx).unwrap();
            match store::write_desktop(&ctx, &m) {
                Ok(p) => println!("installed {} {} ({})\nmenu entry: {}", m.name, m.version, ctx.id, p.display()),
                Err(e) => println!("installed {} {} ({}); menu entry failed: {e}", m.name, m.version, ctx.id),
            }
            match store::link_command(&ctx, &m) {
                Ok(Some(c)) => println!("command: {}", store::bin_dir().join(c).display()),
                Ok(None) => println!("no command linked: {} is taken", m.command_name().unwrap_or_default()),
                Err(e) => println!("no command linked: {e}"),
            }
            0
        }
        Err(e) => die(e),
    }
}

pub fn setup() -> i32 {
    match store::register_handler() {
        Ok(p) => {
            println!("qa: links now open with {}", p.display());
            let bin = store::bin_dir();
            let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == bin));
            if !on_path {
                println!("note: {} is not on your PATH, so app commands won't be found", bin.display());
            }
            0
        }
        Err(e) => die(e),
    }
}

fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

fn human(n: u64) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

pub fn list() -> i32 {
    let apps = store::installed();
    if apps.is_empty() {
        println!("no apps installed");
        return 0;
    }
    let w = apps.iter().map(|(_, m)| m.name.len()).max().unwrap_or(4).max(4);
    println!("{:<16}  {:<w$}  {:<10}  {:>9}  URL", "ID", "NAME", "VERSION", "DATA");
    for (ctx, m) in apps {
        println!("{:<16}  {:<w$}  {:<10}  {:>9}  {}", ctx.id, m.name, m.version, human(dir_size(&ctx.data_dir())), ctx.source);
    }
    0
}

pub fn update(arg: &str) -> i32 {
    let ctx = match find(arg) {
        Ok(c) => c,
        Err(e) => return die(e),
    };
    match store::check_update(&ctx, std::time::Duration::from_secs(30)) {
        Ok(None) => {
            println!("{} is up to date", ctx.name);
            0
        }
        Ok(Some(m)) => match store::download_update(&ctx, &m).and_then(|_| store::apply_update(&ctx)) {
            Ok(m) => {
                let _ = store::write_desktop(&ctx, &m).ok().filter(|_| store::desktop_path(&ctx).exists());
                println!("updated {} to {}", m.name, m.version);
                0
            }
            Err(e) => die(e),
        },
        Err(e) => die(e),
    }
}

pub fn rollback(arg: &str) -> i32 {
    match find(arg).and_then(|c| store::rollback(&c)) {
        Ok(m) => {
            println!("{} is back at {}", m.name, m.version);
            0
        }
        Err(e) => die(e),
    }
}

pub fn remove(arg: &str, keep_data: bool) -> i32 {
    let ctx = match find(arg) {
        Ok(c) => c,
        Err(e) => return die(e),
    };
    match store::remove(&ctx, keep_data) {
        Ok(()) => {
            println!("removed {}{}", ctx.name, if keep_data { " (kept its data)" } else { "" });
            0
        }
        Err(e) => die(e),
    }
}

pub fn grants(arg: &str, rest: &[String]) -> i32 {
    let ctx = match find(arg) {
        Ok(c) => c,
        Err(e) => return die(e),
    };
    let mut g = Grants::load(ctx.grants_path());
    if rest.first().map(|s| s.as_str()) == Some("--revoke") {
        let Some(n) = rest.get(1).and_then(|s| s.parse::<usize>().ok()) else { return die("--revoke needs a line number") };
        if n == 0 || n > g.list.len() {
            return die(format!("no grant {n}"));
        }
        let r = g.list.remove(n - 1);
        g.save();
        println!("revoked: {} (takes effect at the next launch)", r.line());
        return 0;
    }
    if g.list.is_empty() {
        println!("{} has no grants", ctx.name);
    }
    for (i, gr) in g.list.iter().enumerate() {
        println!("{:>3}  {}", i + 1, gr.line());
    }
    0
}

pub fn pack(dir: &str, rest: &[String]) -> i32 {
    let out = match rest {
        [flag, o, ..] if flag == "--out" => Path::new(o).to_path_buf(),
        _ => Path::new(dir).join("dist"),
    };
    match store::pack(Path::new(dir), &out) {
        Ok((z, m)) => {
            let man = Manifest::parse(&std::fs::read_to_string(&m).unwrap_or_default()).ok();
            println!("{}\n{}", z.display(), m.display());
            if let Some(man) = man {
                println!("{} {} · {} bytes · sha256 {}", man.name, man.version, man.size, man.sha256);
            }
            0
        }
        Err(e) => die(e),
    }
}
