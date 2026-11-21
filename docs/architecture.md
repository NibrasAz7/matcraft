# MatCraft architecture

Four decoupled layers. Lower layers never depend on higher ones; the UI crate
is swappable — the same layering rule the other craft apps enforce with
`cargo xtask layers`.

```text
┌────────────────────────────────────────────────────────┐
│                   MatCraft Studio                      │
│   (Native GUI via egui/wgpu or LSP / Jupyter Kernel)   │
└───────────────────────────┬────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────┐
│              Execution & Language Engine               │
│   - M-Code Parser & AST (hand-rolled, chumsky later)   │
│   - Tree-walk interpreter → Bytecode VM → JIT          │
│   - Scope, Workspace & Dynamic Dispatch Runtime        │
└───────────────────────────┬────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────┐
│                 Linear Algebra Engine                  │
│   - matcore (pure Rust) → faer-rs / BLAS fallback      │
│   - N-Dimensional Slicing & Strided Views              │
│   - Multithreaded Vectorization (Rayon + SIMD)         │
└───────────────────────────┬────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────┐
│                    Toolbox Modules                     │
│   [DSP / FFT]    [Control Systems]    [Plotting / GPU] │
└────────────────────────────────────────────────────────┘
```

## 1. The core numerical & tensor engine — `matcore`

Everything is a multidimensional array of `f64` by default.

- **Matrix representation:** dense, column-major — the layout BLAS/LAPACK and
  M-code linear indexing both assume. Today: 2-D only; N-D strided views are
  the next structural change.
- **Linear algebra backend:**
  - Now: hand-rolled LU solve (partial pivoting), det, inv — zero deps.
  - Primary target: `faer-rs`, pure-Rust and benchmarked at/above OpenBLAS
    on factorizations.
  - Secondary: optional dynamic BLAS/LAPACK linkage for legacy tuning.
- **Vectorized broadcasting:** `broadcast()` covers scalar↔matrix implicit
  expansion; `rayon` + SIMD are Phase 5+.

## 2. The language & execution engine — `matlang`

- **Parser & AST:** hand-rolled recursive descent today (no deps; the `'`/
  string ambiguity needs lexer state either way). Chumsky/nom remain options
  as the grammar grows. Handles:
  - 1-based indexing semantics.
  - Matrix literals `[1, 2; 3, 4]` with `,`/space columns and `;`/newline rows.
  - `end`-relative slicing and `:` full-extent ranges.
  - Dynamic typing and grow-on-assign copy semantics.
- **Execution tiers:**
  1. *Tree-walk interpreter (current).* Direct AST traversal — easiest to
     debug, slow for unvectorized loops.
  2. *Bytecode VM (landed for loop bodies).* `interp/vm.rs` compiles
     `for`/`while` bodies to `Op`/`EOp` sequences once per loop entry and
     runs them on a `Value` stack, sharing the tree-walk's value helpers so
     semantics cannot drift. `Flow` carries break/continue/return. Non-
     compilable statements fall back to `exec_stmt` inside the same block.
  3. *JIT tier.* Cranelift (the Wasmtime code generator) for loop hotspots —
     `EOp` is the lift point.

## 3. GPU-accelerated plotting — `matplot` (Phase 3 landed, headless)

`plot`, `surf`, `scatter` are central to the workflow:

- **Headless first — landed:** `crates/matplot` holds a `Figure` data model
  (series, specs, labels, limits) plus a pure-Rust SVG renderer. `plot`,
  `scatter`, `stem`, `bar` build the model; `saveas`/`exportgraphics`/`print`
  rasterise it to `.svg`. CI and the agent channel can verify figures
  visually with no window — same rule as the sibling apps' `ui.render`.
- **Interactive viewport — landed (3-D):** the Studio dock renders 3-D
  figures (`surf`/`mesh`/`plot3`/`contour3`/`quiver3`) through a wgpu
  pipeline with orbit/zoom/pan bound to `fig.view`; 2-D figures keep the
  SVG→texture path with pan/zoom.
- **Immediate + retained rendering on wgpu:** millions of points, real-time
  streams, 3D surface meshes at 60+ FPS.

## 4. Interactive studio & workspace — `apps/*` (Phase 5)

- **Interactive REPL** (already in `matcraft-cli`): instant launch, multiline
  editing and completion come with the terminal-facing polish later.
- **Workspace inspector:** real-time view of variables, types, dims, memory.
- **Decoupled interfaces:**
  - `--control PORT` JSON-lines channel + `matcraft-cli mcp`, identical in
    spirit to the other craft apps — an agent must be able to drive a live
    session (`engine.execute` for `eval.line`, `workspace.get`, `ui.screenshot`).
  - LSP backend for editors; Jupyter kernel protocol for notebooks.

## 5. Domain toolboxes — separate crates

| Toolbox | Algorithms & ecosystem |
| --- | --- |
| Signal processing | IIR/FIR design, rustfft, Welch periodograms, spectrograms |
| Control systems | transfer functions, state-space, Bode/Nyquist, step response |
| Data & I/O | `.mat` v5/v7.3 (HDF5), CSV/Parquet, binary streams |
| Optimization | Levenberg–Marquardt fitting, simplex, gradient descent |

## Non-goals for now

- Complex numbers (the `Value` enum gets a `Cplx` variant when it happens —
  everything flows through `to_matrix` so the change is contained).
- Sparse matrices, GPU arrays, symbolic math.
- `.mat` v7.3 (HDF5) write support.
