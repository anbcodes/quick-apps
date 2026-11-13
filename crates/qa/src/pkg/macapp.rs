//! What a Linux menu entry is on a Mac: a small .app bundle in ~/Applications whose launcher
//! runs `qa run`, and one for `qa:` links that Launch Services is told about.

use super::store::{exe, Manifest};
use super::AppCtx;
use std::path::{Path, PathBuf};
use std::process::Command;

const PREFIX: &str = "codes.anb.qa";

fn applications_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Applications")
}

/// The bundle written for an app; `write_desktop` records where it put it.
pub fn desktop_path(ctx: &AppCtx) -> PathBuf {
    match std::fs::read_to_string(ctx.home.join("bundle_path")) {
        Ok(s) if !s.trim().is_empty() => PathBuf::from(s.trim()),
        _ => applications_dir().join(format!("qa-{}.app", ctx.id)),
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A file name that is safe and still reads as the app's name.
fn file_name(name: &str) -> String {
    let n: String = name.chars().map(|c| if c == '/' || c == ':' || c.is_control() { '-' } else { c }).collect();
    let n = n.trim().trim_start_matches('.').to_string();
    if n.is_empty() {
        "App".into()
    } else {
        n
    }
}

fn bundle_id_of(app: &Path) -> Option<String> {
    let s = std::fs::read_to_string(app.join("Contents/Info.plist")).ok()?;
    let i = s.find("<key>CFBundleIdentifier</key>")?;
    let rest = &s[i..];
    let a = rest.find("<string>")? + 8;
    let b = rest.find("</string>")?;
    Some(rest[a..b].to_string())
}

/// Writes `<name>.app` with a launcher script, an icon if there is one, and the plist.
fn make_bundle(app: &Path, id: &str, name: &str, version: &str, exec_line: &str, icon_png: Option<&Path>, extra_plist: &str, ui_element: bool) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(app);
    let macos = app.join("Contents/MacOS");
    let res = app.join("Contents/Resources");
    std::fs::create_dir_all(&macos).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&res).map_err(|e| e.to_string())?;
    let launcher = macos.join("launcher");
    std::fs::write(&launcher, format!("#!/bin/sh\nexec {exec_line}\n")).map_err(|e| e.to_string())?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let mut icon_key = String::new();
    if let Some(png) = icon_png {
        if make_icns(png, &res.join("icon.icns")) {
            icon_key = "<key>CFBundleIconFile</key><string>icon</string>".into();
        }
    }
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n\
<key>CFBundleName</key><string>{n}</string>\n<key>CFBundleDisplayName</key><string>{n}</string>\n\
<key>CFBundleIdentifier</key><string>{PREFIX}.{id}</string>\n<key>CFBundleExecutable</key><string>launcher</string>\n\
<key>CFBundlePackageType</key><string>APPL</string>\n<key>CFBundleVersion</key><string>{v}</string>\n<key>CFBundleShortVersionString</key><string>{v}</string>\n\
<key>NSHighResolutionCapable</key><true/>\n{icon_key}{ui}{extra_plist}</dict></plist>\n",
        n = xml(name),
        v = xml(version),
        ui = if ui_element { "<key>LSUIElement</key><true/>\n" } else { "" },
    );
    std::fs::write(app.join("Contents/Info.plist"), plist).map_err(|e| e.to_string())?;
    lsregister(app);
    Ok(())
}

/// sips scales the image to each size an icon set needs, and iconutil packs them.
fn make_icns(png: &Path, out: &Path) -> bool {
    let set = std::env::temp_dir().join(format!("qa-icon-{}.iconset", std::process::id()));
    let _ = std::fs::remove_dir_all(&set);
    if std::fs::create_dir_all(&set).is_err() {
        return false;
    }
    let sizes = [("16x16", 16), ("16x16@2x", 32), ("32x32", 32), ("32x32@2x", 64), ("128x128", 128), ("128x128@2x", 256), ("256x256", 256), ("256x256@2x", 512), ("512x512", 512), ("512x512@2x", 1024)];
    let mut ok = true;
    for (name, px) in sizes {
        let r = Command::new("sips").args(["-z", &px.to_string(), &px.to_string()]).arg(png).arg("--out").arg(set.join(format!("icon_{name}.png"))).output();
        ok &= r.is_ok_and(|o| o.status.success());
    }
    ok = ok && Command::new("iconutil").args(["-c", "icns"]).arg(&set).arg("-o").arg(out).output().is_ok_and(|o| o.status.success());
    let _ = std::fs::remove_dir_all(&set);
    ok
}

fn lsregister(app: &Path) {
    let tool = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let _ = Command::new(tool).arg("-f").arg(app).output();
}

pub fn write_desktop(ctx: &AppCtx, m: &Manifest) -> Result<PathBuf, String> {
    let dir = applications_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = format!("app.{}", ctx.id);
    let bundle_id = format!("{PREFIX}.{id}");
    let base = file_name(&m.name);
    // Updating keeps the bundle it wrote before; another app with the same name gets the id.
    let mut app = desktop_path(ctx);
    if !app.exists() || bundle_id_of(&app).as_deref() != Some(&bundle_id) {
        app = dir.join(format!("{base}.app"));
        if app.exists() && bundle_id_of(&app).as_deref() != Some(&bundle_id) {
            app = dir.join(format!("{base} ({}).app", &ctx.id[..ctx.id.len().min(4)]));
        }
    }
    let icon = super::store::bundle_dir(ctx).join("icon.png");
    let line = format!("{} run {}", sh(&exe()), sh(&ctx.source));
    make_bundle(&app, &id, &m.name, &m.version, &line, icon.exists().then_some(icon.as_path()), "", false)?;
    std::fs::write(ctx.home.join("bundle_path"), app.display().to_string()).map_err(|e| e.to_string())?;
    Ok(app)
}

/// Makes `qa:` links open with `qa url-handler`, through a bundle that declares the scheme.
pub fn register_handler() -> Result<PathBuf, String> {
    let dir = applications_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let app = dir.join("Quick Apps.app");
    let line = format!("{} url-handler", sh(&exe()));
    let current = std::fs::read_to_string(app.join("Contents/MacOS/launcher")).ok();
    if current.as_deref() != Some(&format!("#!/bin/sh\nexec {line}\n")) || bundle_id_of(&app).is_none() {
        let urls = "<key>CFBundleURLTypes</key><array><dict><key>CFBundleURLName</key><string>Quick Apps link</string><key>CFBundleURLSchemes</key><array><string>qa</string></array></dict></array>\n";
        make_bundle(&app, "opener", "Quick Apps", "1", &line, None, urls, true)?;
    }
    Ok(app)
}
