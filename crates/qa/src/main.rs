mod demo;
mod cli;
mod dev;
mod gfx;
mod host;
mod io;
mod open;
mod pkg;
mod platform;
mod prompt;
mod shell;
mod shot;
mod sys;
mod text;
mod todo;
mod ui;
#[cfg(test)]
mod tests;

const USAGE: &str = "usage: qa <command>

commands:
  run <url|path|app>            run an app (manifest URL, .wasm, folder, or installed id/name)
  dev <dir> [--map a=b] [--grant g]
                                build a Rust app crate, run it, reload on changes
  shot <app> [options] [step…]  run an app headless, act on it, write a PNG and tree dump
  install <url>                 cache an app, add it to the app menu and ~/.local/bin
  open <qa:url>                 what qa: links run: start an installed app, else ask to install
  setup                         register qa as the handler for qa: links
  list                          installed apps
  update <app>                  check for a newer version and apply it
  rollback <app>                go back to the previous version
  remove <app> [--keep-data]    delete an app
  grants <app> [--revoke N]     show or revoke permissions
  pack <dir> [--out <dir>]      build a bundle zip and manifest
  demo                          a window that exercises the renderer
  demo-todo                     a todo list built through patches by native code
";

fn main() {
    // Started through an app's ~/.local/bin link: run that app.
    let argv0 = std::env::args().next().unwrap_or_default();
    let base = std::path::Path::new(&argv0).file_name().and_then(|s| s.to_str()).unwrap_or("qa");
    if base != "qa" {
        if let Some(ctx) = pkg::store::command_owner(base) {
            std::process::exit(cli::run(&ctx.source));
        }
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(|s| s.as_str()) {
        Some("run") if args.len() == 2 => cli::run(&args[1]),
        Some("install") if args.len() == 2 => cli::install(&args[1]),
        Some("open") if args.len() == 2 => open::open(&args[1]),
        Some("setup") => cli::setup(),
        #[cfg(target_os = "macos")]
        Some("url-handler") => url_handler(),
        Some("list") => cli::list(),
        Some("update") if args.len() == 2 => cli::update(&args[1]),
        Some("rollback") if args.len() == 2 => cli::rollback(&args[1]),
        Some("remove") if args.len() >= 2 => cli::remove(&args[1], args.iter().any(|a| a == "--keep-data")),
        Some("grants") if args.len() >= 2 => cli::grants(&args[1], &args[2..]),
        Some("pack") if args.len() >= 2 => cli::pack(&args[1], &args[2..]),
        Some("dev") if args.len() >= 2 => dev::run(&args[1..]),
        Some("shot") if args.len() >= 2 => shot::run(&args[1..]),
        Some("demo") => demo::run(),
        Some("demo-todo") => run_native(todo::Todo::new(), "Todo", "qa-demo-todo"),
        Some("-h") | Some("--help") | Some("help") => {
            print!("{USAGE}");
            0
        }
        _ => {
            eprint!("{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

/// What the `qa:` link app runs: waits for the link the system sends it, then opens it.
#[cfg(target_os = "macos")]
fn url_handler() -> i32 {
    match platform::macos::wait_url(10_000) {
        Some(url) => open::open(&url),
        None => 0,
    }
}

fn load_fonts() -> Option<text::Fonts> {
    let mut fonts = text::Fonts::new();
    if fonts.find_first(text::DEFAULT_SANS).is_none() {
        eprintln!("qa: no font found. Set the font.family token in ~/.config/quick-apps/theme to a .ttf path.");
        return None;
    }
    Some(fonts)
}

fn run_native(mut d: impl shell::Driver, title: &str, app_id: &str) -> i32 {
    run_native_sized(&mut d, title, app_id, (900, 600))
}

/// Runs a driver in a window that starts at `size` (logical px) unless the
/// compositor picks one.
fn run_native_sized(d: &mut dyn shell::Driver, title: &str, app_id: &str, size: (i32, i32)) -> i32 {
    let Some(fonts) = load_fonts() else { return 1 };
    let mut p = match platform::Native::connect(title, app_id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("qa: {e}");
            return 1;
        }
    };
    p.default_size = size;
    let ui = ui::Ui::new(fonts, ui::theme::Theme::load(app_id.strip_prefix("qa-")));
    let mut sh = shell::Shell::new(p, ui);
    sh.last_title = title.to_string();
    sh.run(d)
}

pub fn run_path(path: &str) -> i32 {
    let p = std::path::Path::new(path);
    let file = if p.is_dir() { p.join("app.wasm") } else { p.to_path_buf() };
    let wasm = match std::fs::read(&file) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("qa: {}: {e}", file.display());
            return 1;
        }
    };
    let name = file.file_stem().and_then(|s| s.to_str()).unwrap_or("app").to_string();
    let name = if name == "app" { p.file_name().and_then(|s| s.to_str()).unwrap_or("app").to_string() } else { name };
    let ctx = pkg::AppCtx::local(p, &name);
    run_app(wasm, ctx)
}

/// Runs a wasm app with every service: single instance, network, messages, inbox.
pub fn run_app(wasm: Vec<u8>, ctx: pkg::AppCtx) -> i32 {
    run_app_with(wasm, ctx, false)
}

pub fn run_app_with(wasm: Vec<u8>, ctx: pkg::AppCtx, check_update: bool) -> i32 {
    if io::activate_running(&ctx.id) {
        return 0;
    }
    let app_id = format!("qa-{}", ctx.id);
    let name = ctx.name.clone();
    let mut d = host::WasmDriver::new(wasm, ctx);
    d.services = Box::new(io::Net::new(&d.ctx, true));
    d.after_start = io::drain_inbox(&d.ctx);
    if check_update {
        d.check_for_update();
    }
    run_native(d, &name, &app_id)
}
