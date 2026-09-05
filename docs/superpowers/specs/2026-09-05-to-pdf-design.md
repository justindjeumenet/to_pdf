# to_pdf — Design

Date: 2026-09-05
Status: Approved for planning

## 1. Purpose

A Rust CLI that converts a source file, or a directory tree of thousands of
files, into PDFs that are as small as a correct PDF can be.

Two goals, in priority order:

1. **Minimal output size.** The dominant lever is avoiding embedded fonts;
   the rest is PDF structure discipline.
2. **Exact source fidelity.** Code must survive the round trip unchanged.
   A PDF that is small but misrepresents the code is a failure.

Supported inputs: `.js`, `.ts`, `.go`, `.rs`, `.py`, `.txt`, `.html`,
`.md`, `.markdown`, `.epub`.

## 2. Non-goals

- Syntax highlighting. Colour costs bytes and buys nothing here.
- Images. EPUB and HTML images are dropped; they would dominate size.
- Embedded fonts, in any mode. See §5 for how non-Latin1 text is handled.
- Round-tripping PDF back to source.
- Any reflow, reformat, or pretty-printing of source code.

## 3. Architecture

Three-phase pipeline. The three inner stages share no types and no
knowledge of each other.

    walk    -> Vec<Job>{src, dst}           single-threaded, cheap
              | rayon par_iter
    extract -> Document{title, Vec<String>} per-format; knows no PDF
    layout  -> Vec<Page>{Vec<String>}       wrapping/pagination; knows no PDF
    pdf     -> Vec<u8>                      knows no text semantics
              | one fs::write

Module layout:

    src/
      main.rs         wire-up, exit code
      cli.rs          argument definitions and parsing
      walk.rs         recursive discovery, extension filter, src->dst mapping
      pipeline.rs     rayon fan-out, error collection, run summary
      extract/
        mod.rs        Document type, dispatch on extension
        plain.rs      code and plain text
        html.rs       HTML -> text
        epub.rs       EPUB -> text (delegates chapters to html.rs)
      layout.rs       wrapping, pagination, gutter
      pdf/
        mod.rs        document builder and writer
        object.rs     object ids, dict and stream serialization
        xref.rs       object streams and cross-reference stream
        encode.rs     WinAnsi mapping, string escaping

Each stage is independently unit-testable: `extract` takes bytes and
returns lines, `layout` takes lines and returns pages, `pdf` takes pages
and returns bytes.

## 4. Size engineering

These are the specific decisions that produce the size target. They are
requirements, not suggestions.

### 4.1 Fonts

Use the PDF standard-14 font `Courier` with `/WinAnsiEncoding`:

    <</Type/Font/Subtype/Type1/BaseFont/Courier/Encoding/WinAnsiEncoding>>

No font program is embedded. No `/FontDescriptor` is required for the
standard 14. This is roughly 70 bytes, once per document, and is the
single largest contributor to the size target — an embedded subsetted
font would add 40-300 KB to every file.

Courier is fixed-pitch at 600/1000 em, so advance width is
`font_size * 0.6` and no metrics table is needed.

### 4.2 Document structure

- `/MediaBox` and `/Resources` (holding `/Font << /F0 ... >>`) are set on
  the `/Pages` node and inherited. Every page dict is therefore
  `<</Type/Page/Parent 2 0 R/Contents N 0 R>>`, about 40 bytes.
- All non-stream objects — catalog, pages node, font, and every page dict
  — are packed into a single `/Type/ObjStm`, Flate-compressed. 200 page
  dicts is roughly 8 KB raw and about 600 bytes compressed.
- Cross-references use a `/Type/XRef` stream with `/W [1 4 2]`,
  Flate-compressed. A classic xref table costs exactly 20 bytes per
  object, uncompressed; for 1000 objects that is 20 KB versus roughly
  800 bytes.
- No `/Info` dictionary, no XMP metadata, no `/ID`. Besides saving bytes
  this makes output byte-deterministic: identical input yields identical
  output.
- Header is `%PDF-1.5` plus the conventional 4-byte high-bit comment line.

### 4.3 Content streams

One Flate-compressed (level 9) content stream per page. Per-page preamble
is issued once:

    BT
    /F0 8.5 Tf
    10 TL
    36 <y0> Td

where `first_baseline = page_height - margin - font_size` and
`y0 = first_baseline + leading`.

Every text line is then a single operator:

    (escaped text)'

The `'` operator means "advance to the next line, then show the string",
so it replaces `T*` followed by `Tj`. `Tf`, `TL`, and `Td` are never
re-issued inside a page. The `Td` y-coordinate is set one leading above
the first baseline so that the first `'` lands correctly.

Line numbers and the wrap marker are prefixed into the same string, so
they cost zero additional operators.

### 4.4 Target

A 500-line Rust source file, roughly 15 KB, should produce a PDF of
about 6-9 KB — smaller than the input. This is enforced by test, not
by assertion in prose. See §8.

## 5. Source fidelity

For code and plain text the **only** permitted transformations are:

1. `\r\n` and lone `\r` are normalized to `\n`.
2. Tabs expand to the **next tabstop**, i.e. advance to the next multiple
   of `--tab-width` (default 4). They do not expand to a flat N spaces;
   flat expansion breaks alignment.
3. Characters with no WinAnsi representation are escaped (see §5.1).

Explicitly forbidden: trimming leading or trailing whitespace, collapsing
blank lines, normalizing quote characters, re-indenting, reordering, or
inserting any header, banner, or footer into the text body. Indentation
is load-bearing in Python and must be reproduced exactly.

Curly quotes, em and en dashes, ellipsis, non-breaking space, and the
Latin-1 accented range all exist in WinAnsi and map directly with no
escaping.

### 5.1 Unmappable characters

Characters outside WinAnsi — CJK, emoji, box-drawing, Cyrillic — are
rendered as a visible, reversible escape rather than silently replaced:

    a String with 😀  ->  a String with \u{1F600}

Controlled by `--on-unmappable`:

- `escape` (default) — as above. Lossless and unambiguous.
- `replace` — substitute `?`. Smaller, lossy; opt-in only.
- `fail` — treat the file as an error and report it.

The run summary reports how many files contained escaped characters.

### 5.2 Long lines

Lines wider than the usable text width are wrapped, never clipped.
Wrapping must not shift the source's own columns, because an added
indent is indistinguishable from real Python indentation.

A 2-column left gutter is therefore reserved for the whole file whenever
any line in that file wraps, or whenever `--line-numbers` is on. The
gutter is carved out of the total column count, not added to it: at the
default 102 columns, an active gutter leaves 100 columns of source text.
Files that need no gutter keep all 102. Normal lines get two spaces in the gutter;
continuation lines get `» ` (WinAnsi 0xBB). Source column 0 is always at
the same page x-coordinate on every line, wrapped or not.

If `--line-numbers` is on, the number field occupies the gutter and
continuation lines show a blank number with the `»` marker.

## 6. Extraction

### 6.1 Code and plain text

Extensions `.js .ts .go .rs .py .txt .md .markdown`. Read bytes, decode
UTF-8 lossily, apply the §5 rules, split on `\n`. Markdown is treated as
plain text: it is source, and rendering it would violate §5.

### 6.2 HTML

A hand-rolled streaming tag stripper, no dependency:

- `<script>`, `<style>`, and `<head>` contents are dropped.
- Block-level tags produce a line break; `<br>` produces a line break.
- `<li>` is prefixed with `- `.
- `<h1>`-`<h6>` produce a blank line before and after.
- `<pre>` content preserves internal whitespace.
- Named and numeric character entities are decoded, then passed through
  the §5.1 unmappable rules.
- Images, including alt text, are dropped.

This is the component with the most exposure to malformed real-world
input, so it carries the heaviest test coverage.

### 6.3 EPUB

Uses the `zip` crate with `default-features = false, features = ["deflate"]`.
Hand-rolling ZIP parsing is not worth the bug surface.

1. Read `META-INF/container.xml`, extract the OPF path.
2. Parse the OPF `<manifest>` and `<spine>` to get the XHTML documents
   in reading order.
3. Feed each spine document through the HTML extractor.
4. Join with a chapter heading derived from the manifest title or
   filename, separated by a blank line.

Images and cover art are dropped.

## 7. CLI, concurrency, and errors

### 7.1 Interface

    to_pdf <INPUT>... [OPTIONS]

      -o, --out <DIR>           output root (default ./pdf-out)
          --in-place            write foo.rs.pdf beside foo.rs
      -j, --jobs <N>            worker threads (default: available parallelism)
          --font-size <PT>      default 8.5
          --dense               7pt font, 8pt leading
          --page <a4|letter>    default a4
          --tab-width <N>       default 4
          --line-numbers        off by default
          --page-numbers        off by default
          --on-unmappable <M>   escape|replace|fail (default escape)
          --ext <LIST>          override the extension set
          --max-file-size <MB>  default 64
          --dry-run
          --fail-fast
      -q, --quiet / -v, --verbose

Output naming: `foo.rs` becomes `foo.rs.pdf`. The source extension is
retained so that `foo.rs` and `foo.go` in one directory do not collide on
`foo.pdf`.

`--out` and `--in-place` are mutually exclusive; supplying both is a
usage error. Under `--out`, each input root is mapped to a subdirectory
named for that root's final path component, so two roots that both
contain `src/main.rs` cannot collide:

    to_pdf api/ web/ -o out/
      api/src/main.rs -> out/api/src/main.rs.pdf
      web/src/main.rs -> out/web/src/main.rs.pdf

A bare file input maps to `<out>/<filename>.pdf` with no subdirectory.
Intermediate output directories are created as needed. An existing output
file is overwritten.

Page defaults: A4 (595.28 x 841.89 pt), 36 pt margins, Courier 8.5 pt with
10 pt leading, giving 102 columns by 76 lines. `--dense` gives 124 by 96
and cuts page count by roughly 25%, which cuts per-page overhead with it.
`--dense` is shorthand for a font size and leading pair; an explicit
`--font-size` given alongside it wins, and leading is then derived as
`font_size * 1.18` rounded to two decimals.

No page header or footer is emitted by default. `--page-numbers` adds a
centred footer number in its own `BT`/`ET` block after the body block, so
it never perturbs the body's text-positioning state.

### 7.2 Concurrency

Phase 1 walks the tree single-threaded and builds `Vec<Job>`. Symlinks
are not followed, which removes cycle risk without needing an inode set.

Phase 2 is `jobs.par_iter().map(convert)` under rayon. Jobs are fully
independent; the only shared state is an atomic counter and a mutex-guarded
error vector. Peak memory is bounded by one file per worker thread, plus
the `--max-file-size` guard which skips oversized inputs rather than
attempting them.

Each job builds the PDF into a `Vec<u8>` and issues exactly one
`fs::write`.

### 7.3 Errors

A per-file failure never aborts the run. Failures are collected as
`(path, error)` and printed as a summary at the end. Exit code is 0 if
every file succeeded, 1 if any failed. `--fail-fast` opts into aborting
on the first error. Files with unsupported extensions are skipped
silently unless `--verbose`.

## 8. Testing

Test-driven: tests are written before the implementation of each module.

Unit tests:

- `pdf/object`: dictionary and stream serialization, literal-string
  escaping of `(`, `)`, and `\`.
- `pdf/encode`: Unicode to WinAnsi mapping across the 0x80-0x9F special
  range, the Latin-1 range, and the unmappable path for all three
  `--on-unmappable` modes.
- `pdf/xref`: object-stream packing, xref stream byte layout for `/W [1 4 2]`,
  Flate round-trip.
- `layout`: wrap at exactly the column limit and at limit+1, gutter
  activation, tab expansion to the next tabstop including mid-line tabs,
  empty file, file with no trailing newline, a single 100 KB line,
  a file that is only blank lines.
- `extract/plain`: leading and trailing whitespace preserved byte-for-byte;
  a Python fixture with mixed indentation round-trips unchanged.
- `extract/html`: entity decoding, nested and unclosed tags, `script` and
  `style` stripping, `pre` whitespace.
- `extract/epub`: an EPUB fixture assembled as a zip inside the test.
- `walk`: extension filtering, src-to-dst path mapping, `--in-place`.

Integration guards:

1. **Size budget** — for a code fixture, `pdf.len() < source.len()`.
   A regression here fails the build. This is the test that enforces
   the primary goal.
2. **Determinism** — converting the same input twice yields identical
   bytes.
3. **Structural validity** — the output parses as well-formed PDF 1.5
   with a resolvable xref stream reaching every object.
4. **Fidelity** — for a Python fixture, every source line appears in the
   page text with identical leading whitespace.

## 9. Dependencies

    flate2   Flate compression of content, object, and xref streams
    rayon    parallel per-file conversion
    zip      EPUB container reading (deflate only)
    clap     argument parsing

Four. HTML extraction, PDF writing, and the directory walk are all
hand-rolled, both for control over output size and to keep the
dependency surface small.
