//! Input discovery and output path mapping.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub src: PathBuf,
    pub dst: PathBuf,
}

#[derive(Debug, Clone)]
pub enum Skip {
    UnsupportedExt(PathBuf),
    TooLarge(PathBuf, u64),
    Unreadable(PathBuf, String),
}

#[derive(Clone, Copy, Debug)]
pub enum OutMode<'a> {
    Dir(&'a Path),
    InPlace,
}

/// `foo.rs` becomes `foo.rs.pdf`, so `foo.rs` and `foo.go` cannot collide.
fn with_pdf(p: &Path) -> PathBuf {
    let mut s: OsString = p.as_os_str().to_os_string();
    s.push(".pdf");
    PathBuf::from(s)
}

/// Collect every convertible file under `roots`.
///
/// Under `OutMode::Dir`, each directory root is mirrored into a subdirectory
/// named for that root's final component, so two roots that both contain
/// `src/main.rs` cannot overwrite each other. A file root maps directly into
/// the output directory with no subdirectory.
pub fn discover(
    roots: &[PathBuf],
    out: OutMode<'_>,
    exts: &BTreeSet<String>,
    max_bytes: u64,
) -> (Vec<Job>, Vec<Skip>) {
    let mut jobs = Vec::new();
    let mut skips = Vec::new();

    for root in roots {
        let meta = match std::fs::symlink_metadata(root) {
            Ok(m) => m,
            Err(e) => {
                skips.push(Skip::Unreadable(root.clone(), e.to_string()));
                continue;
            }
        };
        if meta.is_file() {
            let dst = match out {
                OutMode::InPlace => with_pdf(root),
                OutMode::Dir(d) => with_pdf(&d.join(root.file_name().unwrap_or_default())),
            };
            consider(root, dst, exts, max_bytes, &mut jobs, &mut skips);
        } else if meta.is_dir() {
            let prefix = match out {
                OutMode::InPlace => None,
                OutMode::Dir(d) => {
                    Some(d.join(root.file_name().unwrap_or(std::ffi::OsStr::new("root"))))
                }
            };
            walk_dir(
                root,
                root,
                prefix.as_deref(),
                exts,
                max_bytes,
                &mut jobs,
                &mut skips,
            );
        } else {
            // A symlinked root: not followed, by the same rule as inner entries.
            skips.push(Skip::Unreadable(
                root.clone(),
                "symlinks are not followed".into(),
            ));
        }
    }

    jobs.sort_by(|a, b| a.src.cmp(&b.src));
    (jobs, skips)
}

#[allow(clippy::too_many_arguments)]
fn walk_dir(
    dir: &Path,
    root: &Path,
    prefix: Option<&Path>,
    exts: &BTreeSet<String>,
    max_bytes: u64,
    jobs: &mut Vec<Job>,
    skips: &mut Vec<Skip>,
) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            skips.push(Skip::Unreadable(dir.to_path_buf(), e.to_string()));
            return;
        }
    };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for e in entries {
        let Ok(ft) = e.file_type() else { continue };
        // Never follow symlinks: this removes cycle risk with no inode tracking.
        if ft.is_symlink() {
            continue;
        }
        let path = e.path();
        if ft.is_dir() {
            walk_dir(&path, root, prefix, exts, max_bytes, jobs, skips);
        } else if ft.is_file() {
            let dst = match prefix {
                None => with_pdf(&path),
                Some(p) => {
                    let rel = path.strip_prefix(root).unwrap_or(&path);
                    with_pdf(&p.join(rel))
                }
            };
            consider(&path, dst, exts, max_bytes, jobs, skips);
        }
    }
}

fn consider(
    src: &Path,
    dst: PathBuf,
    exts: &BTreeSet<String>,
    max_bytes: u64,
    jobs: &mut Vec<Job>,
    skips: &mut Vec<Skip>,
) {
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ext.is_empty() || !exts.contains(&ext) {
        skips.push(Skip::UnsupportedExt(src.to_path_buf()));
        return;
    }
    match std::fs::metadata(src) {
        Ok(m) if m.len() > max_bytes => skips.push(Skip::TooLarge(src.to_path_buf(), m.len())),
        Ok(_) => jobs.push(Job {
            src: src.to_path_buf(),
            dst,
        }),
        Err(e) => skips.push(Skip::Unreadable(src.to_path_buf(), e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;

    fn exts() -> BTreeSet<String> {
        ["rs", "py", "txt"].iter().map(|s| s.to_string()).collect()
    }

    fn tree(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("to_pdf_walk_{tag}"));
        let _ = fs::remove_dir_all(&root);
        for (rel, body) in files {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, body).unwrap();
        }
        root
    }

    #[test]
    fn a_single_file_input_maps_into_the_out_dir_without_a_subdirectory() {
        let root = tree("single", &[("main.rs", "fn main() {}")]);
        let out = root.join("out");
        let (jobs, _) = discover(
            &[root.join("main.rs")],
            OutMode::Dir(&out),
            &exts(),
            1 << 20,
        );
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].dst, out.join("main.rs.pdf"));
    }

    #[test]
    fn the_source_extension_is_kept_so_siblings_cannot_collide() {
        let root = tree("collide", &[("a/x.rs", "1"), ("a/x.py", "2")]);
        let out = root.join("out");
        let (jobs, _) = discover(&[root.join("a")], OutMode::Dir(&out), &exts(), 1 << 20);
        let names: Vec<_> = jobs
            .iter()
            .map(|j| j.dst.file_name().unwrap().to_owned())
            .collect();
        assert!(names.contains(&"x.rs.pdf".into()));
        assert!(names.contains(&"x.py.pdf".into()));
    }

    #[test]
    fn subdirectories_are_walked_recursively_and_structure_is_mirrored() {
        let root = tree(
            "deep",
            &[("s/a.rs", "1"), ("s/b/c.rs", "2"), ("s/b/d/e.rs", "3")],
        );
        let out = root.join("out");
        let (jobs, _) = discover(&[root.join("s")], OutMode::Dir(&out), &exts(), 1 << 20);
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[2].dst, out.join("s/b/d/e.rs.pdf"));
    }

    #[test]
    fn two_roots_with_the_same_inner_path_do_not_collide() {
        let root = tree(
            "roots",
            &[("api/src/main.rs", "1"), ("web/src/main.rs", "2")],
        );
        let out = root.join("out");
        let (jobs, _) = discover(
            &[root.join("api"), root.join("web")],
            OutMode::Dir(&out),
            &exts(),
            1 << 20,
        );
        let dsts: Vec<_> = jobs.iter().map(|j| j.dst.clone()).collect();
        assert!(dsts.contains(&out.join("api/src/main.rs.pdf")));
        assert!(dsts.contains(&out.join("web/src/main.rs.pdf")));
    }

    #[test]
    fn in_place_writes_next_to_the_source() {
        let root = tree("inplace", &[("a.rs", "1")]);
        let (jobs, _) = discover(
            std::slice::from_ref(&root),
            OutMode::InPlace,
            &exts(),
            1 << 20,
        );
        assert_eq!(jobs[0].dst, root.join("a.rs.pdf"));
    }

    #[test]
    fn unsupported_extensions_are_skipped_and_reported() {
        let root = tree("skip", &[("a.rs", "1"), ("b.bin", "2"), ("c", "3")]);
        let (jobs, skips) = discover(
            std::slice::from_ref(&root),
            OutMode::InPlace,
            &exts(),
            1 << 20,
        );
        assert_eq!(jobs.len(), 1);
        assert_eq!(skips.len(), 2);
        assert!(skips.iter().all(|s| matches!(s, Skip::UnsupportedExt(_))));
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        let root = tree("case", &[("A.RS", "1")]);
        let (jobs, _) = discover(
            std::slice::from_ref(&root),
            OutMode::InPlace,
            &exts(),
            1 << 20,
        );
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn oversized_files_are_skipped_and_reported() {
        let root = tree("big", &[("a.rs", "0123456789")]);
        let (jobs, skips) = discover(std::slice::from_ref(&root), OutMode::InPlace, &exts(), 5);
        assert!(jobs.is_empty());
        assert!(matches!(skips[0], Skip::TooLarge(_, 10)));
    }

    #[test]
    fn job_order_is_deterministic() {
        let root = tree("order", &[("z.rs", "1"), ("a.rs", "2"), ("m/q.rs", "3")]);
        let (a, _) = discover(
            std::slice::from_ref(&root),
            OutMode::InPlace,
            &exts(),
            1 << 20,
        );
        let (b, _) = discover(
            std::slice::from_ref(&root),
            OutMode::InPlace,
            &exts(),
            1 << 20,
        );
        assert_eq!(a, b);
        let srcs: Vec<_> = a.iter().map(|j| j.src.clone()).collect();
        let mut sorted = srcs.clone();
        sorted.sort();
        assert_eq!(srcs, sorted);
    }

    #[test]
    fn an_unreadable_root_is_reported_not_panicked() {
        let missing = std::env::temp_dir().join("to_pdf_walk_nope_does_not_exist");
        let (jobs, skips) = discover(&[missing], OutMode::InPlace, &exts(), 1 << 20);
        assert!(jobs.is_empty());
        assert!(matches!(skips[0], Skip::Unreadable(_, _)));
    }
}
