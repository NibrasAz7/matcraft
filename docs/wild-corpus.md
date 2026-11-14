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
| clean run | — | **1,907** |
| parse-ok / parse-fail | **10,082 / 23** | — / 15 |
| unknown builtin/function | 1 | 516 |
| runtime error | 6 | 71 |
| missing data/env files | 0 | 115 |
| timeout | 0 | 2 |
| **total** | 10,105 | 2,633 |

- **Parser coverage: 10,082/10,105 function files (99.8%)** parse
  cleanly. Two grammar waves landed against this corpus: (1)
  `get.prop`/`set.prop` classdef accessors, `~` in anonymous params
  (`@(~,~) cb()`), `randi([lo hi])`; (2) `?ClassName` metaclass refs
  (evaluating to `meta.class` objects), `arguments … end` validation
  blocks (tolerated, not enforced), `#` Octave line comments,
  command-form `-flag` args (`mex -outdir bin`) and `~` paths
  (`cd ~/x`), `case'2d'` keyword-quote strings, Fortran `d`/`D`
  exponents (`0.5d-6`), and `zeros(m,n,'uint8')`/`'like',x` type-spec
  tails (accepted; all values are f64 — a documented deviation).
- **Remaining parse-fails (38) are not grammar gaps** — macOS `._*`
  AppleDouble junk (9), mojibake'd non-UTF8 source bytes (8), and
  prose/comment lines that lost their `%` (`-> …`, bare paths,
  statements with no separator) which MATLAB also rejects.
- **The 115 env-errors are missing sibling files** — scripts that
  `load`/`imread`/`fscanf` `*.mat`/`*.txt`/`*.jpg`/`*.fig` the harvest
  never downloads, plus `cd` into lab-machine absolute paths.
- **The 516 unknown-builtin failures are dominated by MEX/toolbox
  externals**: `mexopencv`/`cv` (123 hits — the mexopencv MEX layer),
  `iread`, `readNEMOH`, `readWAMIT`, `simulationClass`, `SerialLink`,
  `mdl_puma560` (Corke/WEC-Sim entry points), `cfg` (FieldTrip's MEX
  config system), `kmeansRnd`, `randinit`, `modem`, `TestSuite`
  (matlab.unittest) — functions that live in repo-internal
  subdirectories (`+pkg`, `private/`, MEX wrappers) that even MATLAB
  needs `addpath(genpath(pwd))` or compiled binaries to find.
- **The 77 runtime errors** are a long tail: `A^B` matrix-exponent
  calls that MATLAB rejects identically (8), deeper per-file semantic
  gaps (struct→scalar coercion sites, `too few outputs`), the
  documented `['a';'b']` multi-row char-array limitation (3), and
  index/struct-boundary singles. `for s = structArray` column
  iteration landed from this tail.
- **2 timeouts** are heavy scripts genuinely computing for >15 s
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
