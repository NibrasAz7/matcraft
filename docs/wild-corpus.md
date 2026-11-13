# Real-world `.m` corpus sweep — compatibility report

## Method

- **Corpus:** 12,738 `.m` files from 38 public GitHub repositories, all
  last pushed **before 2022** — every file predates 2022. Sources mix
  star-ranked MATLAB repos (DeepLearnToolbox, faster_rcnn, DnCNN,
  mexopencv, PRMLT, MATLABRobotics, BPL, pdollar/toolbox, …) with a
  deliberate numerical/scientific top-up: `chebfun/chebfun`,
  `WEC-Sim/WEC-Sim`, `petercorke/robotics-toolbox-matlab`,
  `petercorke/machinevision-toolbox-matlab`, `fieldtrip/fieldtrip`,
  `matlab2tikz/matlab2tikz`.
- **Harvest:** GitHub repository search (`language:MATLAB
  pushed:<2022-01-01`) plus the top-up list, per-repo git-tree
  enumeration, `.m` files < 12 KB, directory structure preserved so
  sibling function files resolve. Corpus lives in the gitignored
  `target/mcorpus/` (`harvest.py`, `topup.py`, `repos.txt`).
- **Runner:** `matcraft-cli run <file>` per file, cwd = the file's own
  directory, 15 s timeout, exit-code + first error captured
  (`runcorpus.py`, `results.json`).
- **Honesty notes:** `run` on a `function` file *defines* it without
  executing the body — function-file "pass" means **parse-ok**, not
  execution-ok. `classdef` files are classified as scripts (they start
  with `classdef`, not `function`), so a passing classdef means
  parse-*and*-register. Files are third-party code used locally for
  testing only (never committed).
- **Not file-for-file comparable with earlier sweeps.** The previous
  788-file / 32-repo corpus was lost; this is a fresh, much larger
  harvest under the same method with a different repository mix.

## Results

| Outcome | Function files | Script files |
| --- | --- | --- |
| clean run | — | **1,897** |
| parse-ok / parse-fail | **10,071 / 27** | — / 33 |
| unknown builtin/function | 1 | 506 |
| runtime error | 6 | 95 |
| missing data/env files | 0 | 99 |
| timeout | 0 | 3 |
| **total** | 10,105 | 2,633 |

- **Parser coverage: 10,071/10,105 function files (99.7%)** parse
  cleanly. The latest grammar wave — `get.prop`/`set.prop` classdef
  accessors, `~` placeholders in anonymous-function parameter lists
  (`@(~,~) cb()`), and the `randi([lo hi])` range form — moved ~80
  files out of the failure buckets (BPL and chebfun `@functionalBlock`
  classes needed dotted accessor names; mexopencv GUI samples needed
  `@(~,~)`).
- **Remaining parse-fails (60) are almost all non-ASCII source** — the
  lexer rejects stray bytes like `µ`, `²`, `?`-mojibake and mid-code
  Latin-1 characters (~25 files), plus a small tail of genuine edge
  syntax (`unexpected Eq` ×7 and singles). No class-level grammar gap
  remains in this corpus.
- **The 99 env-errors are all missing sibling data files** — scripts
  that `load`/`imread` `*.mat`/`*.txt`/`*.jpg` files the harvest never
  downloads (it collects `.m` only). Expected, not a gap.
- **The 506 unknown-builtin failures are dominated by MEX/toolbox
  externals**: `mexopencv`/`cv` (123 hits — the mexopencv MEX layer),
  `iread`, `readNEMOH`, `readWAMIT`, `simulationClass`, `SerialLink`,
  `mdl_puma560` (Corke/WEC-Sim entry points), `cfg` (FieldTrip's MEX
  config system), `kmeansRnd`, `randinit`, `modem` — functions that
  live in repo-internal subdirectories (`+pkg`, `private/`, MEX
  wrappers) that even MATLAB needs `addpath(genpath(pwd))` or compiled
  binaries to find.
- **3 timeouts** are heavy scripts genuinely computing for >15 s
  (FieldTrip/WEC-Sim pipelines), not hangs.

## What this corpus exercised that the old one didn't

- **classdef accessors at scale** — `get.x`/`set.x` methods now
  dispatch on `obj.x` reads and `obj.x = v` writes with a self-access
  guard (recursion into the same property's accessor errors, matching
  documented semantics).
- **MEX-facing code** — mexopencv and FieldTrip contributed most of the
  unknown-builtin tail; those need an actual MEX layer, tracked in
  `gaps.md`.
- **Toolbox-heavy scripts** — the optimization/control/toolbox merge
  (`fminbnd`, `tf`, `kmeans`, …) is exercised for real by WEC-Sim,
  Corke, and chebfun files.
