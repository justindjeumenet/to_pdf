//! Parallel orchestration: discover once, convert independently, report once.

use crate::cli::Cli;
use crate::layout::{self, Geometry};
use crate::pdf::{self, PdfOptions, encode};
use crate::walk::{self, Job, Skip};
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Default)]
pub struct Summary {
    pub converted: usize,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// Files in which at least one character had no WinAnsi slot, in job order.
    pub escaped: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub skipped: Vec<Skip>,
}

struct Outcome {
    bytes_in: u64,
    bytes_out: u64,
    escaped: bool,
}

/// Everything a worker needs, so no worker touches `Cli` or shared mutable state.
struct Config {
    geometry: Geometry,
    pdf: PdfOptions,
    tab_width: usize,
    line_numbers: bool,
    unmappable: encode::Unmappable,
}

pub fn run(cli: &Cli) -> Summary {
    let exts = cli.extensions();
    let (jobs, skipped) = walk::discover(
        &cli.inputs,
        cli.out_mode(),
        &exts,
        cli.max_file_size.saturating_mul(1024 * 1024),
        cli.descend(),
    );

    let mut summary = Summary {
        skipped,
        ..Summary::default()
    };
    if cli.dry_run {
        summary.converted = jobs.len();
        return summary;
    }

    let cfg = Config {
        geometry: cli.geometry(),
        pdf: cli.pdf_options(),
        tab_width: cli.tab_width,
        line_numbers: cli.line_numbers,
        unmappable: cli.unmappable(),
    };

    let abort = AtomicBool::new(false);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(cli.jobs.unwrap_or(0))
        .build()
        .expect("building a rayon thread pool cannot fail with a valid thread count");

    let results: Vec<(PathBuf, Result<Outcome, String>)> = pool.install(|| {
        jobs.par_iter()
            .filter_map(|job| {
                if abort.load(Ordering::Relaxed) {
                    return None;
                }
                let r = convert(job, &cfg);
                if r.is_err() && cli.fail_fast {
                    abort.store(true, Ordering::Relaxed);
                }
                Some((job.src.clone(), r))
            })
            .collect()
    });

    for (src, r) in results {
        match r {
            Ok(o) => {
                summary.converted += 1;
                summary.bytes_in += o.bytes_in;
                summary.bytes_out += o.bytes_out;
                if o.escaped {
                    summary.escaped.push(src);
                }
            }
            Err(e) => summary.failed.push((src, e)),
        }
    }
    summary
}

fn convert(job: &Job, cfg: &Config) -> Result<Outcome, String> {
    let bytes = std::fs::read(&job.src).map_err(|e| e.to_string())?;
    let raw = crate::extract::extract(&job.src, &bytes)?;

    let mut escaped = false;
    let mut lines = Vec::with_capacity(raw.len());
    for l in raw {
        // Tabs must become spaces *before* transcoding. A raw tab has no WinAnsi
        // slot, so transcoding first escapes it as `\u{9}` and destroys the
        // indentation of every tab-indented file.
        let expanded = layout::expand_tabs(&l, cfg.tab_width);
        let (line, touched) = encode::transcode_line(&expanded, cfg.unmappable).map_err(|c| {
            format!(
                "unmappable character U+{:04X} (--on-unmappable=fail)",
                c as u32
            )
        })?;
        escaped |= touched;
        lines.push(line);
    }

    let pages = layout::paginate(&lines, &cfg.geometry, cfg.line_numbers);
    let out = pdf::build(&pages, &cfg.pdf);

    if let Some(parent) = job.dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&job.dst, &out).map_err(|e| e.to_string())?;

    Ok(Outcome {
        bytes_in: bytes.len() as u64,
        bytes_out: out.len() as u64,
        escaped,
    })
}

impl Summary {
    pub fn report(&self, quiet: bool, verbose: bool, mode: encode::Unmappable) {
        if verbose {
            for s in &self.skipped {
                match s {
                    Skip::UnsupportedExt(p) => {
                        eprintln!("skip  {}: unsupported extension", p.display())
                    }
                    Skip::TooLarge(p, n) => {
                        eprintln!("skip  {}: {n} bytes exceeds --max-file-size", p.display())
                    }
                    Skip::IgnoredDir(p) => eprintln!(
                        "skip  {}/: ignored directory (--no-ignore to include)",
                        p.display()
                    ),
                    Skip::Unreadable(p, e) => eprintln!("skip  {}: {e}", p.display()),
                }
            }
        }
        for (p, e) in &self.failed {
            eprintln!("error {}: {e}", p.display());
        }
        if quiet {
            return;
        }
        let ratio = if self.bytes_in > 0 {
            100.0 * self.bytes_out as f64 / self.bytes_in as f64
        } else {
            0.0
        };
        // Ignored directories are one entry each, not one per file inside, so
        // folding them into the file count would be misleading.
        let ignored_dirs = self
            .skipped
            .iter()
            .filter(|s| matches!(s, Skip::IgnoredDir(_)))
            .count();
        let dirs = if ignored_dirs > 0 {
            format!(", {ignored_dirs} dirs ignored")
        } else {
            String::new()
        };
        println!(
            "{} converted, {} skipped{dirs}, {} failed — {} in, {} out ({ratio:.0}% of source)",
            self.converted,
            self.skipped.len() - ignored_dirs,
            self.failed.len(),
            human(self.bytes_in),
            human(self.bytes_out),
        );
        if let Some(note) = self.unmappable_notice(mode) {
            println!("{note}");
        }
    }

    /// The trailing notice about characters WinAnsi could not represent, naming
    /// each affected file. `None` when everything mapped cleanly.
    ///
    /// The wording has to come from `mode`: the same files are recorded whether
    /// their characters were escaped or replaced, so only the mode says which
    /// actually happened.
    pub fn unmappable_notice(&self, mode: encode::Unmappable) -> Option<String> {
        if self.escaped.is_empty() {
            return None;
        }
        let what = match mode {
            encode::Unmappable::Escape => "escaped as \\u{...}",
            encode::Unmappable::Fold => "folded to ASCII, or escaped where no spelling fits",
            encode::Unmappable::Replace => "replaced with '?'",
            // Unreachable: under `fail` an offending file lands in `failed`.
            encode::Unmappable::Fail => "that WinAnsi cannot represent",
        };
        let mut note = format!(
            "{} file(s) contained characters {what}:",
            self.escaped.len()
        );
        for p in &self.escaped {
            note.push_str(&format!("\n  {}", p.display()));
        }
        Some(note)
    }
}

fn human(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;
    use std::fs;

    fn scratch(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("to_pdf_pipe_{tag}"));
        let _ = fs::remove_dir_all(&root);
        for (rel, body) in files {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, body).unwrap();
        }
        root
    }

    fn run_on(args: &[&str]) -> Summary {
        let mut v = vec!["to_pdf"];
        v.extend_from_slice(args);
        run(&Cli::parse_from(v))
    }

    #[test]
    fn converts_a_tree_and_mirrors_it() {
        let root = scratch(
            "tree",
            &[("a.rs", "fn main() {}\n"), ("sub/b.py", "x = 1\n")],
        );
        let out = root.join("out");
        let s = run_on(&[root.to_str().unwrap(), "-o", out.to_str().unwrap()]);
        assert_eq!(s.converted, 2);
        assert!(s.failed.is_empty());
        let name = root.file_name().unwrap();
        assert!(out.join(name).join("a.rs.pdf").exists());
        assert!(out.join(name).join("sub/b.py.pdf").exists());
    }

    #[test]
    fn output_files_start_with_a_pdf_header() {
        let root = scratch("hdr", &[("a.rs", "fn main() {}\n")]);
        run_on(&[root.to_str().unwrap(), "--in-place"]);
        let bytes = fs::read(root.join("a.rs.pdf")).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.5\n"));
    }

    #[test]
    fn dry_run_writes_nothing() {
        let root = scratch("dry", &[("a.rs", "fn main() {}\n")]);
        let s = run_on(&[root.to_str().unwrap(), "--in-place", "--dry-run"]);
        assert_eq!(s.converted, 1);
        assert!(!root.join("a.rs.pdf").exists());
    }

    #[test]
    fn unsupported_files_are_skipped_not_failed() {
        let root = scratch("skip", &[("a.rs", "1\n"), ("b.bin", "2\n")]);
        let s = run_on(&[root.to_str().unwrap(), "--in-place"]);
        assert_eq!(s.converted, 1);
        assert!(s.failed.is_empty());
        assert_eq!(s.skipped.len(), 1);
    }

    #[test]
    fn a_bad_epub_fails_that_file_without_aborting_the_run() {
        let root = scratch("bad", &[("a.rs", "1\n"), ("b.epub", "not a zip\n")]);
        let s = run_on(&[root.to_str().unwrap(), "--in-place"]);
        assert_eq!(s.converted, 1);
        assert_eq!(s.failed.len(), 1);
        assert!(s.failed[0].0.ends_with("b.epub"));
    }

    #[test]
    fn escaped_files_are_recorded_by_path() {
        let root = scratch(
            "esc",
            &[("a.py", "s = \"\u{1F600}\"\n"), ("b.py", "s = 1\n")],
        );
        let s = run_on(&[root.to_str().unwrap(), "--in-place"]);
        assert_eq!(s.converted, 2);
        assert_eq!(s.escaped.len(), 1);
        assert!(s.escaped[0].ends_with("a.py"), "got: {:?}", s.escaped);
    }

    #[test]
    fn the_escape_notice_names_every_affected_file() {
        let root = scratch("escnote", &[("a.py", "s = \"\u{1F600}\"\n")]);
        let s = run_on(&[root.to_str().unwrap(), "--in-place"]);
        let note = s.unmappable_notice(encode::Unmappable::Escape).unwrap();
        assert!(note.contains("escaped as \\u{...}"), "got: {note}");
        assert!(note.contains("a.py"), "got: {note}");
    }

    #[test]
    fn the_replace_notice_says_replaced_rather_than_escaped() {
        let root = scratch("repnote", &[("a.py", "s = \"\u{1F600}\"\n")]);
        let s = run_on(&[
            root.to_str().unwrap(),
            "--in-place",
            "--on-unmappable",
            "replace",
        ]);
        assert_eq!(s.escaped.len(), 1);
        let note = s.unmappable_notice(encode::Unmappable::Replace).unwrap();
        assert!(note.contains("replaced with '?'"), "got: {note}");
        assert!(!note.contains("\\u{"), "got: {note}");
    }

    #[test]
    fn the_fold_notice_says_folded_rather_than_escaped() {
        let root = scratch("foldnote", &[("a.py", "s = \"\u{FB01}\"\n")]);
        let s = run_on(&[
            root.to_str().unwrap(),
            "--in-place",
            "--on-unmappable",
            "fold",
        ]);
        assert_eq!(s.escaped.len(), 1);
        let note = s.unmappable_notice(encode::Unmappable::Fold).unwrap();
        assert!(note.contains("folded"), "got: {note}");
    }

    #[test]
    fn a_clean_run_produces_no_unmappable_notice() {
        let root = scratch("clean", &[("a.py", "s = 1\n")]);
        let s = run_on(&[root.to_str().unwrap(), "--in-place"]);
        assert!(s.unmappable_notice(encode::Unmappable::Escape).is_none());
    }

    #[test]
    fn on_unmappable_fail_turns_the_file_into_a_failure() {
        let root = scratch("fail", &[("a.py", "s = \"\u{1F600}\"\n")]);
        let s = run_on(&[
            root.to_str().unwrap(),
            "--in-place",
            "--on-unmappable",
            "fail",
        ]);
        assert_eq!(s.converted, 0);
        assert_eq!(s.failed.len(), 1);
        assert!(s.failed[0].1.contains("U+1F600"), "got: {}", s.failed[0].1);
    }

    #[test]
    fn byte_totals_are_tracked() {
        let root = scratch("bytes", &[("a.rs", "fn main() {}\n")]);
        let s = run_on(&[root.to_str().unwrap(), "--in-place"]);
        assert_eq!(s.bytes_in, 13);
        assert!(s.bytes_out > 0);
    }

    #[test]
    fn a_second_run_over_the_same_tree_produces_identical_bytes() {
        let root = scratch("det", &[("a.rs", "fn main() {}\n")]);
        run_on(&[root.to_str().unwrap(), "--in-place"]);
        let first = fs::read(root.join("a.rs.pdf")).unwrap();
        run_on(&[root.to_str().unwrap(), "--in-place"]);
        assert_eq!(fs::read(root.join("a.rs.pdf")).unwrap(), first);
    }
}
