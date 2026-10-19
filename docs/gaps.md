# MatCraft — ranked gap list

The work list. Pick from the top; when a gap closes, move it to the ROADMAP
progress log and delete the row here. Order is current priority, not difficulty.

## P0 — family parity scaffolding (everything-is-a-command spine)

| # | Gap | Notes |
| --- | --- | --- |
| ~~G1~~ | ~~No `engine` crate~~ | **Done 2026-10-14:** `matcraft_engine` — 14 commands, `Session::execute` with panic-guard rollback. |
| ~~G2~~ | ~~No `mcp` crate~~ | **Done:** `matcraft_mcp` JSON-RPC stdio server, `matcraft-cli mcp`. |
| ~~G3~~ | ~~CLI lacks `--cmd`~~ | **Done:** `run`/`eval` accept repeated `--cmd 'id {json}'`; `commands` subcommand lists the catalogue. |
| ~~G4~~ | ~~No `xtask layers` check~~ | **Done:** `cargo xtask layers` enforces `matcore → matplot → matlang → engine → mcp/ui → apps` in CI. |
| ~~G5~~ | ~~No `cargo xtask` alias~~ | **Done:** `.cargo/config.toml` added. |

## P1 — Studio + install

| # | Gap | Notes |
| --- | --- | --- |
| ~~G6~~ | ~~No `ui-egui` crate / `matcraft` Studio app~~ | **Done 2026-10-14:** command window, workspace inspector, editor, figure dock (resvg rasterizer); thin UI over `engine`. |
| ~~G7~~ | ~~No control channel~~ | **Done:** `--control PORT` → JSON-lines TCP server in `apps/matcraft` (`engine.execute`, `engine.list`, `ui.state`, `ui.render`, `ui.screenshot`, `ping`); bounded lines, connection cap, verified by TCP smoke tests + dispatch tests. |
| ~~G8~~ | ~~No `apps/matcraft-web`~~ | **Done:** WASM Studio shell via `eframe::WebRunner` + Trunk (`05b0175`); `cargo xtask wasm` gates wasm32 in CI. Remaining: publish/host it. |
| ~~G9~~ | ~~No `packaging/`~~ | **Done:** Windows WiX MSI + portable zip (`packaging/windows/`, verified installed); macOS `.app`/`.dmg` + Linux tarball/`.deb` + `release.yml` three-OS matrix (`ec64cb2`). Remaining: real-device macOS/Linux QA, signing/notarization secrets. |
| ~~G10~~ | ~~Not in the skill catalog~~ | **Done:** `artcraft-cli` SKILL.md row + gotchas, `references/commands/matcraft_commands.txt`, `_learn/matcraft_commands.txt`; `14-MatCraft-ForecastAnalysis` demo in the playground. |
| ~~G35~~ | ~~No debugger hooks for Studio breakpoints~~ | **Done:** `dbstop`/`dbclear`/`dbstatus`/`dbcont`/`dbstep`/`dbquit` + `run` with real pause continuations (`46483c2`); Studio wires gutter clicks → `dbstop`, amber stopped-line highlight, Continue/Step/Quit, `K>>` prompt (`c35af3d`). Limitation: breakpoints match top-level statements — can't suspend mid-`for`-body yet (needs per-statement lines in the AST). |

## P2 — language completeness

| # | Gap | Notes |
| --- | --- | --- |
| ~~G11~~ | ~~`varargin`/`varargout` are numeric only~~ | **Done:** real cell arrays — `varargin`/`varargout` are 1×n cells. |
| ~~G12~~ | ~~No structs~~ | **Done:** scalar structs — `s.f`/`s.f = v`, nested `s.a.b`, dynamic `s.(expr)` (validated as an identifier), field indexing `s.v(i)`/`s.c{i}` incl. chained `s.c{1}(i)`, `struct('f',v,…)`, `isstruct`/`fieldnames`/`isfield`/`rmfield`/`getfield`/`setfield` with strict arg checks; `s.c{1}.f = v` and `s.(s.k) = v` (dynamic name resolved before the root detaches) work via the unified assignment path. Struct **arrays** landed too — `s(2).f`, `s(1:2).f` (CSL expansion), `struct('f',{…})` cell expansion, `orderfields`, `deal`, growth `s(4).f=v`, `s(i)=[]` delete, `[s t]`/`cat` concat on matching field sets. Still open: mxSTRUCT in `.mat` v5. |
| ~~G13~~ | ~~No `.mat` v5 `save`/`load`~~ | **Done:** binary v5 reader/writer (public spec) for double + char; `miCOMPRESSED` wrapped `miMATRIX`. `load` validates the v5 header (no silent ASCII fallback), `S = load('f')` returns a struct, `save` errors on undefined vars, `save f -ascii` command-flag form parses (command args glue `f.txt`/`a-b` into one whitespace-separated word). mxSPARSE reads/writes (`ir`/`jc`/`pr` CSC triplets). Remaining: mxCELL/mxSTRUCT classes, N-D dims. |
| ~~G14~~ | ~~No logical indexing / `find`~~ | **Done:** `A(A>0)` for reads and masked assignment `A(A>0)=x`; `find` single/multi-output. |
| ~~G15~~ | ~~Strings are minimal~~ | **Done:** `strcat`/`strcmp`/`strcmpi`/`num2str`/`str2double`/`str2num`/`upper`/`lower`/`strtrim`/`strrep`/`strfind`; char arrays are still row-only. |
| ~~G16~~ | ~~No `try/catch`, `error` ids, `narginchk`~~ | **Done:** `try … catch [e] … end` (`e.message`/`e.identifier`), `error(msg)` and `error(id, msg)`, `narginchk`/`nargoutchk`. |
| ~~G17~~ | ~~`~` ignore-output, comma-separated lists~~ | **Done:** `~` on `[a, ~, b] = f()` LHS and in function output/param lists; comma-separated lists — `c{1:3}` expands in call args (`f(c{:})`, `varargin`), `[s.f]`/`{c{:}}` literals, `[a,b] = c{1:2}` multi-assign, `A(c{:})` index positions. `end` in function files still open. |
| ~~G17b~~ | ~~No char→double promotion~~ | **Done:** `Value::to_numeric` — chars promote to code-point doubles in arithmetic, comparisons, `'` transpose, `x(i)` numeric writes, `[…]` literals; `['a' 'b']`/`[72 'i']` stay char; `''` is `0×0`. Remaining: char arrays are still row-only (no 2-D char matrix → `char` on multi-element cellstr unsupported). |
| ~~G17c~~ | ~~Cell-blind shape builtins~~ | **Done:** `reshape`/`repmat`/`fliplr`/`flipud`/`cat`/`horzcat`/`vertcat` on cells, `num2cell(A,dim)` + char, `char(cellstr)` (scalar), `c(:,1)=[]`/`c(1,:)=[]` row/col delete, `A(i)=[]` matrix deletion (bounds-checked, vectors keep orientation). |
| ~~G31~~ | ~~No `@` handles / anonymous functions~~ | **Done (`f623a3e`):** `@name` handles, `@(x) expr` closures capturing the defining workspace, `feval`/`func2str`/`str2func`/`functions`, `arrayfun`/`cellfun` with handles, `inline`. |
| ~~G32~~ | ~~No `classdef` / OOP~~ | **Done:** `classdef X < super`, `properties`/`methods`/`enumeration`/`events` sections with attribute lists, bodiless signatures, `Name(args)` ctors, `obj.prop` get/set, `obj.m(…)` + `m(obj,…)` dispatch, `obj@Super`/`name@Super` calls, value semantics + `handle` shared state, `@dir` method files (class-private), `isa`/`class`/`isobject`/`methods`/`properties`, operator overloads (`plus`/`eq`/`horzcat`/`transpose`/…), `Class.enum` values. Deferred: object arrays, `events` listeners, attribute enforcement (`Access`/`Constant`/`Dependent`/`Abstract`/`Sealed`), packages, `metaclass`, enumeration objects, `.mat` objects, `subsref`/`subsasgn`. |
| ~~G33~~ | ~~Corpus cheap wins~~ | **Done** (2025 sweep): `for(i=…)` paren headers, `cd ..`/`cd ../x` bare-dot command args, `deg2rad` `rad2deg` `sinc` `randperm` `normrnd` `assert` `rng` `fullfile` `genpath` `csvread` `mkdir` `eps()`/`Inf(n,m)`/`NaN(n)` constructors, `axis`/`box`, `db2pow`/`pow2db`, `uint8`, `rectpuls`/`tripuls`, `addpath` splits `;`-lists. |
| ~~G34~~ | ~~Image I/O~~ | **Done (`6007c27`):** `imread`/`imwrite`/`im2double`/`fspecial` — PNG codec in pure Rust (`imgpng.rs`), M×N / M×N×3 doubles. Remaining: `imshow` (needs the matplot image path), other formats (JPEG). |

## P3 — numerics depth

| # | Gap | Notes |
| --- | --- | --- |
| ~~G18~~ | ~~LU only~~ | **Done:** `matcore::linalg` — `eig`/`eigvals`, `svd`, `qr`, `lu`, `chol`, `rref`, `pinv`, `rank`, `cond`, `trace`, `null`, `orth`. Complex spectra via `eigvals_c` (double-shift QR, 2×2 blocks → complex pairs). Still missing: `eigs`. |
| G19 | No `faer` backend | Phase 5 swap; `matcore` API is stable so the backend is pluggable. Not vendored in this environment — pure Rust stands. |
| ~~G20~~ | ~~Elementwise/stdlib gaps~~ | **Done:** `cumsum`/`cumprod`/`diff`, `sort`(+`'descend'`/dim), `unique`, `polyval`/`polyfit`/`roots`/`poly`/`conv`/`deconv`, `interp1`, `trapz`, `std`/`var`/`median`, `repmat`, `kron`, `diag`, `triu`/`tril`, `fliplr`/`flipud`, `circshift`, `rot90`, `hypot`, `nthroot`, `nextpow2`, `pow2`. |
| ~~G21~~ | ~~No complex numbers~~ | **Done:** `Value::Cplx(CMatrix)` — `i`/`j` constants + `2i` glued literals, `real`/`imag`/`conj`/`abs`/`angle`/`complex`/`isreal`, `'` ctranspose vs `.'`, domain promotion (`sqrt(-4)`→`2i`), auto-demote when imag=0, complex eig/det/inv/norm/trace/reductions/sort/find, complex `.mat` v5 (FLAG_COMPLEX pr/pi) + tagged-ASCII round-trips. |
| ~~G22~~ | ~~Sparse~~ / ~~N-D~~ | **Sparse done:** `Value::Sparse(SparseMat)` — `sparse`/`full`/`nnz`/`speye`/`spalloc`/`spones`/`issparse`/`nonzeros`, sparse-aware indexing/assign/growth/delete, `+` `-` `*` `.*` `./` transpose, `[S T]` concat, `find` column-major, mxSPARSE `.mat` v5. **N-D done (`fbaa859`):** `dims: Vec<usize>` storage, `zeros(2,3,4)`, `x(i,j,k)` merged-tail + linear indexing, `ndims`/`cat(dim)`/`reshape`/`squeeze`, page display, mxDOUBLE N-D `.mat` round-trip. 2-D-only ops reject N-D operands cleanly. |

## P4 — plotting breadth

| # | Gap | Notes |
| --- | --- | --- |
| ~~G23~~ | ~~`imagesc`, `pie`, `contour`~~ | **Done:** `imagesc`/`image` (grid cells via colormap, y-reverse axis), `pie` (wedges + labels), `contour` (marching squares, auto/explicit levels, `(Z)`, `(Z,n|v)`, `(X,Y,Z[,…])`). Remaining: filled contours (`contourf`), image upsampling. |
| ~~G24~~ | ~~PNG/PDF export~~ | **Done:** `matplot::to_png` (resvg rasterizer) + `to_pdf` (raster embedded as FlateDecode image); `saveas`/`exportgraphics` dispatch on extension (`.svg`/`.png`/`.pdf`), `print` accepts `-dsvg`/`-dpng`/`-dpdf`. |
| ~~G25~~ | ~~3-D: `plot3`, `surf`, `mesh`~~ | **Done:** orthographic projection under `view(az,el)` (default −37.5/30; `view(2)`/`view(3)` presets), `plot3` multi-triplets, `surf` painter's-order shaded quads, `mesh` wireframe — both `(Z)` and `(X,Y,Z)` with vector or meshgrid X/Y. Plus `zlim`/`zlabel`, `colormap` (parula/jet/hot/gray), `colorbar`, `caxis`/`clim`, `axis ij`-style `yrev`. Remaining: lighting/surfc/contour3, interactive rotate. |
| G26 | Interactive wgpu figure window | Phase 5 GUI work; pairs with Studio. |

## P5 — toolboxes (separate crates, later)

| # | Gap | Notes |
| --- | --- | --- |
| ~~G27~~ | ~~DSP toolbox~~ | **Done:** `matcore::dsp` — mixed-radix FFT (`fft`/`ifft`/`fft2`/`ifft2`, prime lengths via naive DFT), `fftshift`/`ifftshift`, `dftmtx`, `filter` (DF-IIT, `zi`/`zf`), `xcorr` (maxlag + biased/unbiased/coeff), `freqz` (n-point/`'whole'`/w-vector/`f,fs`), `unwrap`, windows (`hamming`/`hann`/`blackman`/`bartlett`/`rectwin`/`triang`/`kaiser`/`gausswin`). `conv` predates this row. |
| ~~G28~~ | ~~Data I/O~~ | **Done:** `readmatrix`/`writematrix`/`dlmread`/`dlmwrite`/`csvread`/`csvwrite`, `textscan`/`strread`/`fscanf`/`sscanf`, `fopen`/`fread`/`fwrite`/`fseek`/`fgetl`…, `.mat` v5 incl. cell/struct/N-D/complex/sparse. Remaining: `*.xlsx`, netCDF/HDF5 (documented rejects). |
| G29 | Optimization | Landed: `fzero` (safeguarded secant), `fminsearch` (scalar Nelder–Mead), `integral`/`quad` (adaptive Simpson). Remains: `lsqcurvefit`, multi-param `fminsearch`. |
| G30 | Control/systems | `tf`, `step`, `bode`, `lsim` — after complex numbers. |
