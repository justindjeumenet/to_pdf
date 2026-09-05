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
    pub escaped_files: usize,
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
                summary.escaped_files += usize::from(o.escaped);
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
        let (line, touched) = encode::transcode_line(&l, cfg.unmappable).map_err(|c| {
            format!(
                "unmappable character U+{:04X} (--on-unmappable=fail)",
                c as u32
            )
        })?;
        escaped |= touched;
        lines.push(line);
    }

    let pages = layout::paginate(&lines, &cfg.geometry, cfg.tab_width, cfg.line_numbers);
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
    pub fn report(&self, quiet: bool, verbose: bool) {
        if verbose {
            for s in &self.skipped {
                match s {
                    Skip::UnsupportedExt(p) => {
                        eprintln!("skip  {}: unsupported extension", p.display())
                    }
                    Skip::TooLarge(p, n) => {
                        eprintln!("skip  {}: {n} bytes exceeds --max-file-size", p.display())
                    }
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
        println!(
            "{} converted, {} skipped, {} failed — {} in, {} out ({ratio:.0}% of source)",
            self.converted,
            self.skipped.len(),
            self.failed.len(),
            human(self.bytes_in),
            human(self.bytes_out),
        );
        if self.escaped_files > 0 {
            println!(
                "{} file(s) contained characters escaped as \\u{{...}}",
                self.escaped_files
            );
        }
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
    fn escaped_files_are_counted() {
        let root = scratch(
            "esc",
            &[("a.py", "s = \"\u{1F600}\"\n"), ("b.py", "s = 1\n")],
        );
        let s = run_on(&[root.to_str().unwrap(), "--in-place"]);
        assert_eq!(s.converted, 2);
        assert_eq!(s.escaped_files, 1);
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
