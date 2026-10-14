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

**Studio + engine + language/numerics sweep landed** — the language core, a
command engine with MCP, a real linear-algebra stack, headless 2D plotting,
and a native Studio app: matrices, logical indexing, control flow, user
`function`s with `varargin`/`varargout`, `eig`/`svd`/`qr`/`chol`/`pinv`,
`plot`/`scatter`/`stem`/`bar`/`area`/`stairs`/`errorbar`/`histogram` with
log axes and SVG export — ~190 builtins via `matcraft-cli`. The web app,
packaging and toolbox crates are ahead. See [`ROADMAP.md`](ROADMAP.md).

## What runs today

- Dense column-major `f64` matrices: `+ - * / \ ' .* ./ .^ ^`, `A(i,j)`,
  linear indexing incl. logical masks (`A(A>0)`), growth on assignment,
  `a:b` / `a:s:b` ranges.
- `if/elseif/else/end`, `for/end`, `while/end`, `switch/case/otherwise`,
  `break`/`continue`/`return`, `;` echo suppression, command syntax
  (`save 'f.mat'`).
- `function y = f(x)` / `function [a,b] = f(x)` — local scopes, hoisted
  script-local functions, `<name>.m` function files (`addpath`/`path`),
  `nargin`/`nargout`, `varargin`/`varargout`, recursion.
- Linear algebra: `det inv norm eig svd qr lu chol rref pinv rank cond
  trace null orth` — pure Rust, no external BLAS.
- Figures: `plot scatter stem bar area stairs errorbar histogram` with
  `'r--o'` specs, `semilogx`/`semilogy`/`loglog`, `hold`/`grid`/labels/
  `legend`/`xlim`/`ylim`, `saveas`/`print` to SVG — headless, no window yet.
- Numerical methods: `fzero fminsearch integral quad interp1 trapz
  polyval polyfit roots conv deconv`.
- Automation: `matcraft-cli --cmd 'id {json}'` runs engine commands;
  `matcraft-cli mcp` serves them over JSON-RPC (stdio).
- **MatCraft Studio** (`matcraft`): command window, workspace inspector,
  editor and figure dock — `matcraft --control 7979` exposes the JSON-lines
  control channel (`engine.execute`, `ui.state`, `ui.render`,
  `ui.screenshot`).
- ~190 builtins total — `matcraft-cli functions` lists the catalogue.

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
