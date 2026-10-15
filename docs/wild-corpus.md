# Real-world `.m` corpus sweep — 2025-10 compatibility report

## Method

- **Corpus:** 638 `.m` files from 32 public GitHub repositories across 8 domains
  (control, finance, general/education, image, numerics, physics, signal,
  statistics), all in repos last pushed **before 2022** (2016–2021) — so every
  file predates 2022.
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
| clean run | — | **7** |
| parse-ok (fdef) / parse-fail | **401 / 60** | — |
| parse-fail (script) | — | 68 |
| unknown builtin/function | 0 | 48 |
| runtime error | 10 | 40 |
| timeout (>15 s) | 0 | 4 |
| **total** | 471 | 167 |

- **Parser coverage: 401/461 function files (87%)** parse cleanly.
- Scripts die overwhelmingly on missing builtins first (48) or a later parse
  wall (68) — mostly the same `@` wall the function files hit.
- Several `unknown function` hits are repo-internal functions in sibling
  *subdirectories* (`design_array_1d`, `ml_plot_data`, `strain_to_*`…) — these
  need `addpath(genpath(pwd))` even in MATLAB, so they are expected failures,
  not MatCraft gaps. `genpath` + multi-path `addpath` now work.

## What the corpus taught us (ranked)

| Rank | Failure class | Files | Status |
| --- | --- | --- | --- |
| 1 | **`@` anonymous functions / function handles** | ~91 parse-fails | **the #1 blocker** — `f = @(x) x.^2`, `@sin` handles, `cellfun`/`fminsearch` args. See G31. |
| 2 | Missing builtins | 48 scripts | Fixed this round: `deg2rad` `rad2deg` `sinc` `randperm` `normrnd` `assert` `rng` `fullfile` `genpath` `csvread` `mkdir` `eps` `Inf(n,m)` `NaN(n)` `axis` `box` `db2pow` `pow2db` `uint8` `rectpuls` `tripuls`. Remaining: `imread` (needs N-D image data), `fir1`, `chi2inv`/`gamrnd`, `fspecial`/`im2double`, `syms` (symbolic — out of scope), `urlwrite` (network). |
| 3 | N-D indexing (`x(i,j,k)`) | 3+ | `expected 1 or 2 indices, got 3` — needs N-D arrays (G22 tail). |
| 4 | `classdef` OOP | 2+ | `classdef Foo < handle` — documented big lift. |
| 5 | Misc syntax | ~20 | `expected a scalar` (14 — mostly downstream of unimplemented calls), string-in-numeric contexts, command-form edge cases. |

**Fixed during the sweep** (syntax): `for(i=1:n)` parenthesised loop headers;
`cd ..`/`cd ../x` bare-`..` command arguments.

## Reproducing

```bash
python target/mcorpus/harvest.py    # re-download (needs a github.com credential)
python target/mcorpus/runcorpus.py  # re-run, writes results.json
```

The corpus is intentionally **not** committed (third-party licences); regenerate
it on demand.
