# to_pdf

Convert a source file — or a directory tree of thousands of files — into PDFs
that are as small as a correct PDF can be, without altering the source text.

```
inspect_ai (3,702 files)   43 MB  →  19 MB   (44% of source, 1.4s)
3,000-file synthetic tree  34 MB  →  4.7 MB  (14% of source, 0.33s)
this repo's src/           82 KB  →   45 KB  (55% of source)
```

Two rules drive every design decision:

1. **Output must be small.** No embedded fonts, ever.
2. **Code must survive unchanged.** Indentation is load-bearing in Python; a
   PDF that is small but misrepresents the code is a failure.

## Install

Needs [Rust](https://rustup.rs) 1.85 or newer (2024 edition).

```bash
git clone <this repo> && cd to_pdf
cargo build --release
# binary at ./target/release/to_pdf
```

To run it from anywhere:

```bash
cargo install --path .
```

## Use

```bash
to_pdf main.py                        # → ./pdf-out/main.py.pdf
to_pdf ~/projects/api -o ~/pdfs       # whole tree, structure mirrored
to_pdf src --in-place                 # foo.rs.pdf written beside foo.rs
to_pdf src -o out --dense             # ~25% fewer pages
to_pdf src -o out --line-numbers
to_pdf ~/code -o ~/pdfs --dry-run -v  # see what would happen, write nothing
```

Files are named `foo.rs.pdf`, keeping the source extension so `foo.rs` and
`foo.go` in one directory cannot collide on `foo.pdf`.

Passing several roots keeps them apart — each gets its own subdirectory, so
`api/src/main.rs` and `web/src/main.rs` never overwrite each other:

```bash
to_pdf api web -o out
#   out/api/src/main.rs.pdf
#   out/web/src/main.rs.pdf
```

### Formats

**Code** — `.c` `.cc` `.cjs` `.cpp` `.cs` `.cts` `.go` `.h` `.hpp` `.java`
`.js` `.jsx` `.kt` `.lua` `.mjs` `.mts` `.php` `.py` `.rb` `.rs` `.sh` `.sql`
`.swift` `.ts` `.tsx`

**Markup and prose** — `.css` `.epub` `.htm` `.html` `.markdown` `.md` `.qmd`
`.rst` `.scss` `.txt` `.xml`

**Configuration** — `.cfg` `.ini` `.json` `.toml` `.yaml` `.yml`

**Build and data** — `Dockerfile` `Makefile` `.csv` `.in`

Entries match a file's extension *or* its whole filename, case-insensitively,
so `dockerfile` covers both a bare `Dockerfile` and `debug.Dockerfile`. A
suffixed `Dockerfile.prod` is not matched — its extension is `prod`.

Binary formats (`.png`, `.svg`, `.pdf`, `.zip`) and bulk data (`.jsonl`, lock
files) are deliberately excluded — they either cannot be typeset as text or
produce enormous PDFs nobody reads. `-v` lists everything skipped and why.

`--ext` **replaces** the default set rather than adding to it, so list
everything you want:

```bash
to_pdf ~/code --ext rs,toml,yaml,csv -o out
```

### Ignored directories

By default the walk does not descend into hidden directories (any name starting
with `.`, which covers `.git`, `.venv`, `.idea`, `.tox`, `.next`) or into
`node_modules`, `__pycache__`, `target`, `dist`, `build`, `vendor`, `venv`,
`coverage`, `site-packages`, and `bower_components`.

Without this, `to_pdf ~/myproject` on a JavaScript project would try to convert
every vendored file in `node_modules`. Pass `--no-ignore` to walk everything.

Naming an ignored directory explicitly still works — the rule applies only to
directories found during the walk, so `to_pdf .git` does what you asked.

- **Code and text** are reproduced verbatim — see *Fidelity* below.
- **Markdown is treated as source, not rendered.** `# Title` stays `# Title`.
  Rendering it would mean changing it, which rule 2 forbids.
- **HTML** is reduced to text: scripts, styles, and images dropped; entities
  decoded; `<pre>` whitespace preserved.
- **EPUB** chapters are extracted in spine (reading) order. Images and cover
  art are dropped — they would dominate the file size.

### Options

| Flag | Default | |
|---|---|---|
| `-o, --out <DIR>` | `pdf-out` | Output root. Mutually exclusive with `--in-place`. |
| `--in-place` | | Write beside the source instead. |
| `-j, --jobs <N>` | all cores | Worker threads. |
| `--dense` | | 7pt/8pt instead of 8.5pt/10pt: 124×96 chars per page. |
| `--font-size <PT>` | `8.5` | Overrides `--dense`; leading derives as `size × 1.18`. |
| `--page <a4\|letter>` | `a4` | |
| `--tab-width <N>` | `4` | Tabs advance to the next tabstop, not a flat N spaces. |
| `--line-numbers` | off | |
| `--page-numbers` | off | |
| `--on-unmappable <M>` | `escape` | `escape` \| `replace` \| `fail` — see below. |
| `--ext <LIST>` | see above | Comma-separated; replaces the default set. |
| `--no-ignore` | | Descend into hidden and vendor directories too. |
| `--max-file-size <MB>` | `64` | Larger inputs are skipped, not attempted. |
| `--dry-run` | | |
| `--fail-fast` | | Stop at the first failure instead of collecting them. |
| `-q` / `-v` | | Quiet / list skipped files. |

A failing file never aborts the run. Failures are collected, printed at the
end, and the exit code is 1 if any file failed.

## What to expect from the size

Files under roughly 500 bytes get **bigger**. A PDF cannot be smaller than the
~600 bytes of structure it requires — catalog, page tree, font dictionary,
cross-reference stream. Everything above about 1 KB shrinks, and the effect
compounds across a tree.

The savings come from four decisions:

- **Standard-14 Courier with `/WinAnsiEncoding`.** Zero bytes of embedded font.
  This alone is the difference between an 8 KB file and a 300 KB one.
- **`/MediaBox` and `/Resources` inherited from the page tree**, so each page
  dictionary is about 40 bytes.
- **All non-stream objects packed into one `/ObjStm`**, Flate-compressed.
- **A `/XRef` stream instead of a classic table**, which costs exactly 20
  uncompressed bytes per object.

Text costs one `'` operator per line. No `/Info`, no XMP, no `/ID` — which also
makes output **byte-deterministic**: the same input always produces the same
bytes.

## Fidelity

For code and text, the only transformations applied are:

1. `\r\n` and lone `\r` → `\n`
2. Tabs expanded to the next tabstop
3. Characters WinAnsi cannot represent → a visible `\u{...}` escape

Never: trimming leading or trailing whitespace, collapsing blank lines,
normalising quotes, re-indenting, or inserting any header or banner into the
text body.

**Unmappable characters.** Courier's WinAnsi encoding covers ASCII and Latin-1
(including curly quotes, em dashes, ellipses, and accented letters). Anything
outside it — CJK, emoji, box-drawing — is rendered as a visible, reversible
escape rather than silently dropped:

```python
s = "😀"    →    s = "\u{1F600}"
```

Use `--on-unmappable replace` for a lossy `?` instead, or `fail` to reject such
files outright. The run summary reports how many files contained escapes.

**Long lines** wrap rather than clip, and wrapping never shifts the source's
columns. A two-column gutter is carved out of the page width for the whole
file, blank on normal lines and `»` on continuations, so source column 0 always
lands at the same place. An added indent would be indistinguishable from real
Python indentation.

## Portability

The source is portable; the compiled binary is not.

Every dependency is pure Rust — there is no C toolchain, no `zlib`, no system
library. The code contains no `unsafe` and no platform-specific branches. It is
verified to compile clean for macOS (Apple Silicon), `x86_64-unknown-linux-gnu`,
and `x86_64-pc-windows-msvc`.

To use it on another machine, either install Rust there and
`cargo build --release`, or cross-compile:

```bash
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu
```

Cross-compiling to Linux or Windows from macOS needs a linker for that target
(`cargo-zigbuild` or `cross` are the easy routes); building natively on the
target machine needs nothing but Rust.

## Development

```bash
cargo test                                # 136 tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Four integration guards in `tests/integration.rs` enforce the two goals and
fail the build if either regresses:

1. **Size budget** — a real source file must produce a PDF smaller than itself.
2. **Determinism** — converting twice yields identical bytes.
3. **Structure** — every object the cross-reference stream names resolves.
4. **Fidelity** — a Python fixture is decoded back out of the finished PDF and
   compared line by line, leading and trailing whitespace included.

Layout:

```
src/
  pdf/{mod,object,xref,encode}.rs   hand-rolled PDF 1.5 writer
  extract/{mod,plain,html,epub}.rs  per-format text extraction
  layout.rs                         wrapping, gutter, pagination
  walk.rs                           discovery and output path mapping
  cli.rs  pipeline.rs  main.rs
docs/superpowers/specs/             design rationale
docs/superpowers/plans/
```

The three inner stages share no types: `extract` turns bytes into lines,
`layout` turns lines into pages, `pdf` turns pages into bytes. None of them
knows about the others.
