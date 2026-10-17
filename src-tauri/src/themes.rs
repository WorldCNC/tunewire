//! Themes are JSON files: a map of CSS custom properties plus optional extra CSS.
//! Built-ins are compiled in; user themes live in `<data>/themes/*.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::store::write_atomic;

const BUILTIN: &[&str] = &[
    include_str!("../themes/midnight.json"),
    include_str!("../themes/daylight.json"),
    include_str!("../themes/synthwave.json"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default = "yes")]
    pub dark: bool,
    pub vars: BTreeMap<String, String>,
    #[serde(default)]
    pub css: String,
    #[serde(default, skip_deserializing)]
    pub builtin: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct ThemeMeta {
    pub id: String,
    pub name: String,
    pub author: String,
    pub dark: bool,
    pub builtin: bool,
    pub accent: Option<String>,
    pub bg: Option<String>,
}

impl From<&Theme> for ThemeMeta {
    fn from(t: &Theme) -> Self {
        ThemeMeta {
            id: t.id.clone(),
            name: t.name.clone(),
            author: t.author.clone(),
            dark: t.dark,
            builtin: t.builtin,
            accent: t.vars.get("--accent").cloned(),
            bg: t.vars.get("--bg").cloned(),
        }
    }
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').chars().take(48).collect::<String>();
    if out.is_empty() {
        "theme".into()
    } else {
        out
    }
}

/// Reject anything that could escape a CSS declaration or pull remote content.
fn validate(t: &mut Theme) -> AppResult<()> {
    if t.name.trim().is_empty() {
        return Err(AppError::Theme("theme needs a name".into()));
    }
    t.name = t.name.trim().chars().take(64).collect();
    t.author = t.author.trim().chars().take(64).collect();
    if t.vars.len() > 200 {
        return Err(AppError::Theme("too many variables".into()));
    }
    for (k, v) in &t.vars {
        let key_ok = k.starts_with("--") && k.len() < 64 && k[2..].chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !key_ok {
            return Err(AppError::Theme(format!("invalid variable name '{k}'")));
        }
        let lower = v.to_lowercase();
        if v.len() > 300 || v.contains([';', '{', '}', '<', '>']) || lower.contains("url(") || lower.contains("expression(") {
            return Err(AppError::Theme(format!("invalid value for {k}")));
        }
    }
    if t.css.len() > 64 * 1024 {
        return Err(AppError::Theme("custom CSS is too large (max 64 KB)".into()));
    }
    let lower = t.css.to_lowercase();
    if lower.contains("@import") || lower.contains("</style") || lower.contains("url(http") || lower.contains("url('http") || lower.contains("url(\"http") {
        return Err(AppError::Theme("custom CSS may not use @import or remote url()".into()));
    }
    Ok(())
}

pub struct ThemeStore {
    dir: PathBuf,
}

impl ThemeStore {
    pub fn new(dir: PathBuf) -> ThemeStore {
        let _ = std::fs::create_dir_all(&dir);
        ThemeStore { dir }
    }

    fn builtins() -> Vec<Theme> {
        BUILTIN
            .iter()
            .filter_map(|s| serde_json::from_str::<Theme>(s).ok())
            .map(|mut t| {
                t.builtin = true;
                if t.id.is_empty() {
                    t.id = slug(&t.name);
                }
                t
            })
            .collect()
    }

    fn user(&self) -> Vec<Theme> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut v: Vec<Theme> = rd
            .flatten()
            .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
            .filter_map(|e| {
                let s = std::fs::read_to_string(e.path()).ok()?;
                let mut t: Theme = serde_json::from_str(&s).ok()?;
                t.id = e.path().file_stem()?.to_string_lossy().into_owned();
                validate(&mut t).ok()?;
                Some(t)
            })
            .collect();
        v.sort_by_key(|a| a.name.to_lowercase());
        v
    }

    pub fn list(&self) -> Vec<ThemeMeta> {
        Self::builtins().iter().chain(self.user().iter()).map(ThemeMeta::from).collect()
    }

    pub fn get(&self, id: &str) -> AppResult<Theme> {
        Self::builtins()
            .into_iter()
            .chain(self.user())
            .find(|t| t.id == id)
            .ok_or_else(|| AppError::NotFound(format!("theme '{id}'")))
    }

    /// Save (create or overwrite) a user theme. Built-ins are never overwritten:
    /// editing one saves a copy.
    pub fn save(&self, mut t: Theme) -> AppResult<ThemeMeta> {
        validate(&mut t)?;
        let builtin_ids: Vec<String> = Self::builtins().into_iter().map(|b| b.id).collect();
        let mut id = if t.id.is_empty() { slug(&t.name) } else { slug(&t.id) };
        if builtin_ids.contains(&id) {
            id = format!("{id}-custom");
        }
        t.id = id.clone();
        t.builtin = false;
        let data = serde_json::to_vec_pretty(&t)?;
        write_atomic(&self.dir.join(format!("{id}.json")), &data)?;
        Ok(ThemeMeta::from(&t))
    }

    pub fn delete(&self, id: &str) -> AppResult<()> {
        let id = slug(id);
        let p = self.dir.join(format!("{id}.json"));
        if !p.exists() {
            return Err(AppError::NotFound(format!("user theme '{id}' (built-in themes cannot be deleted)")));
        }
        std::fs::remove_file(p)?;
        Ok(())
    }

    pub fn export_json(&self, id: &str) -> AppResult<String> {
        let mut t = self.get(id)?;
        t.builtin = false;
        Ok(serde_json::to_string_pretty(&t)?)
    }

    pub fn import_json(&self, json: &str) -> AppResult<ThemeMeta> {
        if json.len() > 256 * 1024 {
            return Err(AppError::Theme("file too large".into()));
        }
        let mut t: Theme = serde_json::from_str(json).map_err(|e| AppError::Theme(format!("not a valid theme file: {e}")))?;
        if t.id.is_empty() {
            t.id = slug(&t.name);
        }
        // Don't clobber an existing theme with the same id.
        let mut id = slug(&t.id);
        let mut n = 2;
        while self.dir.join(format!("{id}.json")).exists() {
            id = format!("{}-{n}", slug(&t.id));
            n += 1;
        }
        t.id = id;
        self.save(t)
    }
}
