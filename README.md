# MatCraft

**A clean-room, Rust-native numerical computing environment — an open-source
alternative to MATLAB, built the way the craft apps are built.**

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](LICENSE-MIT)
[![Listed on Remade](https://remade.wiki/badges/listed-on-remade.svg)](https://remade.wiki/projects/matcraft/)
[![I did my own MATLAB](https://idmo.app/badge/matcraft)](https://idmo.app/p/matcraft)

Real `.m`-language engine. Column-major matrix core. Instant launch,
memory-safe, zero bloat — desktop Studio, CLI, LSP, MCP, and web, all driving
the same command layer.

![MatCraft Studio — interactive 3-D figure, workspace and command window](docs/images/studio.png)

```text
matcraft> A = [2 1; 1 3];
matcraft> b = [5; 5];
matcraft> x = A\b

x =

            2
            1
```

## Why MatCraft

- **Runs `.m` code for real** — matrices, control flow, functions, `classdef`
  OOP, N-D arrays, complex and sparse, struct/cell, `.mat` v5 files.
- **845 builtins** — linear algebra, DSP, statistics, optimization, control,
  clustering, graphs, bioinformatics, HDF5/NetCDF/xlsx I/O, image ops.
- **Plots** — 2-D + 3-D figures with SVG/PNG/PDF export, and an interactive
  wgpu viewport in the Studio (orbit/zoom/pan on `surf`/`mesh`/`plot3`).
- **Hackable by design** — every user-visible action is a command id
  reachable from the CLI, the TCP control channel, MCP, and (soon) Jupyter.
  An agent can drive a live session end-to-end.
- **Interconnected** — the [SimCraft](https://github.com/NibrasAz7/simcraft)
  sibling brings Simulink-class block-diagram simulation: `sim` runs a model,
  scope traces land in your workspace as figures.
- **Clean-room & safe** — 100% Rust, one `unsafe` boundary (`matffi`), no
  panics on untrusted input, MIT OR Apache-2.0.

## Status

**Alpha.** Studio + engine + language/numerics + DSP + broad plotting are in.

**Compatibility, measured honestly:** in a sweep of 788 real-world `.m` files
from 32 pre-2022 GitHub repos across 8 domains, **632/645 function files (98%)
parse cleanly** — the only parse failures are prose files saved with a `.m`
extension. Scripts fail mostly on repo-local helpers and external toolboxes —
see [`docs/wild-corpus.md`](docs/wild-corpus.md) and
[`docs/gaps.md`](docs/gaps.md); the complete function dictionary is
[`docs/functions.md`](docs/functions.md). See [`ROADMAP.md`](ROADMAP.md) for
what's next (Cranelift JIT, Jupyter transport, wasm Studio hosting).

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
  trace null orth` — pure Rust (optional `faer` backend); `eig` returns
  complex eigenvalues and eigenvectors.
- DSP: `fft ifft fft2 ifft2 fftshift ifftshift dftmtx filter xcorr freqz
  unwrap` + `hamming hann blackman bartlett rectwin triang kaiser
  gausswin` windows.
- FFI: `loadlibrary`/`calllib`/`libpointer` calls plain-C functions in
  DLLs/shared objects.
- Execution: tree-walk interpreter with bytecode-compiled loop bodies;
  Cranelift JIT is the roadmap's next tier.

## Figures

Rendered by MatCraft's own `saveas` — the same SVG→PNG path the Studio
figure dock and `exportgraphics` use:

| `plot` + `text`                     | `surf(peaks)`                     | `imshow(peaks,[])`                    | `bar(magic(4))`                 |
| ----------------------------------- | --------------------------------- | ------------------------------------- | ------------------------------- |
| ![lines](docs/images/fig-lines.png) | ![surf](docs/images/fig-surf.png) | ![imshow](docs/images/fig-imshow.png) | ![bar](docs/images/fig-bar.png) |

## Interfaces — pick your driver

| Interface          | What it gives you                                                                                                      |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------- |
| `matcraft` Studio  | HOME/PLOTS/EDITOR/VIEW ribbon, command window, workspace inspector, editor with signature hints, figure dock, debugger |
| `matcraft-cli`     | `run` / `eval` / `repl` / `functions` / `commands` / `--cmd`                                                           |
| `matcraft-cli lsp` | LSP 3.17 for editors: diagnostics, completion, hover, symbols                                                          |
| `matcraft-cli mcp` | MCP server (JSON-RPC stdio) for agents                                                                                 |
| `--control PORT`   | JSON-lines TCP channel: `engine.execute`, `ui.state`, `ui.render`, `ui.screenshot`                                     |
| `matcraft-web`     | WASM Studio shell                                                                                                      |
| SimCraft bridge    | `sim.run`, `model.set_param`, `sim.check` engine commands                                                              |

## Getting started

```bash
cargo run -p matcraft-cli -- run examples/solve.m   # run a script
cargo run -p matcraft-cli -- eval "mean(1:100)"      # one-shot
cargo run -p matcraft-cli -- repl                    # interactive
cargo run -p matcraft-cli -- lsp                     # language server
cargo run -p matcraft                                # the Studio
```

## Layout

| Path                | What                                                                        |
| ------------------- | --------------------------------------------------------------------------- |
| `crates/matcore`    | matrix type + linear algebra (pure Rust; optional `faer` backend)           |
| `crates/matffi`     | plain-C dynamic-library FFI (the only `unsafe` boundary)                    |
| `crates/matplot`    | figure model + headless SVG renderer                                        |
| `crates/matlang`    | lexer, parser, interpreter (tree-walk + bytecode loop bodies), builtins     |
| `crates/engine`     | command registry + guarded `Session`                                        |
| `crates/mcp`        | JSON-RPC 2.0 server over stdio                                              |
| `crates/lsp`        | LSP 3.17 language server for `.m` files                                     |
| `crates/jupyter`    | Jupyter kernel core (wire + semantics; ZMQ transport pending)               |
| `crates/ui-egui`    | Studio UI: command window, workspace, editor, figure dock, control dispatch |
| `apps/matcraft-cli` | `run` / `eval` / `repl` / `functions` / `--cmd` / `mcp` / `lsp`             |
| `apps/matcraft`     | MatCraft Studio (`--control PORT` for the JSON-lines channel)               |
| `docs/`             | architecture, language reference, function dictionary                       |
| `xtask`             | `cargo xtask ci` — fmt, clippy, tests                                       |

## Design

Four decoupled layers — GUI/Studio, execution engine, linear algebra, toolboxes
— detailed in [`docs/architecture.md`](docs/architecture.md).

## License

MIT OR Apache-2.0. MatCraft is an original clean-room implementation; no
MathWorks code, assets or documentation text is used.
