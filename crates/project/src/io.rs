//! Atomic save, versioned load, autosave/recovery and media collection.

use crate::model::{MediaRef, Project, FORMAT_NAME, FORMAT_VERSION};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const EXTENSION: &str = "ostp";

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: std::io::Error },
    #[error("{path} is not a valid project: {source}")]
    Parse { path: PathBuf, source: serde_json::Error },
    #[error("{path} uses format '{format}' version {version}; this build reads '{FORMAT_NAME}' up to version {FORMAT_VERSION}")]
    Unsupported { path: PathBuf, format: String, version: u32 },
}

/// Writes `bytes` atomically: temp file in the same directory, fsync, rename.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
    let err = |source| ProjectError::Write { path: path.to_path_buf(), source };
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(err)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("project"),
        std::process::id()
    ));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(err)
}

pub fn to_json(project: &Project) -> Vec<u8> {
    serde_json::to_vec_pretty(project).expect("project serialisation cannot fail")
}

pub fn save(project: &Project, path: &Path) -> Result<(), ProjectError> {
    atomic_write(path, &to_json(project))
}

pub fn load(path: &Path) -> Result<Project, ProjectError> {
    let bytes = fs::read(path).map_err(|source| ProjectError::Read { path: path.to_path_buf(), source })?;
    parse(path, &bytes)
}

pub fn parse(path: &Path, bytes: &[u8]) -> Result<Project, ProjectError> {
    #[derive(serde::Deserialize)]
    struct Header {
        format: String,
        version: u32,
    }
    let parse_err = |source| ProjectError::Parse { path: path.to_path_buf(), source };
    let header: Header = serde_json::from_slice(bytes).map_err(parse_err)?;
    if header.format != FORMAT_NAME || header.version > FORMAT_VERSION || header.version == 0 {
        return Err(ProjectError::Unsupported { path: path.to_path_buf(), format: header.format, version: header.version });
    }
    serde_json::from_slice(bytes).map_err(parse_err)
}

/// Autosave lives next to the project (`song.ostp` → `song.ostp.autosave`), or in
/// the app support directory for unsaved projects.
pub fn autosave_path(project_path: Option<&Path>) -> PathBuf {
    match project_path {
        Some(p) => {
            let mut s = p.as_os_str().to_owned();
            s.push(".autosave");
            PathBuf::from(s)
        }
        None => app_support_dir().join("untitled.ostp.autosave"),
    }
}

pub fn app_support_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Ostinato")
    } else {
        home.join(".local/share/ostinato")
    }
}

pub fn write_autosave(project: &Project, project_path: Option<&Path>) -> Result<PathBuf, ProjectError> {
    let path = autosave_path(project_path);
    atomic_write(&path, &to_json(project))?;
    Ok(path)
}

/// Returns an autosave that is newer than the saved project (crash recovery candidate).
pub fn recovery_candidate(project_path: Option<&Path>) -> Option<PathBuf> {
    let auto = autosave_path(project_path);
    let auto_time = fs::metadata(&auto).and_then(|m| m.modified()).ok()?;
    match project_path.and_then(|p| fs::metadata(p).and_then(|m| m.modified()).ok()) {
        Some(saved) if saved >= auto_time => None,
        _ => Some(auto),
    }
}

pub fn discard_autosave(project_path: Option<&Path>) {
    let _ = fs::remove_file(autosave_path(project_path));
}

/// Resolves a media reference against the project directory.
pub fn resolve_media(media: &MediaRef, project_dir: Option<&Path>) -> PathBuf {
    let p = Path::new(&media.path);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    project_dir.map(|d| d.join(p)).unwrap_or_else(|| p.to_path_buf())
}

/// Makes a media path relative to the project directory when it lies inside it.
pub fn relativize(path: &Path, project_dir: &Path) -> String {
    path.strip_prefix(project_dir).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

pub fn missing_media(project: &Project, project_dir: Option<&Path>) -> Vec<MediaRef> {
    project.media_refs().into_iter().filter(|m| !resolve_media(m, project_dir).is_file()).collect()
}

/// Copies every referenced file into `<project_dir>/Samples/` and rewrites the
/// references as relative paths. Returns the number of files copied.
pub fn collect_media(project: &mut Project, project_dir: &Path) -> Result<usize, ProjectError> {
    let samples = project_dir.join("Samples");
    let mut copied = 0;
    let mut error = None;
    let mut seen = std::collections::HashMap::<String, String>::new();
    project.for_each_media_mut(|m| {
        if error.is_some() {
            return;
        }
        if let Some(done) = seen.get(&m.path) {
            m.path = done.clone();
            return;
        }
        let src = resolve_media(m, Some(project_dir));
        if src.starts_with(&samples) || !src.is_file() {
            return;
        }
        let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "sample".into());
        let mut dst = samples.join(&name);
        let mut n = 1;
        while dst.exists() {
            dst = samples.join(format!("{n}-{name}"));
            n += 1;
        }
        let r = fs::create_dir_all(&samples).and_then(|_| fs::copy(&src, &dst));
        match r {
            Ok(_) => {
                copied += 1;
                let rel = relativize(&dst, project_dir);
                seen.insert(m.path.clone(), rel.clone());
                m.path = rel;
            }
            Err(source) => error = Some(ProjectError::Write { path: dst, source }),
        }
    });
    error.map_or(Ok(copied), Err)
}
