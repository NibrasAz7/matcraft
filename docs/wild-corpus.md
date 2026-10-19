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
| clean run | — | **20** |
| parse-ok (fdef) / parse-fail | **557 / 48** | — |
| parse-fail (script) | — | 35 |
| unknown builtin/function | 2 | 65 |
| runtime error | 9 | 16 |
| non-UTF-8 source | 29 | 7 |
| **total** | 645 | 143 |

- **Parser coverage: 557/616 decodable function files (90%)** parse cleanly
  (29 files fail earlier on non-UTF-8 encoding — see below). Against the
  previous sweep's 87%, the `@`-handle and N-D work moved most of the wall.
- The previous #1 blocker — `@` anonymous functions and handles — is
  **resolved**: `f = @(x) x.^2`, `@sin`, `feval`/`arrayfun`/`cellfun`/
  `func2str`/`str2func`/`functions` all land since f623a3e.
- N-D indexing (`x(i,j,k)`), the previous #3, is **resolved** since fbaa859:
  `zeros(2,3,4)`, merged-tail last subscripts, `ndims`/`cat(dim)`/`squeeze`.
- Several `unknown function` hits are repo-internal functions in sibling
  *subdirectories* (`simulationClass`, `Read_WAMIT`, `optset`, `inpaint_nans`,
  `iread`, `plotColors`, `setSeed`…) — these need `addpath(genpath(pwd))`
  even in MATLAB, so they are expected failures, not MatCraft gaps.

## What the corpus teaches now (ranked)

| Rank | Failure class | Files | Status |
| --- | --- | --- | --- |
| 1 | Missing builtins | 67 | Now the top class, but the tail is thin: only a handful are *real* builtins (`global`, `inline`, `peaks`, `fspecial`, `borders`, `ncreadatt`, `vline`, `wmean`); the rest are repo-internal helpers (expected fails). In flight this round: file I/O (`fopen`/`fread`/`fscanf`/`textscan`), regex (`regexp`/`regexprep`), stats (`prctile`/`hist`/`cov`), DSP/image (`fir1`/`peaks`/`im2double`/`imread`). |
| 2 | Parse fails (residual) | 83 total | Mostly `classdef` OOP, `try/catch` edge forms, and exotic command syntax — the long tail after `@` handles landed. |
| 3 | Non-UTF-8 sources | 36 | Legacy `.m` files are latin-1/cp1252, not UTF-8. Fix: decode latin-1 when UTF-8 fails (in flight — `read_m_source` already exported for it). |
| 4 | `classdef` OOP | subset of parse-fail | Documented big lift — the last major language feature gap. |
| 5 | `imread`-class image data | subset of builtins | PNG decode via the `png` dep (already used by matplot exports) — in flight with the builtins sweep. |

## Reproducing

```bash
python target/mcorpus/harvest.py    # re-download (needs a github.com credential)
python target/mcorpus/runcorpus.py  # re-run, writes results.json
```

The corpus is intentionally **not** committed (third-party licences); regenerate
it on demand.
