# MatCraft roadmap

**Stage: early development (pre-alpha). Engine, language/numerics sweep and native Studio landed.**

## Headline

| Dimension | Now | Target |
| --- | --- | --- |
| Language subset | matrices, ops, logical indexing, control flow, functions, `varargin`/`varargout`, cells, struct arrays, complex, sparse, CSL, `@` handles, `classdef` OOP, N-D arrays, `global`/`persistent`, `import`, ~440 builtins | full M-code |
| Execution | tree-walk interpreter | bytecode VM → Cranelift JIT |
| Linear algebra | pure-Rust LU/QR/Cholesky/eig/SVD/pinv/cond/rank/rref/null/orth | faer-rs backend + complex eigensolver |
| Interface | `matcraft-cli` (run/eval/repl/--cmd/mcp), `matcraft` Studio + control channel | web app, LSP, Jupyter kernel |
| Plotting | `matplot`: 2-D + 3-D (plot3/surf/mesh), imagesc/pie/contour, subplots, colormaps, colorbar, SVG/PNG/PDF export | wgpu 2D/3D interactive window |
| Toolboxes | DSP (`fft`/`ifft`/`filter`/`xcorr`/`freqz`/windows), optimization (`fminbnd`…`fmincon`/`patternsearch`/`ga`), control (`tf`/`ss`/`zpk`, MIMO/discrete `step`/`lsim`, `bode`/`margin`/`nyquist`/`pid`), statistics (distributions/`fitdist`, `regress`, `kmeans`, `ttest`/`kstest`/`adtest`/`pca`), xlsx/table + HDF5/NetCDF I/O, image processing incl. morphology | MIMO frequency arrays, HDF5 v2/chunked, Canny/bicubic, PCoA/UMAP and multiple-testing corrections |

## Phases

- [x] **Phase 1 — numerical core.** `matcore`: column-major dense `f64`
  matrix, broadcast arithmetic, matmul, LU solve, det, inv, norm, reshape,
  concat, grow-on-assign.
- [x] **Phase 2 — minimal REPL & interpreter.** `matlang` lexer/parser +
  tree-walk interpreter: matrix literals, ranges, indexing with `:`/`end`,
  `if`/`for`/`while`, echo suppression, builtin library, `matcraft-cli`.
- [x] **Phase 2.5 — language basics.** `break`/`continue`/`return`,
  `switch/case/otherwise`, `&&`/`||`/`&`/`|`, `fprintf`/`sprintf`,
  `format long`, command syntax (`format long`, `clear x`).
- [x] **Phase 3 — 2D plotting (headless).** `matplot` crate: figure model +
  SVG renderer. `plot`/`scatter`/`stem`/`bar`, line specs (`'r--o'`),
  `hold`/`grid`/`xlim`/`ylim`, `title`/`xlabel`/`ylabel`/`legend`,
  `figure`/`gcf`/`clf`/`close`, `saveas`/`exportgraphics`/`print` → SVG.
  Extended: `area`/`stairs`/`errorbar`/`histogram`, `semilogx`/`semilogy`/
  `loglog`, `xscale`/`yscale`, `subplot` tiling (`(m,n,p)`, vector `p` spans
  cells, packed `mnp`). Still open: wgpu interactive window, `imagesc`,
  `surf`/3-D, PDF/PNG export.
- [x] **Phase 4 — functions & files.** `function` definitions (multi-output,
  optional `end`), fresh local scopes, hoisted script-local functions,
  `<name>.m` function-file loading from the search path, `nargin`/`nargout`,
  recursion. `varargin`/`varargout` as real cell arrays; `save`/`load` —
  tagged ASCII, numeric-table `load`, and **`.mat` v5 binary** (public spec,
  double + char, `miCOMPRESSED`). Cell arrays landed: `{…}` literals,
  `c{i}`/`c(i)` indexing + assignment + deletion, `for` over cells,
  `cell`/`iscell`/`iscellstr`/`cell2mat`/`num2cell`/`celldisp`.
  Scalar structs landed: `s.f`/`s.f = v`, nested `s.a.b`, dynamic `s.(expr)`,
  `s.v(i)`/`s.c{i}` field indexing, `struct()` name/value pairs,
  `fieldnames`/`isfield`/`rmfield`/`getfield`/`setfield`/`isstruct`.
  Chained subscripts `c{i}(r,c) = v` / `s.c{1}{1} = v` write through any
  depth. Allocation caps on all input-driven allocations; `load`
  validates MAT headers and returns a struct in function form.
  `addpath`/`rmpath`/`path`/`cd`/`pwd`/`dir`. Still open:
  mxCELL/mxSTRUCT in `.mat` v5.
- [x] **Phase 5 — engine + language sweep.** `engine` command layer
  (`eval.line`, `workspace.clear`, `figure.export`…): 14 commands,
  panic-guard rollback `Session::execute`, `matcraft-cli --cmd` repeat flag
  and `matcraft-cli mcp` (JSON-RPC 2.0 stdio, tools + resources),
  `cargo xtask layers` in CI. Numerics: `matcore::linalg` — Householder QR,
  Cholesky, Jacobi symmetric eig, Hessenberg+QR `eigvals`, SVD, `pinv`,
  `rank`, `cond`, `trace`, `rref`, `null`, `orth` (pure Rust; faer deferred —
  not vendored here). Language: dim-aware reductions (`sum(A,2)`,
  `sum(A,'all')`), multi-output builtins (`size`/`min`/`max`/`find`/`eig`/
  `svd`/`qr`/`lu`/`meshgrid`/`histcounts`), logical indexing incl. masked
  assignment, M-code matrix-literal whitespace rule (`[0 -2 5]` vs
  `[0 - 2]`), command-style quoted strings (`save 'f.mat'`),
  `tic`/`toc`, `fzero`/`fminsearch`/`integral`/`quad`, `interp1`/`trapz`,
  `poly*`/`roots`/`conv`/`deconv`, string library, `exist`/`which`.
- [x] **Phase 6 — MatCraft Studio.** `ui-egui` (egui 0.36, thin UI over the
  engine): command window with history, workspace inspector, editor pane
  (`.m` open/save/run), figure dock (SVG → texture via resvg), all commands
  routed through `Session::execute`. `apps/matcraft` adds a JSON-lines TCP
  control channel (`--control PORT`: `engine.execute`, `engine.list`,
  `ui.state`, `ui.render` → PNG, `ui.screenshot` → PNG) with bounded line
  reads and a connection cap. Verified end-to-end by TCP smoke tests and
  headless dispatch tests.
  Still open: `matcraft-web` (wasm), macOS/Linux packaging.

## Further out

- Bytecode VM, then Cranelift JIT for loop hotspots.
- Toolboxes as separate crates: signal (rustfft), control, data I/O
  (`.mat` v5/v7.3, CSV/Parquet), optimization (LM curve fitting).
- LSP backend + Jupyter kernel protocol.
- WASM target (craftrules: never break wasm).

## Progress log

- Initial scaffold: `matcore`, `matlang`, `matcraft-cli`, xtask, docs.
- Language basics: `break`/`continue`/`return`, `switch`, `&&`/`||`, `&`/`|`,
  `fprintf`/`sprintf`, `format`, command syntax.
- Functions: `function` defs, multi-output `[a,b] = f(x)`, local scopes,
  script-local hoisting, `.m` function-file loading, `nargin`/`nargout`;
  two-arg elementwise `min`/`max`.
- Engine: `matcraft_engine` command registry (`eval.line`, `script.run`,
  `workspace.*`, `path.*`, `figure.*`, `engine.*`, `session.reset`,
  `functions.list`); panic guard snapshots and restores the session.
  `matcraft-cli` gained `--cmd 'id {json}'` and `mcp` (JSON-RPC tools:
  `eval`, `run_script`, `execute`, `list_commands`, `workspace`; resources:
  `matcraft://workspace`, `matcraft://commands`).
- Plotting: `matplot` crate — headless SVG; `plot`/`scatter`/`stem`/`bar`,
  spec strings, `hold`/`grid`/labels/`legend`/`xlim`, `figure`/`gcf`/`clf`,
  `saveas`/`exportgraphics`/`print`.
- Numerics: `matcore::linalg` — QR, Cholesky, symmetric + real-spectrum eig,
  SVD, `lu`, `pinv`, `rank`, `cond`, `trace`, `rref`, `null`, `orth`; all
  exposed as builtins with multi-output forms.
- Language sweep: whitespace-sensitive matrix literals (lexer keeps
  `ws_before`), logical indexing + masked assignment, dim-aware reductions,
  `varargin`/`varargout`, `save`/`load`, path builtins, `tic`/`toc`, solvers
  (`fzero`/`fminsearch`/`integral`), poly/interp, ~190-name builtin table.
- Plotting sweep: `area`/`stairs`/`errorbar`/`histogram`, log axes via
  `semilogx`/`semilogy`/`loglog`/`xscale`/`yscale` — nonpositive points are
  skipped under log transforms.
- Studio: `ui-egui` + `matcraft` — command window, workspace table, editor,
  figure dock (resvg rasterizer), control channel on `--control PORT`.
  egui 0.36 port used the new `logic`/`ui` app split; fixed a layout
  feedback loop where an expanding `ScrollArea` inside a content-sized
  `Panel::bottom` ratcheted the panel taller every frame (bounded the
  scroll height). `FUNCTION_GROUPS` now mirrors `matlang::BUILTIN_NAMES`,
  enforced by a test.
- Packaging: `packaging/windows/` WiX installer + portable zip
  (`matcraft-0.1.0-windows-x64.msi`), icon embedded via `winresource`,
  correct PE subsystems (GUI/console); installed to
  `C:\Program Files\MatCraft` and verified. Optional Authenticode/Azure
  signing via `sign.ps1` when secrets are present.
- Family integration: `artcraft-cli` skill lists MatCraft (binary, native
  `.m`/`.mat` formats, command ids, gotchas); `_learn/matcraft_commands.txt`
  and `references/commands/matcraft_commands.txt`; demo project
  `14-MatCraft-ForecastAnalysis` in the playground (`polyfit` forecast,
  SVG exports, `.mat` save, reproducible `build.py`).
- Cells + structs: `CellArray`/`Value::Struct` values, `{…}` literals,
  brace/paren indexing and assignment, `struct()`/`isfield`/`rmfield`/
  `fieldnames`/`getfield`/`setfield`, `varargin`/`varargout` as cells.
- Hardening sweep (test-agent driven): allocation caps on every
  input-derived path (`zeros(1e9)` errors instead of aborting), `.mat`/ASCII
  `load` header validation (no silent garbage files), `save` errors on
  undefined vars, `subplot` bounds-checking, fractional-index errors,
  `c(:)`/`A(:)` columnize, `S = load('f')` returns a struct, struct builtins
  strict (`isfield` arg checks + cellstr form, `rmfield` on missing fields,
  `s.()` identifier validation), `{a} {b}` whitespace-separated cell
  literals, chained writes `c{1}{1}=v`/`c{1}(i)=v`/`s.c{1}.f=v` via the
  unified `PathStep` assignment path, control-channel eval echo into
  Command Window/History.
- Semantic sweep: char→double promotion (`to_numeric` — arithmetic,
  comparisons, `'` transpose, `x(i)='a'`, `['a' 'b']`/`[72 'i']`, `''` →
  0×0); cell-aware `reshape`/`repmat`/`fliplr`/`flipud`/`cat`/`horzcat`/
  `vertcat`, `num2cell(A,dim)`+char, `char(cellstr)`, `c(:,j)=[]`;
  comma-separated lists (`c{1:3}` → `f(c{:})` args, `[s.f]`/`{c{:}}`
  literals, `[a,b]=c{1:2}` multi-assign, `A(c{:})` indices); `try/catch`+
  `error(id,msg)`+`e.message`/`e.identifier`, `~` ignored outputs/params,
  `narginchk`/`nargoutchk`; `A(i)=[]`/`A(:,j)=[]`/`A(i,:)=[]` matrix
  deletion; `save f -ascii` flag form via glued command-word args.
- Type sweep (Phase C): complex numbers (`Value::Cplx`/`CMatrix`, `i`/`j`,
  `real`/`imag`/`conj`/`abs`/`angle`/`complex`/`isreal`, `'` ctranspose vs
  `.'`, domain promotion `sqrt(-4)`→`2i`, auto-demote when imag=0, complex
  `eig` via `eigvals_c`, complex `.mat` v5 FLAG_COMPLEX + tagged-ASCII);
  struct arrays (field-oriented `StructArray`, `s(2).f`, `s.f` CSL
  expansion, `struct('f',{…})`, `orderfields`, `deal`, growth/delete,
  `[s t]` concat); sparse matrices (`Value::Sparse`/`SparseMat`,
  `sparse`/`full`/`nnz`/`nzmax`/`nonzeros`/`speye`/`spalloc`/`spones`/
  `issparse`, indexing/assign/growth, sparse arithmetic + matmul +
  transpose + concat, `find` column-major order, mxSPARSE `.mat` v5
  read/write).
- DSP toolbox: `matcore::dsp` — mixed-radix `fft`/`ifft`/`fft2`/`ifft2`
  (prime lengths via naive DFT), `fftshift`/`ifftshift`, `dftmtx`, `filter`
  (DF-IIT with `zi`/`zf`), `xcorr` (lags + biased/unbiased/coeff), `freqz`
  (`[h,w]`, `'whole'`, `f,fs` Hz form), `unwrap`, window functions
  (`hamming`/`hann`/`blackman`/`bartlett`/`rectwin`/`triang`/`kaiser`/
  `gausswin`); `roots` upgraded to `eigvals_c` for complex roots.
- Plotting breadth (Phase D10): `imagesc`/`image` (colormap cells,
  ij-axis), `pie` (labels), `contour` (marching squares, `n`/explicit
  levels), `plot3`, `surf` (painter-order shaded quads), `mesh`
  (wireframe) under `view(az,el)`/`view(2)`/`view(3)`; `zlim`/`zlabel`,
  `colormap` (parula/jet/hot/gray), `colorbar`, `caxis`/`clim`; PNG
  (resvg) and PDF (embedded raster) export via `saveas`/`exportgraphics`
  extension or `print -dsvg/-dpng/-dpdf`.
- Optimization toolbox wave (2026-11): `fminbnd` (Brent), N-parameter
  `fminsearch` (Nelder–Mead), `fminunc` (BFGS with central-difference
  gradients), and `fsolve`/`lsqnonlin`/`lsqcurvefit` (Levenberg–Marquardt,
  box bounds, jacobian + lambda outputs), all with MATLAB option aliases
  (`MaxIterations`/`MaxIter`, `TolX`/`StepTolerance`, …) via
  `optimset`/`optimoptions`. Tests cover the documented multi-output forms.
  Second wave: `linprog`/`quadprog` on a shared primal-dual interior-point
  solver (bounds folded into inequalities, `problem`-struct interface,
  `lambda.ineqlin`/`eqlin`/`lower`/`upper`, active-set polish for accurate
  degenerate bounds), exercised against representative external M-code call shapes.
  Third wave: `fmincon` (SQP — linearised-constraint QP subproblems, BFGS
  Lagrangian Hessian, L1 merit line search, `[c,ceq]` nonlinear
  constraints, `problem` struct, `lambda.ineqnonlin`/`eqnonlin`) and
  `patternsearch` (±coordinate + diagonal derivative-free polling, same
  constraint set and merit function).
- Control toolbox wave (2026-11): SISO `tf`/`ss`/`zpk` objects with native
  `s = tf('s')` algebra, `tfdata`/`ssdata`/`zpkdata`, `pole`/`zero`/`dcgain`,
  `series`/`parallel`/`feedback`, `tf2ss`/`ss2tf`, `step`/`impulse`/
  `initial`/`lsim` via zero-order-hold state space on the new
  `matcore::expm`, and `bode`/`freqresp`. `examples/control.m` tours the
  interfaces.
- Statistics toolbox wave (2026-11): `matcore::stats` — erf/erfc, Lanczos
  `gammaln`, regularized incomplete gamma/beta, normal/exponential/uniform/
  Poisson/Binomial PDF/CDF/inverses and inverse t/F CDFs. Builtins:
  `norm*`/`exp*`/`unif*`/`pois*`/`bino*` (PDF/CDF/inverse), seeded
  `exprnd`/`unifrnd`/`poissrnd`/`binornd`, `iqr`, `corrcoef` (R + p),
  `regress` (b/residuals/R²/F/p), and `kmeans` (k-means++/Lloyd,
  replicates, four distance metrics, singleton empty clusters).
  `examples/stats.m` exercises the interfaces.
- XLSX/table I/O wave (2026-11): a dependency-free `.xlsx` ZIP store/
  reader (stored + deflated entries) and OpenXML reader/writer covering
  shared strings, inline strings, numbers, booleans, A1 ranges and multiple
  sheets. Builtins: `xlsread`/`xlswrite` (`[num,txt,raw]`, sheet/range),
  `readcell`/`writecell`, `readtable`/`writetable` (class-tagged `table`
  objects with `Properties.VariableNames` and per-variable fields),
  `height`/`width`/`istable`, and `readmatrix`/`writematrix` dispatch for
  `.xlsx`. `examples/xlsx_io.m` exercises the round trips.
- Image-processing wave (2026-11): grayscale/RGB N-D image support with
  `rgb2gray`, `imresize` (nearest/bilinear), `imfilter` (correlation/
  convolution, same/full, four boundary modes), `medfilt2`, `edge`
  (Sobel/Prewitt/Roberts), `bwlabel` (4/8 connectivity), `regionprops`
  (Area/Centroid/BoundingBox/PixelIdxList/PixelList/Extent/EquivDiameter),
  and Otsu `graythresh`/`imbinarize`. `examples/image_processing.m`
  demonstrates the pipeline.
- Control depth wave (2026-11): `pid(Kp,Ki,Kd[,Tf])`, `margin`
  (`[Gm,Pm,Wcg,Wcp]` with unwrapped-phase interpolation), `nyquist`
  (`[re,im,w]`), `isstable` (continuous poles or discrete `Ts` unit
  circle), `damp` (`[wn,zeta,p]`, mapping discrete poles through
  `log(z)/Ts`) and `bandwidth`. MIMO/discrete response wave: `ss` `step`/
  `impulse` return N×p×m arrays, `lsim` accepts N×m inputs, `initial`
  returns N×p, and `Ts > 0` uses the direct discrete recurrence instead of
  the matrix exponential.
- Genetic algorithm wave (2026-11): seeded real-valued `ga` with tournament
  selection, blend crossover, Gaussian mutation, elitism, box bounds,
  linear/nonlinear constraints (`nonlcon`), `problem` structs, and
  `[x,fval,exitflag,output,population,scores]`; options include
  `PopulationSize`, `MaxGenerations`, `MaxStallGenerations`,
  `CrossoverFraction`, `EliteCount`, `InitialPopulation`.
  `examples/ga.m` demonstrates it.
- HDF5 v2/chunked wave (2026-11): reader support for superblocks v2/v3,
  object headers v1/v2, link messages, compact/contiguous/chunked datasets,
  fixed-array and B-tree v1/v2 chunk indexes, and shuffle/deflate/fletcher32
  filters. Validated against h5py 3.16/HDF5 2.0 files with
  `libver='earliest'` and `'latest'`, gzip, shuffle and B-tree v2 fixtures.
- HDF5/NetCDF wave (2026-11): pure-Rust HDF5 v0 writer/reader (superblock
  v0, object header v1, root symbol table/B-tree/local heap, contiguous
  little-endian f64 datasets) validated against h5py in both directions;
  NetCDF classic CDF-1 writer + CDF-1/2 reader (dimensions, numeric
  variables, absent-list handling) validated against SciPy. Builtins:
  `h5read`/`h5write`/`h5create`/`h5disp`, `ncread`/`ncwrite`/`ncinfo`/
  `ncdisp`, with start/count/stride selection.
- Hypothesis-test wave (2026-11): `ttest`/`ttest2` (Welch or pooled
  variance, `Alpha`/`Tail`/`Vartype`), `kstest` (standard normal, function
  handle or tabulated CDF), `adtest` (Anderson–Darling with
  estimated-parameter p-value and critical values) and `pca`
  (eigen-decomposition of the covariance, descending coefficients/scores/
  latent roots, T², explained variance, `NumComponents`). Values
  cross-checked against SciPy 1.16.
- Distribution fit wave (2026-11): gamma/beta/chi-square/t/F PDF, CDF and
  quantile kernels in `matcore::stats` (including digamma/trigamma),
  `fitdist` for Normal/Exponential/Gamma/Beta/Poisson returning
  `prob.*Distribution` objects, and `pdf`/`cdf`/`icdf`/`random` plus
  `mean`/`std`/`var`/`median` dispatch on those objects. MLE values
  cross-checked against SciPy 1.16.
- Multivariate/multiple-testing wave (2026-11): `cmdscale`/`pcoa`
  classical multidimensional scaling, `bonferroni`, `holm`, `sidak`,
  `benjamini_hochberg` and `mafdr` p-value corrections, plus `signrank`
  (exact small-sample distribution), `ranksum` (Mann–Whitney) and
  `kruskalwallis`. Cross-checked against SciPy 1.16.
- MIMO frequency wave (2026-11): `bode` returns real `p×m×N`
  magnitude/phase arrays and `freqresp` a plain complex `(p*m)×N` matrix
  (row `out*m+inp`) using `C(jwI-A)^-1B+D`; SISO shapes are unchanged.
  Cross-checked against analytic diagonal and shared-input state-space
  responses. `examples/control.m` demonstrates it.
- Parallel/GPU wave (2026-11): Rayon-backed large matrix multiplication,
  `maxNumCompThreads`/`parpool` worker configuration, and an optional `gpu`
  feature (`gpuArray`/`gather`/`canUseGPU`/`gpuDevice`) that runs matmul on a
  wgpu compute shader with a transparent CPU fallback.
- Editor wave (2026-11): MATLAB-style syntax colours and an Excel-style
  function-signature popup that follows the caret, highlights the current
  argument and understands local `function` definitions.
- Class wave (2026-11): `addlistener`/`notify` handle events, object
  arrays with growth/deletion/concatenation, and enforced
  `Access`/`GetAccess`/`SetAccess`, `Constant`, `Dependent`, `Abstract`
  and `Sealed` attributes.
- Final compatibility sweep (2026-11): table grouping/joining/cleanup,
  categorical/datetime basics, `fitlm`/`boxplot`/`optimalleaforder`,
  web/UI and parallel-compatibility stubs, ODE mass matrices/events/`deval`,
  Lanczos/anti-aliased resize, expanded `regionprops`, and HDF5 fixed-array
  indexes beyond 1024 chunks. Tests use synthetic/random data and
  representative external workflows.
- Graphics/table/math compatibility wave (2026-11): explicit axes
  tick/label/angle controls, `sgtitle`, `tiledlayout`/`nexttile`, `barh`,
  `yline`, graphics-handle helpers and `savefig`; table conversion/editing
  helpers; `containers.Map`; `diary`; text extract/insert/predicate helpers;
  JSON/import helpers; validation and `rethrow`; `corr`, `tabulate`,
  `gammaln`, `hygecdf`, `nchoosek`, `pdist`, `squareform`, `linkage`,
  `partialcorr`, `isoutlier`, `isbetween`; `expm`, `vecnorm`, `blkdiag`,
  `mat2cell` and `gradient`. Tested with synthetic data and existing
  workflows.
- Corpus compatibility wave (2026-11): MATLAB implicit expansion,
  delimited-text `readtable`/`writetable`, table subsetting/`sortrows`/dot
  assignment, cellstr `str2double`/`strcmp`, `erase`/`replace`,
  `mfilename('fullpath')`, empty-cell `end+1` growth, `lines`/`validatecolor`,
  `rectangle`/`patch`/`daspect`, figure property pairs and method-form
  `findobj`, plus a 64 MB CLI interpreter stack for debug builds. Exercised
  end-to-end on representative external M-code with real data.
- Corpus class wave (2026-11): handle-based `inputParser` with
  `addRequired`/`addOptional`/`addParameter`/`addParamValue`/`parse`,
  validators, case-insensitive/unique-prefix matching and `KeepUnmatched`;
  `string`/`strings`/`isstring` using scalar strings plus cellstr fallback;
  `table` creation with identifier/default/explicit variable names and
  `table2array`. Tests cover all paths, and `examples/corpus_classes.m`
  demonstrates the workflow.
- HDF5 chunked-writing wave (2026-11): the writer now emits superblock v3,
  v2 object headers/link messages, chunked little-endian f64 datasets with
  gzip-compressed chunks and fixed-array chunk indexes, plus Jenkins lookup3
  metadata checksums. Round-trips are checked with the native reader and
  h5py 3.16/HDF5 2.0 (multiple datasets, 100×100 multi-chunk grids, 1-D row
  chunks, gzip/chunk metadata).
- Canny/bicubic wave (2026-11): Keys bicubic `imresize` (`a=-0.5`,
  column-major output fixed) and Canny `edge` (Gaussian smoothing, Sobel
  gradients, subpixel non-maximum suppression, double-threshold hysteresis,
  optional sigma). Cross-checked against SciPy-built bicubic reference grids
  and an independent vertical-step Canny test. `examples/image_processing.m`
  demonstrates both.
- Corpus file/shell wave (2026-11): persisted `savepath`/`path` search
  paths, `addpath -begin/-end`, `dos`/`unix` shell aliases alongside
  `system` and `!cmd`, dimensional `true`/`false`, and `xline`; regression
  tests cover path persistence, shell output, globals across functions,
  `mkdir`/`rmdir`, and `genpath`.
- ANOVA wave (2026-11): `anova1` (matrix columns, cell groups, vector +
  labels), `anova2` (with and without replication), `multcompare` (LSD,
  Bonferroni, Sidak, Holm and finite-df Tukey–Kramer studentized range) and
  `friedman` (with replication), with ANOVA cell tables and stats structs.
  Checked against SciPy `f_oneway`, `friedmanchisquare` and closed-form
  pairwise t comparisons. `examples/anova.m` tours it.
- ODE wave (2026-11): `ode45` (adaptive Dormand–Prince 5(4)) and
  `ode15s` (stiffly stable TR-BDF2 with step doubling and finite-difference
  Newton Jacobians), `odeset`/`odeget`, scalar/vector tolerances, `MaxStep`,
  `InitialStep`, `MaxIterations`, `NonNegative`, and cubic-Hermite output
  interpolation. Checked against closed-form decay/oscillator/stiff
  solutions and SciPy BDF on Robertson kinetics. `examples/ode.m` tours it.
- Corpus-compatibility wave (2026-11): mined representative external
  M-code corpora for missing generic helpers, adding `contains`,
  `startsWith`, `endsWith`,
  `strsplit`, `strjoin`, `strlength`, `fileread`, `isfolder`, `rows`,
  `array2table` and `ismissing`. `examples/corpus_helpers.m` exercises them.
- Manifold wave (2026-11): dependency-free `umap` — fuzzy k-nearest
  neighbour graph, per-point bandwidth calibration, symmetrised weights,
  `1/(1+a x^(2b))` curve fit, seeded random initialisation and negative-
  sampled cross-entropy SGD. Options for neighbours, minimum distance,
  components and epochs. `examples/stats.m` demonstrates it on two clusters.
- Morphology wave (2026-11): grayscale/binary/RGB `imdilate`, `imerode`,
  `imopen`, `imclose` with `strel` (`square`, `rectangle`, `disk`,
  `diamond`, `line`, `arbitrary`), nearest/bilinear `imrotate` with
  loose/crop bounding boxes, and `imcrop`. `examples/image_processing.m`
  exercises the operations.
- Wild-corpus sweep (2025-10): 638 real-world `.m` files pulled from 32
  GitHub repos across 8 domains, all last-pushed pre-2022; run through
  `matcraft-cli run` (see `docs/wild-corpus.md`). Findings fixed this
  round: `for(i=…)` parenthesised loop headers, `cd ..`/`cd ../x`
  bare-dot command args, and the cheap-builtin tier — `deg2rad`
  `rad2deg` `sinc` `randperm` `normrnd` `assert` `rng` `fullfile`
  `genpath` `csvread` `mkdir` `eps()`/`Inf(n,m)`/`NaN(n)` constructors,
  `axis`/`box`, `db2pow`/`pow2db`, `uint8`, `rectpuls`/`tripuls`,
  `addpath` splitting `;`-lists. Ranked gaps surfaced: `@` anonymous
  functions/handles (G31 — 91 parse failures), `classdef` OOP (G32),
  N-D indexing (G22 tail), image I/O (G34). Initial GitHub import pushed
  to `github.com/NibrasAz7/matcraft`.
- Studio toolstrip + chrome pass (2025-10): MATLAB-style tabbed ribbon
  (HOME / PLOTS / EDITOR / VIEW) replacing the flat toolbar — captioned
  section groups, contextual EDITOR tab auto-raised with the document,
  PLOTS gallery acting on the Workspace selection (or a `f(` stub on the
  command line). Path bar gains back/forward stacks + recent-folder
  dropdown; document tabs show `*` for dirty buffers and an accent
  underline for the raised tab; Command Window renders `Error` blocks in
  red / `Warning` in dark orange; desktop shortcuts Ctrl+N/O/S/W, F5 run;
  MATLAB-ish light palette + a minimal Preferences window (zoom, layout
  reset). SimCraft spec landed as `../simcraft/` (README + ROADMAP +
  AGENTS) — the Simulink-class sibling interconnected via `sim`,
  `set_param`/`get_param`, workspace-exchange blocks and M-code
  callbacks.
- Language saturation waves (2026-10): the corpus findings above all
  landed. `@` handles + closures (`f623a3e`), `run` + `dbstop`/`dbcont`
  debugger core (`46483c2`), WASM Studio shell (`05b0175`), live
  breakpoint UI (`c35af3d`), N-D arrays + indexing (`fbaa859`),
  craft-libs adoption — atomic saves, settings store, quit prompt
  (`baeacc5`), macOS/Linux packaging + release workflow (`ec64cb2`),
  craft-ai panel + `ai.*` commands (`bbf9522`), ~140 corpus-driven
  builtins — file I/O, regex, stats, image I/O (pure-Rust PNG),
  system/env, dates, sets/bits/casts (`6007c27`), latin-1 `.m` decoding
  on every source path (`1d78428`), `global`/`persistent`/`import` +
  destructuring LHS + grammar gaps (`4cf61f6`), and `classdef` OOP with
  value/handle classes and `@dir` dispatch (`50e8e90`). Corpus re-run:
  parse-fail 93 → 2 (both non-code doc files), 632/645 function files
  parse (98%), 39/143 scripts run clean.
