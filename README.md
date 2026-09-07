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

**Build and data** — `Dockerfile` `Makefile` `.csv` `.in` `.mk` `.ipynb`

`.ipynb` is typeset as raw notebook JSON, not as rendered cells.

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