//! App identity and on-disk layout (docs/packaging.md).

#[cfg(target_os = "macos")]
pub mod macapp;
pub mod json;
pub mod sha256;
pub mod store;
pub mod zip;

use std::path::{Path, PathBuf};

pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"))
        .join("quick-apps")
}

/// The first 16 hex chars of sha256(source URL).
pub fn app_id(source: &str) -> String {
    sha256::hex(&sha256::digest(source.as_bytes()))[..16].to_string()
}

/// The window's app id (and WM_CLASS on X11): `qa.` and the name in lowercase
/// with dashes, like `qa.slack-lite`. Bars and switchers that show the app id
/// as it is, or its last dotted part, then show something a person can read.
pub fn window_class(name: &str) -> String {
    let slug: String = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let slug = slug.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    format!("qa.{}", if slug.is_empty() { "app" } else { &slug })
}

/// Who an app is and where its things live.
#[derive(Clone, Debug)]
pub struct AppCtx {
    pub id: String,
    /// The manifest URL, or file://<path> for local apps.
    pub source: String,
    pub name: String,
    /// ~/.local/share/quick-apps/apps/<id>
    pub home: PathBuf,
    /// Handle 1: the bundle folder (read-only).
    pub bundle: Option<PathBuf>,
}

impl AppCtx {
    pub fn new(source: &str, name: &str, bundle: Option<PathBuf>) -> AppCtx {
        let id = app_id(source);
        let home = data_home().join("apps").join(&id);
        AppCtx { id, source: source.to_string(), name: name.to_string(), home, bundle }
    }

    pub fn local(path: &Path, name: &str) -> AppCtx {
        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let bundle = if abs.is_dir() { Some(abs.clone()) } else { abs.parent().map(|p| p.to_path_buf()) };
        AppCtx::new(&format!("file://{}", abs.display()), name, bundle)
    }

    pub fn data_dir(&self) -> PathBuf {
        self.home.join("data")
    }

    pub fn grants_path(&self) -> PathBuf {
        self.home.join("grants")
    }
}
