# Real-world `.m` corpus sweep — compatibility report

## Method

- **Corpus:** 12,728 `.m` files from 38 public GitHub repositories, all
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
- **Runner:** `matcraft-cli eval "addpath(genpath('<repo_root>')); run('<file>')"`
  per file — the `addpath(genpath)` prelude reproduces what every
  toolbox's `startup.m` does for a MATLAB user — cwd = the file's own
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
| clean run | — | **1,913** |
| parse-ok / parse-fail | **10,095 / 4** | — / 5 |
| unknown builtin/function | 0 | 515 |
| runtime error | 6 | 69 |
| missing data/env files | 0 | 119 |
| timeout | 0 | 2 |
| **total** | 10,105 | 2,623 |

- **Parser coverage: 10,095/10,105 function files (99.9%)** parse
  cleanly. Three grammar waves landed against this corpus: (1)
  `get.prop`/`set.prop` classdef accessors, `~` in anonymous params
  (`@(~,~) cb()`), `randi([lo hi])`; (2) `?ClassName` metaclass refs
  (evaluating to `meta.class` objects), `arguments … end` validation
  blocks (tolerated, not enforced), `#` Octave line comments,
  command-form `-flag` args (`mex -outdir bin`) and `~` paths
  (`cd ~/x`), `case'2d'` keyword-quote strings, Fortran `d`/`D`
  exponents (`0.5d-6`), and `zeros(m,n,'uint8')`/`'like',x` type-spec
  tails (accepted; all values are f64 — a documented deviation); (3)
  nested `function`s inside `end`-terminated method bodies
  (`classdef`/`methods` blocks share the file-local hoisting rule),
  `%{ … %}` block comments, `catch x = …` Octave-dialect disambiguation,
  and left-associative chained comparisons (`a < b > c`); (4) `=`/`==`
  tolerance in `if`/`elseif`/`while` conditions (`if nargin = 3`),
  `properties`/`methods` used as ordinary identifiers inside bodies;
  (5) Octave compound assignment (`x += e`, `v .*= 2`), chained
  `a = b = e` (binds right-to-left), and `[a, b]` ending a line with
  `=` opening the next.
- **Remaining parse-fails (9) are not grammar gaps** — each was
  inspected: a truncated `fprintf` fragment (`MATVEC_to_WARP`), raw
  data lines inside an `if(0)` block (`New_input`), a typo'd unclosed
  `if` (`endsourceTFR`), statements glued with no separator
  (`idisp(im)im`, `vector_form matrix_form(i,:)`), an Octave session
  log (`octave_session.m`), a source with spaces inside identifiers
  (`randInitializeWeights`), and an unbalanced `if` in an old unit
  test. MATLAB rejects all of them too. The ten macOS `._*` AppleDouble
  forks are now excluded at harvest time.
- **The 119 env-errors are missing sibling files** — scripts that
  `load`/`imread`/`fscanf` `*.mat`/`*.txt`/`*.jpg`/`*.fig` the harvest
  never downloads, plus `cd` into lab-machine absolute paths. The
  count grew as the `addpath(genpath)` prelude let scripts run deeper.
- **The 515 unknown-builtin failures split into two honest halves**:
  ~140 are real externals — `mexopencv`/`cv` (the OpenCV MEX bridge),
  `mex`, `syms` (Symbolic Toolbox), `sim`/`sl_*` (Simulink), `websave`
  — the rest are sibling-toolbox functions the harvest didn't capture
  (`iread`, `readNEMOH`, `readWAMIT`, `kmeansRnd`, `randinit`) plus
  FieldTrip preamble scripts that reference caller-workspace variables
  (`cfg`) when run standalone. The MEX binary decision is recorded in
  `scope.md` (out of scope) with `loadlibrary`-style FFI as G56.
- **The 75 runtime errors** are a long tail: `A^B` matrix-exponent
  calls that MATLAB rejects identically, deeper per-file semantic gaps
  (struct→scalar coercion sites), the documented `['a';'b']` multi-row
  char-array limitation, index/struct-boundary singles, and files that
  now *reach* unsupported externals (`cv.*` mexopencv calls) after
  wave-7 semantics fixes — an honest error class, not a regression.
- **2 timeouts** are heavy scripts genuinely computing for >15 s
  (FieldTrip/WEC-Sim pipelines), not hangs.

### Runtime-error waves

Two execution waves landed after the grammar work:

- **Wave 7a — empty-array semantics.** `Matrix::cat` skips zero-element
  operands (`[find-empty, 15x1]` keeps the non-empty side), `broadcast`
  treats a `0` dim as compatible-with-anything and empty-dominating,
  and `N.^` lexes as elementwise power (not a `N.` literal).
- **Wave 7b — call-shape fixes.** `randi(imax,[m n])` /
  `repmat(A,[m n])` dims-vector tails, `t = tic` and `h = imshow(A)`
  returning values, `A\b` QR least squares for tall systems,
  `[B,I] = sort(A)` permutation output, and `v = eval('expr')`
  returning the expression's value. Several files now run to
  completion (`SolveMax.m`) or fail later on genuinely-unsupported
  externals (`cv.*`, `fvecs_read`).

## What this corpus exercised that the old one didn't

- **classdef accessors at scale** — `get.x`/`set.x` methods now
  dispatch on `obj.x` reads and `obj.x = v` writes with a self-access
  guard (recursion into the same property's accessor errors, matching
  documented semantics).
- **MEX-facing code** — mexopencv and FieldTrip contributed most of the
  unknown-builtin tail. Binary MEX plugins are declared out of scope
  (`scope.md`): they link the MathWorks `mxArray` ABI, unreachable
  under clean-room rules. The planned native-interop path is
  `loadlibrary`/`calllib` plain-C FFI (gaps.md G56).
- **Toolbox-heavy scripts** — the optimization/control/toolbox merge
  (`fminbnd`, `tf`, `kmeans`, …) is exercised for real by WEC-Sim,
  Corke, and chebfun files.
