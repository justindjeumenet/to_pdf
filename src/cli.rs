//! Command-line surface.

use crate::extract::DEFAULT_EXTS;
use crate::layout::Geometry;
use crate::pdf::PdfOptions;
use crate::pdf::encode::Unmappable;
use crate::walk::{Descend, OutMode};
use clap::{Parser, ValueEnum};
use std::collections::BTreeSet;
use std::path::PathBuf;

const MARGIN: f64 = 36.0;

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum PageSize {
    A4,
    Letter,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum UnmappableArg {
    Escape,
    Fold,
    Replace,
    Fail,
}

#[derive(Parser, Debug)]
#[command(
    name = "to_pdf",
    about = "Convert source files and trees to very small PDFs",
    version
)]
pub struct Cli {
    /// Files or directories to convert.
    #[arg(required = true)]
    pub inputs: Vec<PathBuf>,

    /// Output root directory.
    #[arg(short, long, value_name = "DIR", default_value = "pdf-out")]
    pub out: PathBuf,

    /// Write foo.rs.pdf beside foo.rs instead of into an output root.
    #[arg(long, conflicts_with = "out")]
    pub in_place: bool,

    /// Worker threads. Defaults to available parallelism.
    #[arg(short, long, value_name = "N")]
    pub jobs: Option<usize>,

    /// Font size in points. Overrides --dense.
    #[arg(long, value_name = "PT")]
    pub font_size: Option<f64>,

    /// 7pt font with 8pt leading: about 25% fewer pages.
    #[arg(long)]
    pub dense: bool,

    #[arg(long, value_enum, default_value_t = PageSize::A4)]
    pub page: PageSize,

    #[arg(long, value_name = "N", default_value_t = 4)]
    pub tab_width: usize,

    /// Prefix each line with its source line number.
    #[arg(long)]
    pub line_numbers: bool,

    /// Add a centred page number in the footer.
    #[arg(long)]
    pub page_numbers: bool,

    /// What to do with characters WinAnsi cannot represent.
    #[arg(long, value_enum, default_value_t = UnmappableArg::Escape)]
    pub on_unmappable: UnmappableArg,

    /// Comma-separated extension list replacing the default set.
    #[arg(long, value_name = "LIST", value_delimiter = ',')]
    pub ext: Option<Vec<String>>,

    /// Descend into hidden and vendor directories (.git, node_modules, target...).
    #[arg(long)]
    pub no_ignore: bool,

    /// Skip inputs larger than this, in megabytes.
    #[arg(long, value_name = "MB", default_value_t = 64)]
    pub max_file_size: u64,

    /// List what would be converted without writing anything.
    #[arg(long)]
    pub dry_run: bool,

    /// Stop at the first failure instead of collecting them.
    #[arg(long)]
    pub fail_fast: bool,

    #[arg(short, long)]
    pub quiet: bool,

    #[arg(short, long, conflicts_with = "quiet")]
    pub verbose: bool,
}

impl Cli {
    pub fn geometry(&self) -> Geometry {
        let (width, height) = match self.page {
            PageSize::A4 => (595.28, 841.89),
            PageSize::Letter => (612.0, 792.0),
        };
        // --dense is shorthand for a size/leading pair; an explicit --font-size
        // wins and derives its own leading.
        let (font_size, leading) = match (self.font_size, self.dense) {
            (Some(f), _) => (f, (f * 1.18 * 100.0).round() / 100.0),
            (None, true) => (7.0, 8.0),
            (None, false) => (8.5, 10.0),
        };
        Geometry {
            width,
            height,
            margin: MARGIN,
            font_size,
            leading,
        }
    }

    pub fn pdf_options(&self) -> PdfOptions {
        let g = self.geometry();
        PdfOptions {
            width: g.width,
            height: g.height,
            margin: g.margin,
            font_size: g.font_size,
            leading: g.leading,
            page_numbers: self.page_numbers,
        }
    }

    /// Lowercased, dot-stripped extensions. `--ext` replaces the default set.
    pub fn extensions(&self) -> BTreeSet<String> {
        match &self.ext {
            Some(list) => list
                .iter()
                .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
                .filter(|e| !e.is_empty())
                .collect(),
            None => DEFAULT_EXTS.iter().map(|e| (*e).to_string()).collect(),
        }
    }

    pub fn out_mode(&self) -> OutMode<'_> {
        if self.in_place {
            OutMode::InPlace
        } else {
            OutMode::Dir(&self.out)
        }
    }

    pub fn descend(&self) -> Descend {
        if self.no_ignore {
            Descend::All
        } else {
            Descend::SkipVendor
        }
    }

    pub fn unmappable(&self) -> Unmappable {
        match self.on_unmappable {
            UnmappableArg::Escape => Unmappable::Escape,
            UnmappableArg::Fold => Unmappable::Fold,
            UnmappableArg::Replace => Unmappable::Replace,
            UnmappableArg::Fail => Unmappable::Fail,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        let mut v = vec!["to_pdf"];
        v.extend_from_slice(args);
        Cli::parse_from(v)
    }

    #[test]
    fn defaults_match_the_spec() {
        let c = parse(&["src"]);
        let g = c.geometry();
        assert_eq!(g.columns(), 102);
        assert_eq!(g.rows(), 76);
        assert_eq!(c.tab_width, 4);
        assert_eq!(c.max_file_size, 64);
        assert!(!c.line_numbers);
        assert!(!c.page_numbers);
        assert_eq!(c.unmappable(), Unmappable::Escape);
        assert_eq!(c.out, PathBuf::from("pdf-out"));
    }

    #[test]
    fn dense_shrinks_font_and_leading() {
        let g = parse(&["src", "--dense"]).geometry();
        assert_eq!(g.font_size, 7.0);
        assert_eq!(g.leading, 8.0);
        assert_eq!(g.columns(), 124);
    }

    #[test]
    fn an_explicit_font_size_beats_dense_and_derives_leading() {
        let g = parse(&["src", "--dense", "--font-size", "9"]).geometry();
        assert_eq!(g.font_size, 9.0);
        assert_eq!(g.leading, 10.62);
    }

    #[test]
    fn letter_page_size_changes_geometry() {
        let g = parse(&["src", "--page", "letter"]).geometry();
        assert_eq!(g.width, 612.0);
        assert_eq!(g.height, 792.0);
    }

    #[test]
    fn out_and_in_place_are_mutually_exclusive() {
        assert!(Cli::try_parse_from(["to_pdf", "src", "-o", "x", "--in-place"]).is_err());
    }

    #[test]
    fn in_place_selects_the_in_place_out_mode() {
        assert!(matches!(
            parse(&["src", "--in-place"]).out_mode(),
            OutMode::InPlace
        ));
        assert!(matches!(parse(&["src"]).out_mode(), OutMode::Dir(_)));
    }

    #[test]
    fn the_default_extension_set_is_used_when_ext_is_absent() {
        let e = parse(&["src"]).extensions();
        assert_eq!(e.len(), DEFAULT_EXTS.len());
        assert!(e.contains("rs") && e.contains("markdown"));
        assert!(!e.contains("epub"), "epub support was removed");
    }

    #[test]
    fn ext_overrides_the_default_set_and_is_normalised() {
        let e = parse(&["src", "--ext", ".RS,py, .Go"]).extensions();
        assert_eq!(
            e.iter().cloned().collect::<Vec<_>>(),
            vec!["go", "py", "rs"]
        );
    }

    #[test]
    fn on_unmappable_maps_to_the_encode_enum() {
        assert_eq!(
            parse(&["s", "--on-unmappable", "replace"]).unmappable(),
            Unmappable::Replace
        );
        assert_eq!(
            parse(&["s", "--on-unmappable", "fail"]).unmappable(),
            Unmappable::Fail
        );
    }

    #[test]
    fn on_unmappable_accepts_fold() {
        assert_eq!(
            parse(&["s", "--on-unmappable", "fold"]).unmappable(),
            Unmappable::Fold
        );
    }

    #[test]
    fn pdf_options_track_geometry_and_page_numbers() {
        let c = parse(&["src", "--page-numbers"]);
        let o = c.pdf_options();
        assert_eq!(o.width, 595.28);
        assert_eq!(o.font_size, c.geometry().font_size);
        assert!(o.page_numbers);
    }

    #[test]
    fn at_least_one_input_is_required() {
        assert!(Cli::try_parse_from(["to_pdf"]).is_err());
    }

    #[test]
    fn quiet_and_verbose_conflict() {
        assert!(Cli::try_parse_from(["to_pdf", "src", "-q", "-v"]).is_err());
    }
}
