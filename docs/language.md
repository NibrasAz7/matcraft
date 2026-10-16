# MatCraft language reference (current subset)

M-code semantics as implemented today. Anything not listed here is unimplemented
— see `ROADMAP.md`. Behaviour is defined by public language documentation, not
by observing MathWorks software.

## Values

- `double` matrix — everything is a column-major `f64` array of `N` dims
  (`N ≥ 2`); scalars are 1×1. Linear indexing runs column-major through
  every dim: `x(i,j,k)` on a 2×3×4 array is the same element as `x(24)`.
- `char` string — `'like this'` (`''` escapes a quote). `''` is a `0×0`
  empty char; nonempty chars are `1×n`. In numeric contexts a char promotes
  to its code-point doubles: `'a'+1`, `'a'=='a'`, `x'` on a char (a numeric
  column), `x(i)='a'` writing 97 into a double, `[1 'a']`. All-char rows in
  a literal stay char: `['a' 'b']` → `'ab'`, `[72 'i']` → `'Hi'`.
- `cell` array — `{1, 'two', [3 4]}`; any value per element, matrices and
  nested cells included.
- `struct` — struct **array** with named fields; `s.f = v` creates/extends it,
  `s(2).f` indexes elements. Scalars are 1×1 struct arrays.
- `sparse` — sparse double matrix (`sparse`/`speye`/`spalloc`); mixed
  sparse+full arithmetic densifies.
- `complex` — `a+bi` doubles via `i`/`j`; real values promote on contact and
  demote back when every imaginary part is zero.
- Logicals are doubles `0`/`1` (comparisons and `~` produce them).

## Statements & syntax

- Statements end with `;` (suppresses echo), `,` or newline.
- `%` comments to end of line; `...` continues a line.
- `x = expr` assigns; `A(i,j) = v` / `A(i) = v` assigns through indexing and
  grows the target with zeros (1-based, column-major linear indexing).
- `if cond ... elseif cond ... else ... end`
- `for v = expr ... end` — `v` takes each **column** of `expr`.
- `while cond ... end` (hard iteration cap: 1,000,000).
- `switch expr ... case v ... otherwise ... end` — string and scalar matching.
- `try ... catch ... end` / `try ... catch e ... end` — `e.message` and
  `e.identifier` carry the error text and ID.
- `break`, `continue`, `return` — work inside loops and at script level.
- Command syntax: `name arg arg` = `name('arg','arg')` — `format long`,
  `clear x`, `disp hello`, `save f.txt -ascii` all work. Each whitespace-
  separated *word* is one arg, so `f.txt` and `-ascii` are literal strings.

## Expressions

- Matrix literal `[a b; c d]`, empty `[]`.
- Ranges `a:b`, `a:step:b` (row vectors).
- Indexing `A(i)`, `A(i,j)`, `A(i,j,k,…)`, `A(:)`, `A(end)`, `A(i:j)`, and
  mixing: `A(2:end, :)` works. Subscripts beyond the second dim index an
  N-D array's trailing dims; fewer subscripts than `ndims` fold the
  remaining dims into the last one (`x(:,5)` on a 2×3×4 array reads the
  5th element of the merged 3×4 tail). A subscripted read keeps one dim
  per subscript, so `x(:,:,2)` reports `[2 3 1]` and `x(:,2,:)` reports
  `[2 1 4]` — `size` returns the stored dims.
- N-D writes work the same way: `x(i,j,k) = v` broadcasts a scalar RHS,
  matches element counts otherwise, grows any dim past its end with
  zeros, and extends the dims vector — `x(1,1,1) = 5` on a 2-D matrix
  makes it 2×2×1. `x(i) = v` writes flat column-major; a linear index
  past `numel` still only grows vectors.
- Cell literal `{a b; c d}` mirrors matrix literals; braces index **content**
  (`x = c{i}`, `c{i} = v`) while parens index **cells** (`c(i)` is a sub-cell,
  `c(i) = v` writes a cell, `c(i) = []`/`c(:,j)=[]`/`c(i,:)=[]` delete).
  `for x = c` iterates 1x1 cells like M-code; use `x{1}` for the content.
  Cell concatenation in `[…]` works when every element is a cell.
  A multi-element brace pick is a **comma-separated list**: `c{1:3}` expands
  to `v1, v2, v3` in call args (`f(c{:})`, `varargin`), in `[…]`/`{…}`
  literals, on `[a,b] = c{1:2}` LHS, and as index args (`A(c{:})`); `[s.f]`
  over a field works the same way.
- Structs: `s.f` reads a field, `s.f = v` creates/extends the struct
  (undefined `s` becomes a struct; an existing non-struct errors), `s.a.b`
  nests, `s.(expr)` takes the field name from a char expression (even
  `s.(s.k)` — the name resolves before the root detaches), and assignment
  paths interleave fields and subscripts freely (`s.v(i) = x`,
  `s.c{1} = x`, `s.c{1}.f = x`, `s.c{1}(i) = []`).
  `struct('f', v, 'g', w, …)` builds one from name/value pairs; a **cell**
  value expands element-wise into a struct array (`struct('f',{1,2})` → 1×2).
  Struct arrays index like matrices — `s(2).f`, `s(1:2).f` (a comma-separated
  list), `s(i).f = v` (grows), `s(i) = []` (deletes) — and concatenate with
  `[s t]`/`cat` when field sets match; `orderfields` sorts the field order.
- `A(i) = []`, `A(:,j) = []`, `A(i,:) = []` delete elements/columns/rows
  (bounds-checked; a vector keeps its orientation, a matrix linear delete
  collapses to a row). Chained writes `c{1}{1} = v`, `c{1}(i) = v`,
  `s.c{1}.f = v` work to any depth.
- Operators: `+ - * / \ ^ '` and element-wise `.* ./ .^ .'`.
  `A\b` solves `A*x = b`; `A/B` solves `x*B = A`.
- Comparisons `== ~= < <= > >=` are element-wise, returning `0`/`1`.
- Logicals: `&` `|` element-wise; `&&` `||` short-circuit (scalar).
- Unary `-`, `+`, `~` (logical not).
- Operator precedence: `'` `^` > unary > `* / \ .* ./` > `+ -` > comparisons >
  `:` ranges.

## Call vs. indexing

`name(args)` indexes when `name` is bound in the workspace; otherwise it calls
a user function, then a builtin — the dynamic-dispatch rule M-code uses.
An unbound name in expression position is a zero-arg call: `x = rand`,
`nargin` inside a function, bare `who`.

## Functions

- `function y = f(a, b) ... end` and `function [o1, o2] = f(a) ... end`;
  `function f(a)` (no outputs) and `function y = f` (no args) also work.
- `end` is optional at file scope — the next `function` or EOF closes it.
- Bodies run in a **fresh local scope**: parameters bind arguments, all other
  names are local, and nothing leaks to the caller's workspace.
- `[a, b] = f(x)` binds multiple outputs; `y = f(x)` or `f(x)` takes the
  first. `~` ignores a slot: `[a, ~, c] = f(x)`, `function [a, ~] = g(…)`,
  `function r = g(~, b)`.
- `narginchk(min, max)` / `nargoutchk(min, max)` validate call arity.
- `error('msg')` / `error('my:id', 'msg')` throw; `warning` prints.
- Script-local functions are **hoisted** — defined at the end of a script,
  callable from its first line.
- **Function files**: calling `foo()` when `foo` is unbound searches the path
  for `foo.m`, hoists every function in it, then calls. The path is the
  current directory plus the running script's directory; `Session::add_path`
  adds more.
- `nargin`/`nargout` report the argument/output counts of the active call.
- `varargin` collects extra inputs as a 1×n cell; `varargout` is a cell of
  extra outputs (`varargin{k}`, `varargout{k} = …`).
- `return` exits the function early; recursion works naturally.

## Builtin library

| Group | Functions |
| --- | --- |
| Construction | `zeros ones eye magic rand randn randi randperm linspace logspace reshape squeeze transpose repmat diag triu tril flipud fliplr kron cat horzcat vertcat circshift rot90` (shape builtins are cell-aware; `magic(2)` returns the conventional placeholder `[1 3; 4 2]` since no order-2 magic square exists). `zeros`/`ones`/`rand`/`randn`/`Inf`/`NaN` take extra dims (`zeros(2,3,4)` or `zeros([2 3 4])`), `reshape` takes an N-D dims vector or extra args, `squeeze` drops singleton dims, and `cat(d,a,b)` concatenates along any dim (`cat(3,…)` stacks pages; `horzcat`/`vertcat` work on N-D operands too) |
| Shape | `size numel length ndims` — `size(x)` returns the whole dims vector as a row, `size(x,d)` one dim, multi-output `[r,c,…] = size(A)` folds trailing dims into the last output; `ndims` counts stored dims (≥2) |
| Linear algebra | `det inv norm eig svd qr lu chol rref pinv rank cond trace null orth` (multi-output `[V,D] = eig(A)`, `[U,S,V] = svd(A)`, `[Q,R] = qr(A)`, `[L,U,P] = lu(A)`) |
| Reductions | `sum mean prod min max std var median any all` — dim-aware (`f(A)`, `f(A,2)`, `f(A,'all')`, `f(A,[],2)`); N-D arrays reduce along `dim` over the whole array (`sum(ones(2,3,4),3)` → 2×3×1) and default to the first non-singleton dim; two-output `[M,I] = max(A)`; elementwise `min(A,B)` |
| Running | `cumsum cumprod diff sort unique find` (`find` multi-output; `cumsum`/`cumprod`/`diff` are 2-D-only) |
| Element-wise | `abs sqrt exp log log10 log2 expm1 log1p sin cos tan asin acos atan atan2 sinh cosh tanh sech csch coth floor ceil round fix sign mod rem hypot nthroot nextpow2 pow2 dot cross deg2rad rad2deg sinc db2pow pow2db uint8 rectpuls tripuls` |
| Poly / interp | `polyval polyfit roots poly conv deconv interp1 trapz` (`roots` returns complex pairs) |
| DSP | `fft ifft fft2 ifft2 fftshift ifftshift dftmtx filter xcorr freqz unwrap` + windows `hamming hann blackman bartlett rectwin triang kaiser gausswin` (column-wise over matrices; `filter`/`xcorr`/`freqz` have multi-output forms `[y,zf]`, `[r,lags]`, `[h,w]`) |
| Solvers | `fzero fminsearch integral quad` — first arg is a function name string (`fzero('cos', 1)`) |
| Predicates | `isnan isinf isfinite isempty isscalar isvector isrow iscolumn ismatrix isnumeric ischar islogical isa isequal isequaln` |
| Strings | `num2str str2double str2num strcmp strcmpi strcat upper lower strtrim strrep strfind double logical char` |
| Cells | `cell iscell iscellstr cell2mat num2cell celldisp` (`num2cell(A,dim)`, `num2cell` on char) |
| Structs | `struct isstruct fieldnames isfield rmfield getfield setfield orderfields deal` |
| Complex | `i j complex real imag conj abs angle isreal ctranspose` — `'` conjugates, `.'` does not |
| Sparse | `sparse full nnz nzmax nonzeros speye spalloc spones issparse` |
| Constants | `pi eps inf nan true false` — constructor forms too: `Inf(n,m)`, `NaN(n)`, `eps(x)` |
| Workspace | `who whos clear disp display format save load type assert rng normrnd` (`save 'f.mat'` writes tagged ASCII, `load` reads it back or a numeric table; `rng(n)`/`rng('shuffle')`/`rng('default')` seed `rand`/`randn`/`randperm`/`normrnd`) |
| Environment | `addpath rmpath path pwd cd dir ls mkdir genpath fullfile csvread exist which eval run error warning clc home tic toc pause` (`cd ..`/`cd ../x` bare-dot command form; `addpath(genpath(pwd))` splits `;`-lists; `run 'file.m'` executes a script in the current session — shared workspace — and reports failures as `Error in <file> (line N)`) |
| Debugger | `dbstop in <file> [at <line>]` / `dbstop if error`, `dbclear in <file> [at <line>]` / `dbclear all` / `dbclear if error`, `dbstatus [file]`, `dbcont`, `dbstep`, `dbquit` — breakpoints match top-level statements of files run via `run` (a line that isn't a statement start slides to the next one); a stop returns a `LangError` with id `matcraft:debug:pause` and the session exposes `is_paused`/`pause_info`/`resume`/`step`/`quit_debug` for the caller to drive it |
| Output | `fprintf sprintf` (`%d %i %f %e %g %s %c %x %%`, `\n \t \\`, width/precision, cyclic args) |
| Functions | `nargin nargout varargin varargout narginchk nargoutchk` (inside a `function`) |
| Figures | `figure gcf clf close subplot` (`close`, `close n`, `close all`; `subplot(m,n,p)`, `subplot(m,n,[p…])` spans cells, `subplot(mnp)` packed) |
| Plotting | `plot scatter stem bar area stairs errorbar histogram` — `(y)`, `(x,y)`, `(x,y,'spec')`, multi-triplets; matrix `y` → one series per column |
| Axes | `hold on/off`, `grid on/off`, `box on/off`, `title xlabel ylabel zlabel legend`, `xlim/ylim/zlim` (get & set), `axis` (`axis([x0 x1 y0 y1])`/`v = axis`/`axis ij`/`axis xy`/`axis tight`; `equal`/`off`/`on`/… accepted, no visual effect), `xscale/yscale 'log'`/`'linear'`, `view` (`view(2)`/`view(3)`/`view(az,el)`/`view([az el])`) |
| Colour | `colormap parula/jet/hot/gray`, `colorbar`, `caxis`/`clim [lo hi]` — colour-mapped kinds: `imagesc`, `contour`, `surf`, `mesh` |
| Images/contours | `imagesc(C)`/`imagesc(x,y,C)` (y axis reversed, `axis ij`-style), `pie(x[,labels])`, `contour(Z[,n|v])`/`contour(X,Y,Z[,…])` |
| 3-D | `plot3(x,y,z[,spec])` multi-triplets, `surf(Z)`/`surf(X,Y,Z)`, `mesh(Z)`/`mesh(X,Y,Z)` — orthographic projection under `view` |
| Log plots | `semilogx semilogy loglog` — nonpositive points are dropped |
| Grids | `meshgrid` (`[X,Y] = meshgrid(x,y)`), `histcounts` (`[n,e] = histcounts(v,k)`) |
| Export | `saveas`, `exportgraphics`, `print` — `.svg`/`.png`/`.pdf` by extension or `-dsvg`/`-dpng`/`-dpdf` flag (PNG/PDF rasterised via resvg; headless) |

Notes:

- Line specs: colours `b r g c m y k w`, dashes `- -- : -.`, markers
  `o + * . x s d ^ v < >` (e.g. `'r--o'`, `'k:'`, `'b'`). Marker-only specs
  draw markers without a line.
- `plot` without an open figure creates figure 1 automatically; `hold off`
  (default) makes each `plot` replace the figure's series.
- `rand`/`randn` use an internal SplitMix64 PRNG; `randn` is the sum-of-12
  uniform approximation (not yet Box–Muller).
- `disp`/`who`/`whos`/`clear`/`format`/`fprintf` print and produce no `ans`.
- `format long` prints ~14 digits, `format short` (default) ~4.
- `f(n)` dims constructors make `n×n`; `f(r,c)` makes `r×c`; `f()` makes `1×1`;
  `f(d1,d2,d3,…)` or `f([d1 d2 d3])` makes an N-D array.
- N-D arrays display page by page — `x(:,:,1) =` / `x(:,:,2) =` blocks —
  and inherently 2-D operations (`*` `\` `'` `^`, `det`/`inv`/`eig`/…,
  `cumsum`/`diff`, `[v,i] = max`) reject N-D operands with a 2-D-operand
  error rather than reading the first page.
- Matrix literals follow the M-code whitespace rule: `[0 -2 5]` is three
  elements (space before `-`, none after), `[0 - 2]` is one.
- Logical masks index and assign: `v = A(A > 0)`, `A(A < 0) = 0`.
  A mask that is all-ones stays an index list (ambiguous case).

## Known gaps (by design, for now)

- N-D applies to dense real `double` only: cells, structs, complex and
  sparse arrays stay 2-D, and a `()`-subscripted `= []` delete on an N-D
  double is rejected (linear `x(i) = []` still works). Subscripted reads
  return one dim per subscript (`x(:,:,1)` is 2×3×1, not squeezed 2×3 —
  use `squeeze`). `.mat` files round-trip N-D dims for doubles.
- No nested/local-scoped subfunctions. `.mat` v5 binary `save`/`load`
  covers double, complex (FLAG_COMPLEX pr/pi), char and sparse (mxSPARSE
  `ir`/`jc`/`pr`) arrays — saving a cell or struct errors
  (mxCELL/mxSTRUCT writing is deferred). `load` validates the v5
  header (a non-MAT `.mat` errors; so does a file with no numeric data),
  `S = load('f')` returns the file's vars as a struct, and `save` errors
  on undefined variables.
- All allocations driven by input are capped (`zeros(1e9)` errors with
  `allocation exceeds limit` rather than aborting) — matrices ~1e8
  elements, cells ~1e7, figures 1000.
- Cells: chained subscripts work both ways (`c{i}(r,c)` reads and
  `c{i}(r,c) = v` writes, `c{i}{j} = v`/`c{i}.f = v` chains, any depth,
  through struct fields too), and multi-element brace picks expand as
  comma-separated lists. `mat2cell` is not in. `A(0)` on a
  one-element array is still read as an empty logical mask (the all-zeros
  ambiguity) — everywhere else index `0` errors.
- Struct arrays are in (`s(2).f`, `orderfields`, `deal`, growth and
  deletion), but `getfield`/`setfield` on a non-scalar struct take an
  explicit element index rather than expanding as a CSL.
- Complex numbers are in, with auto-demotion: a complex value whose
  imaginary parts are all zero collapses back to real, so
  `isreal(complex(3,0))` is `1` (MATLAB keeps the class). Relational
  ops on complex compare real parts.
- Plotting is headless: SVG/PNG/PDF export but no interactive figure
  window yet; no `contourf`/`contour3`, no `set`/`get` on graphics
  properties, no `camlight`/`lighting`.
- Functions get a fresh scope but not yet `nargin`-style default *outputs*
  (unassigned output args error) or closures/function handles.
- Char arrays are row-only — `char` on a multi-element cellstr needs a
  2-D char matrix, which the `Str` value can't represent yet.
- The debugger stops at top-level statement granularity: `dbstop in f at N`
  inside a `for`/`if`/`function` body can't suspend there (nested
  statements carry no line info yet), and a pause raised inside a
  function call abandons that call frame on `dbcont`.
