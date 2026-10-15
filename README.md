# MatCraft

A clean-room, Rust-native numerical computing environment — the ArtCraft
ecosystem's answer to MATLAB. Instant launch, memory-safe, zero bloat: a real
`.m`-language engine with a column-major matrix core, built the way the craft
apps are built.

```text
matcraft> A = [2 1; 1 3];
matcraft> b = [5; 5];
matcraft> x = A\b

x =

            2
            1
```

## Status

**Alpha** — Studio + engine + language/numerics + DSP + broad plotting are
in. ~280 builtins, complex and sparse matrices, struct/cell arrays, `.mat`
v5 I/O, 2D and 3D figures with SVG/PNG/PDF export, a MATLAB-style desktop
Studio (ribbon toolstrip, docked panes, command window, workspace, editor,
figure tabs), CLI + MCP + a JSON-lines control channel.

**Compatibility, measured honestly:** in a sweep of 638 real-world `.m`
files from 32 pre-2022 GitHub repos across 8 domains, ~87% parse cleanly —
scripts fail first on missing toolbox builtins, then on `@` handles. See
[`docs/wild-corpus.md`](docs/wild-corpus.md) and [`docs/gaps.md`](docs/gaps.md).
Not yet: `@` anonymous functions/handles, `classdef` OOP, N-D indexing,
image I/O, most toolbox functions, Simulink-style modeling (planned as the
SimCraft sibling). See [`ROADMAP.md`](ROADMAP.md).

## What runs today

- Dense column-major `f64` matrices, `complex`, `sparse`, `cell`, `struct`
  (incl. struct arrays): `+ - * / \ ' .* ./ .^ ^`, `A(i,j)`, logical masks
  (`A(A>0)`), growth on assignment, `a:b`/`a:s:b` ranges.
- `if/elseif/else/end`, `for`/`for(i=…)`, `while`, `switch`, `try/catch`,
  `break`/`continue`/`return`, `;` echo suppression, command syntax
  (`cd ..`, `save 'f.mat'`).
- `function y = f(x)` / `function [a,b] = f(x)` — local scopes, hoisted
  script-local functions, `<name>.m` function files (`addpath`/`genpath`),
  `nargin`/`nargout`, `varargin`/`varargout`, recursion.
- Linear algebra: `det inv norm eig svd qr lu chol rref pinv rank cond
  trace null orth` — pure Rust, no external BLAS; `eig` returns complex
  pairs.
- DSP: `fft ifft fft2 ifft2 fftshift ifftshift dftmtx filter xcorr freqz
  unwrap` + `hamming hann blackman bartlett rectwin triang kaiser
  gausswin` windows.
- Figures: `plot scatter stem bar area stairs errorbar histogram pie
  imagesc contour plot3 surf mesh` with `'r--o'` specs, `semilog*`/`loglog`,
  `meshgrid`, `view`/`zlim`/`zlabel`, `colormap`/`colorbar`/`caxis`, `grid`
  `box` `axis`, `saveas`/`exportgraphics`/`print` to **SVG/PNG/PDF**.
- `.mat` v5 `load`/`save`: numeric, complex, sparse, char.
- Numerical methods: `fzero fminsearch integral quad interp1 trapz
  polyval polyfit roots conv deconv` (complex roots).
- Automation: `matcraft-cli --cmd 'id {json}'` engine commands;
  `matcraft-cli mcp` JSON-RPC; Studio `--control 7979` JSON-lines
  (`engine.execute`, `ui.state`, `ui.render`, `ui.screenshot`).
- **MatCraft Studio** (`matcraft`): HOME/PLOTS/EDITOR/VIEW toolstrip,
  command window (colored errors, history, tab-completion), workspace
  inspector + variable editor, editor with MATLAB-style syntax colors and
  F5/F9, figure dock with pan/zoom, current-folder browser.

## Getting started

```bash
cargo run -p matcraft-cli -- run examples/solve.m   # run a script
cargo run -p matcraft-cli -- eval "mean(1:100)"      # one-shot
cargo run -p matcraft-cli -- repl                    # interactive
```

## Layout

| Path | What |
| --- | --- |
| `crates/matcore` | matrix type + linear algebra (pure Rust; `faer` planned) |
| `crates/matplot` | figure model + headless SVG renderer |
| `crates/matlang` | lexer, parser, tree-walk interpreter, builtins |
| `crates/engine` | command registry + guarded `Session` |
| `crates/mcp` | JSON-RPC 2.0 server over stdio |
| `crates/ui-egui` | Studio UI: command window, workspace, editor, figure dock, control dispatch |
| `apps/matcraft-cli` | `run` / `eval` / `repl` / `functions` / `--cmd` / `mcp` |
| `apps/matcraft` | MatCraft Studio (`--control PORT` for the JSON-lines channel) |
| `docs/` | architecture, language reference |
| `xtask` | `cargo xtask ci` — fmt, clippy, tests |

## Design

Four decoupled layers — GUI/Studio, execution engine, linear algebra, toolboxes
— detailed in [`docs/architecture.md`](docs/architecture.md).

## License

MIT OR Apache-2.0. MatCraft is an original clean-room implementation; no
MathWorks code, assets or documentation text is used.
