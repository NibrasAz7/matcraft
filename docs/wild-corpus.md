# Real-world `.m` corpus sweep — 2026-10 compatibility report

## Method

- **Corpus:** 788 `.m` files from 32 public GitHub repositories across 8 domains
  (control, finance, general/education, image, numerics, physics, signal,
  statistics), all in repos last pushed **before 2022** — so every file
  predates 2022. (Regrown from the lost Oct-2025 harvest; file count rose
  from 638 to 788.)
- **Harvest:** GitHub repository search (`language:MATLAB pushed:<2022-01-01`),
  per-repo git-tree enumeration, `.m` files < 12 KB, directory structure
  preserved so sibling function files resolve. Corpus lives in the gitignored
  `target/mcorpus/`; the downloader is `target/mcorpus/harvest.py`.
- **Runner:** `matcraft-cli run <file>` per file, cwd = the file's own
  directory, 15 s timeout, exit-code + first error captured
  (`target/mcorpus/runcorpus.py`, results in `target/mcorpus/results.json`).
- **Honesty notes:** `run` on a `function` file *defines* it without executing
  the body — function-file "pass" means **parse-ok**, not execution-ok. Files
  are third-party code used locally for testing only (never committed).

## Results

| Outcome | Function files | Script files |
| --- | --- | --- |
| clean run | — | **21** |
| parse-ok (fdef) / parse-fail | **582 / 52** | — |
| parse-fail (script) | — | 41 |
| unknown builtin/function | 2 | 63 |
| runtime error | 9 | 18 |
| non-UTF-8 source | 0 | 0 |
| **total** | 645 | 143 |

- **Parser coverage: 582/645 function files (90%)** parse cleanly — *all*
  files now decode since `read_m_source` falls back to latin-1 (the 36
  non-UTF-8 files are gone; the newly-decodable ones re-entered as
  parse-ok (+25) or honest parse-fails, which is why the raw parse-fail
  count rose). Against the original sweep's 87%, the `@`-handle, N-D and
  encoding work moved most of the wall.
- The previous #1 blocker — `@` anonymous functions and handles — is
  **resolved**: `f = @(x) x.^2`, `@sin`, `feval`/`arrayfun`/`cellfun`/
  `func2str`/`str2func`/`functions` all land since f623a3e.
- N-D indexing (`x(i,j,k)`), the previous #3, is **resolved** since fbaa859:
  `zeros(2,3,4)`, merged-tail last subscripts, `ndims`/`cat(dim)`/`squeeze`.
- The builtins sweep landed ~134 functions from this list (6007c27): file
  I/O (`fopen`/`fread`/`fscanf`/`textscan`/`dlmread`/`writematrix`), regex
  (`regexp`/`regexprep`), stats (`cov`/`prctile`/`quantile`/`hist`/nan-*),
  sets/bits/integer casts, degree trig, `fir1`/`peaks`/`fspecial`,
  `imread`/`imwrite`/`im2double` (pure-Rust PNG), system/env, date/time,
  `ind2sub`/`sub2ind`/`ndgrid`/`accumarray`/`bsxfun`, `inline`/`optimset`.
- Remaining `unknown function` hits are mostly repo-internal functions in
  sibling *subdirectories* (`simulationClass`, `Read_WAMIT`, `optset`,
  `inpaint_nans`, `iread`, `plotColors`, `setSeed`…) — these need
  `addpath(genpath(pwd))` even in MATLAB, so they are expected failures,
  not MatCraft gaps. Genuine remaining builtins are a thin tail
  (`borders`, `ncreadatt`, `vline`, `wmean`…).

## What the corpus teaches now (ranked)

| Rank | Failure class | Files | Status |
| --- | --- | --- | --- |
| 1 | Parse fails (residual) | 93 total | Now the top class — mostly `classdef` OOP plus `try/catch` edge forms and exotic command syntax; includes the newly-decoded latin-1 files. |
| 2 | Missing builtins / repo-local helpers | 65 | Thin tail after the 6007c27 sweep; mostly expected repo-internal helpers, a handful of real builtins remain. |
| 3 | `classdef` OOP | subset of parse-fail | Documented big lift — the last major language feature gap. |
| 4 | `global`/`persistent` semantics | unknown count | Keyword-level state semantics not yet implemented. |
| ~~5~~ | ~~Non-UTF-8 sources~~ | ~~36~~ | **Resolved** — `read_m_source` decodes latin-1 when UTF-8 fails; wired into CLI, engine `script.run`, `run`, function loading and the Studio editor. |

## Reproducing

```bash
python target/mcorpus/harvest.py    # re-download (needs a github.com credential)
python target/mcorpus/runcorpus.py  # re-run, writes results.json
```

The corpus is intentionally **not** committed (third-party licences); regenerate
it on demand.
