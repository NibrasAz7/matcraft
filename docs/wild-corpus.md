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
| clean run | — | **22** |
| parse-ok (fdef) / parse-fail | **632 / 1** | — |
| parse-fail (script) | — | 18 |
| unknown builtin/function | 2 | 75 |
| runtime error | 10 | 28 |
| non-UTF-8 source | 0 | 0 |
| **total** | 645 | 143 |

- **Parser coverage: 632/645 function files (98%)** parse cleanly; the one
  remaining function fail is a bare non-code line in a doc-style file.
  All 18 script parse-fails are `classdef` OOP (`<` superclass,
  `properties (Access = …)` blocks) except two genuinely-invalid files
  (bare URL / bare changelog text). This wave's grammar work —
  double-quoted strings, `#!` shebangs, Octave `end`-variants
  (`endfunction`/`endif`/…), `import pkg.*`, command args with quoted or
  punctuated words (`cpu etoc 'x'`, `FieldTrip:fileio:id`,
  `im subplot [1 2 3]`, `cd ../..`), `[s.f, v(i)] = f()` multi-assign,
  `parfor`, `!cmd` shell escape, and `x.^s.d`/`a^b^c`/`(-1).^(0:5)'`
  exponent parsing — is what drained the old 93-file parse-fail class.
  Unknown-builtin and runtime-error counts *rose* as files moved past
  the parser into execution.
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
  `wecSim`, `inpaint_nans`, `iread`, `plotColors`, `setSeed`…) — these
  need `addpath(genpath(pwd))` even in MATLAB, so they are expected
  failures, not MatCraft gaps. The thin builtin tail was landed this
  wave: `vline` (vertical reference lines), `wmean` (weighted mean),
  `filesep`, `get` (root `ScreenSize` + basic figure properties) and
  `set` (root `Default*` props + the same figure props), Octave's
  `argv`, plus the `global`/`persistent`/`import` declarations.
- **Rejected as infeasible/out-of-scope** (documented, not stubbed):
  `ncreadatt`/`h5struct`/`h5*` (netCDF/HDF5 file formats — no
  readers in-tree), `SX`/`casadi` (the CasADi symbolic toolbox the
  `import casadi.*` scripts need), `text` (axes text annotation —
  `matplot` has no label series kind yet), `borders`/`cmocean`/`cdtgrid`/
  `sca`/`gjet`/`ntitle`/`polyplot` (Mapping-Toolbox/classroom helpers —
  repo- or toolbox-local), `ssh2_simple_command` (SSH), `mdl_puma560`/
  `SerialLink`/`Revolute`/`puma560`/`read_curv`/`vl_root`/
  `eeglab_options`/`xb_generate_model`/`wecSim`/`wecSimInputFile`/
  `simulationClass`/`Read_WAMIT`/`optset`/`cfg`/`SS`/`randinit`/`readcfl`
  (toolbox/repo-local entry points), `logsumexp`/`isvar`/`t`/`start`/
  `nodeunif`/`euler2DCM`/`crazyflie`/`trajectory_generator`/
  `daily_insolation`/`air_density`/`filt2`/`Gblur2`/`ellipse_im`/
  `path_find_dir`/`clock_out`/`waves`/`iroi` (repo-local helpers).

## What the corpus teaches now (ranked)

| Rank | Failure class | Files | Status |
| --- | --- | --- | --- |
| 1 | Parse fails (residual) | 19 total | **Drained 93 → 19** — all that remains is `classdef` OOP (16 script files) plus two bare non-code lines and one `properties (Access = …)` block. |
| 2 | Missing builtins / repo-local helpers | 77 | Real builtins landed (`vline` `wmean` `filesep` `get` `set` `argv`); remainder is repo-local helpers, external toolboxes (CasADi `SX` ×12), and documented rejects. |
| 3 | `classdef` OOP | 16 of parse-fail | Documented big lift — the last major language feature gap, now nearly the entire residual parse-fail class. |
| 4 | `global`/`persistent` semantics | unknown count | **Resolved** — `global x` links a frame to the shared store, `persistent x` keeps values across calls of the same function. |
| ~~5~~ | ~~Non-UTF-8 sources~~ | ~~36~~ | **Resolved** — `read_m_source` decodes latin-1 when UTF-8 fails; wired into CLI, engine `script.run`, `run`, function loading and the Studio editor. |

## Reproducing

```bash
python target/mcorpus/harvest.py    # re-download (needs a github.com credential)
python target/mcorpus/runcorpus.py  # re-run, writes results.json
```

The corpus is intentionally **not** committed (third-party licences); regenerate
it on demand.
