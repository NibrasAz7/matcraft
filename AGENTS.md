# MatCraft — instructions for agents

MatCraft is a clean-room, open-source, Rust-native numerical computing environment
(M-code compatible) — the MATLAB-class member of the ArtCraft ecosystem. It will
run natively on macOS, Windows and Linux, and on the web via WASM. Siblings with
the same conventions: `../gridcraft` (Excel-class), `../vectorcraft` (Illustrator),
`../photocraft` (Photoshop), `../pdfcraft` (Acrobat).

## Start every session here

1. Read [`ROADMAP.md`](ROADMAP.md) (phase, what's next) and [`docs/gaps.md`](docs/gaps.md)
   if it exists.
2. Read [`docs/architecture.md`](docs/architecture.md) for the layer design and
   [`docs/language.md`](docs/language.md) for the M-code subset currently supported.
3. Behaviour reference: public documentation only (see Clean-room below).

## Non-negotiables

- **Clean-room: never use MathWorks software.** MATLAB's license forbids reverse
  engineering. So:
  - Never launch, run, script, screenshot or measure MATLAB (or Octave copied
    code), and never compare output against a MathWorks installation.
  - Never read or copy anything inside a MATLAB install (binaries, toolboxes,
    built-ins, help files, icons, sample `.m` data files).
  - Behaviour comes only from public documentation and language papers, cited by
    URL, and from files users send us that they own. Function *names* and
    documented semantics are fine; implementations are our own.
  - Never copy GPL/LGPL code (GNU Octave is GPL — **do not read it**), or any
    code whose licence is incompatible with MIT OR Apache-2.0.
- **Assets:** every non-code file is original, OSI open source, public domain or
  CC0, and has a row in [`ASSETS.md`](ASSETS.md).
- **Never crash: no panics in non-test code.** Scripts, files and CLI input are
  untrusted. Fallible work returns `Result`; cap input-derived allocations
  (ranges, `zeros(n)`, reshapes), bound loops (`MAX_LOOP_ITERS`), and index with
  `.get()`/`get()` rather than `[]` on data-derived indices. Workspace lints
  (`clippy::unwrap_used` etc.) enforce this; tests may unwrap.
- **Rust only.** No handwritten JS/TS; never break a future wasm target.
- **Layering** (enforced by `cargo xtask layers` once it exists):
  `matcore` (L0: numbers, no deps) → `matlang` (L1: lexer/parser/interp) →
  `matplot` (L2, planned) → `engine` (L5, planned) → `ui-egui`/`mcp` (L6) →
  `apps/*` (L7). Nothing below L6 may depend on egui/wgpu/winit.
- **Everything is a command** once `engine` lands: user-visible behaviour becomes
  a command id (`workspace.clear`, `eval.line`, …) reachable from CLI, control
  channel and MCP — same pattern as the sibling apps.
- **Quality gates** before every commit: `cargo xtask ci` (fmt, clippy -D
  warnings including no-panic lints, tests).

## Running it

- `cargo run -p matcraft-cli -- run examples/solve.m` — run a script.
- `cargo run -p matcraft-cli -- eval "A = [1 2; 3 4]; det(A)"` — one-shot eval.
- `cargo run -p matcraft-cli -- repl` — interactive session.
- `cargo run -p matcraft-cli -- functions` — builtin library.

## Semantics that bite (learned the hard way)

- `'` is transpose **and** the string quote — the lexer resolves it by whether
  the previous token can end an operand.
- Everything is a matrix: scalars are 1x1. `mat_or_num` keeps `Num` for 1x1 so
  echo formatting stays clean.
- `for v = expr` iterates **columns** of `expr` (a row vector `1:n` gives scalars).
- `f(...)` means indexing when `f` is bound in the workspace, a builtin call
  otherwise — resolved at eval time, not parse time.
- Assignment past the end grows the matrix with zeros; linear indexing is
  column-major.
- A statement-ending `;` suppresses the echo, `,` and newline don't.

## Repo layout

- `crates/matcore` — column-major dense `f64` matrix + LU solve/det/inv (pure
  Rust today; `faer` integration is Phase 5, see ROADMAP).
- `crates/matlang` — lexer, parser, tree-walk interpreter, builtins.
- `apps/matcraft-cli` — `run`/`eval`/`repl`/`functions`.
- `docs/` — architecture, language reference.
- `examples/` — runnable `.m` scripts (keep them dependency-free).
- `xtask` — `cargo xtask ci`.
