//! Clean-room guard (docs/CLEAN_ROOM.md §2.5): XDAW code, specs and README must
//! not reference third-party products, their manuals or their file formats.
//! Provenance records (docs/CLEAN_ROOM.md, docs/research/, NOTICE) are exempt.

use std::path::{Path, PathBuf};

/// Case-insensitive substrings that must not appear.
const TERMS: &[&str] = &[
    "ableton",
    "live 9",
    "live 10",
    "live 11",
    "live 12",
    "live-style",
    "max for live",
    "drum rack",
    "drumrack",
    "simpler",
    "reference manual",
    "manual §",
];

/// File extensions of third-party project/preset formats.
const EXTENSIONS: &[&str] = &["als", "alc", "adg", "adv", "alp", "amxd"];

/// Lines allowed to keep a term, for backward compatibility with old projects.
const ALLOWED: &[&str] = &["#[serde(alias = \"DrumRack\")]", "serde_json::from_str(\"\\\"DrumRack\\\"\")"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        if name == "target" || name.to_string_lossy().starts_with('.') {
            continue;
        }
        if p.is_dir() {
            collect(&p, out);
        } else if matches!(p.extension().and_then(|x| x.to_str()), Some("rs" | "md" | "toml" | "sh" | "json")) {
            out.push(p);
        }
    }
}

fn has_extension_ref(line: &str, ext: &str) -> bool {
    let needle = format!(".{ext}");
    line.match_indices(&needle).any(|(i, _)| {
        let after = line[i + needle.len()..].chars().next();
        !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

#[test]
fn code_specs_and_readme_do_not_reference_third_party_products() {
    let root = root();
    let mut files = vec![root.join("README.md")];
    for dir in ["crates", "docs/specs", "scripts"] {
        collect(&root.join(dir), &mut files);
    }
    let me = Path::new(file!()).file_name().unwrap();
    let mut hits = Vec::new();
    for f in files.iter().filter(|f| f.file_name() != Some(me)) {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        for (n, line) in text.lines().enumerate() {
            if ALLOWED.iter().any(|a| line.contains(a)) {
                continue;
            }
            let lower = line.to_lowercase();
            let term = TERMS.iter().find(|t| lower.contains(*t)).copied();
            let ext = EXTENSIONS.iter().find(|e| has_extension_ref(&lower, e)).copied();
            if let Some(what) = term.or(ext) {
                hits.push(format!("{}:{}: '{}'", f.strip_prefix(&root).unwrap_or(f).display(), n + 1, what));
            }
        }
    }
    assert!(files.len() > 50, "scan found only {} files; wrong root?", files.len());
    assert!(hits.is_empty(), "third-party references (see docs/CLEAN_ROOM.md):\n{}", hits.join("\n"));
}

#[test]
fn extension_matcher_ignores_longer_words() {
    assert!(has_extension_ref("open song.als now", "als"));
    assert!(!has_extension_ref("x.alsa_device", "als"));
}
