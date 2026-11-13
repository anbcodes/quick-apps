//! The per-app grants file: one grant per line, human-editable.
//!
//! ```text
//! internet
//! folder rw /home/me/notes
//! memory 209715200
//! cpu
//! message https://apps.example.com/calendar/app.json
//! ```

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum Grant {
    Internet,
    Folder { write: bool, path: String },
    Memory(u64),
    Cpu,
    Message(String),
    Other(String),
}

impl Grant {
    pub fn parse(line: &str) -> Option<Grant> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let mut it = line.splitn(3, ' ');
        Some(match it.next()? {
            "internet" => Grant::Internet,
            "cpu" => Grant::Cpu,
            "memory" => Grant::Memory(it.next()?.parse().ok()?),
            "message" => Grant::Message(line[8..].trim().to_string()),
            "folder" => {
                let mode = it.next()?;
                Grant::Folder { write: mode == "rw", path: it.next()?.to_string() }
            }
            _ => Grant::Other(line.to_string()),
        })
    }

    pub fn line(&self) -> String {
        match self {
            Grant::Internet => "internet".into(),
            Grant::Cpu => "cpu".into(),
            Grant::Memory(n) => format!("memory {n}"),
            Grant::Message(t) => format!("message {t}"),
            Grant::Folder { write, path } => format!("folder {} {path}", if *write { "rw" } else { "r" }),
            Grant::Other(s) => s.clone(),
        }
    }
}

pub struct Grants {
    pub path: PathBuf,
    pub list: Vec<Grant>,
    /// Granted for this run only (`--grant`); never saved.
    pub session: Vec<Grant>,
}

impl Grants {
    pub fn load(path: PathBuf) -> Grants {
        let list = std::fs::read_to_string(&path).map(|s| s.lines().filter_map(Grant::parse).collect()).unwrap_or_default();
        Grants { path, list, session: Vec::new() }
    }

    pub fn save(&self) {
        if let Some(d) = self.path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mut s = String::new();
        for g in &self.list {
            s.push_str(&g.line());
            s.push('\n');
        }
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    pub fn add(&mut self, g: Grant) {
        // A newer memory grant replaces the old one.
        if let Grant::Memory(_) = g {
            self.list.retain(|x| !matches!(x, Grant::Memory(_)));
        }
        if !self.list.contains(&g) {
            self.list.push(g);
            self.save();
        }
    }

    fn all(&self) -> impl Iterator<Item = &Grant> {
        self.list.iter().chain(&self.session)
    }

    pub fn internet(&self) -> bool {
        self.all().any(|g| *g == Grant::Internet)
    }

    pub fn cpu(&self) -> bool {
        self.all().any(|g| *g == Grant::Cpu)
    }

    pub fn memory(&self) -> Option<u64> {
        self.all().filter_map(|g| if let Grant::Memory(n) = g { Some(*n) } else { None }).max()
    }

    pub fn message(&self, to: &str) -> bool {
        self.all().any(|g| matches!(g, Grant::Message(t) if t == to))
    }

    /// A folder grant covering `path` with at least the given access.
    pub fn folder(&self, path: &str, write: bool) -> bool {
        self.all().any(|g| matches!(g, Grant::Folder { write: w, path: p } if p == path && (*w || !write)))
    }
}
