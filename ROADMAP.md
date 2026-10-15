# MatCraft roadmap

**Stage: early development (pre-alpha). Engine, language/numerics sweep and native Studio landed.**

## Headline

| Dimension | Now | Target |
| --- | --- | --- |
| Language subset | matrices, ops, logical indexing, control flow, functions, `varargin`/`varargout`, cells, struct arrays, complex, sparse, CSL, ~220 builtins | full M-code |
| Execution | tree-walk interpreter | bytecode VM → Cranelift JIT |
| Linear algebra | pure-Rust LU/QR/Cholesky/eig/SVD/pinv/cond/rank/rref/null/orth | faer-rs backend + complex eigensolver |
| Interface | `matcraft-cli` (run/eval/repl/--cmd/mcp), `matcraft` Studio + control channel | web app, LSP, Jupyter kernel |
| Plotting | `matplot`: 2-D + 3-D (plot3/surf/mesh), imagesc/pie/contour, subplots, colormaps, colorbar, SVG/PNG/PDF export | wgpu 2D/3D interactive window |
| Toolboxes | DSP (`fft`/`ifft`/`filter`/`xcorr`/`freqz`/windows) | control/IO/optimization |

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
