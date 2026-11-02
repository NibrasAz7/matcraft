# MatCraft scope: blocks vs post-1.0 depth

This PR ships a set of **compatibility blocks**. The objective "finish all blocks"
means these blocks, each with implementation, tests and documentation. It does not
claim feature-for-feature parity with every MATLAB toolbox.

## Shipped blocks

| Block | Representative coverage |
| --- | --- |
| Language and classes | matrices/N-D, cells, structs, tables, `classdef`, packages, object arrays, events, `subsref`/`subsasgn`, dynamic properties, `builtin` |
| Optimization | linear/quadratic/nonlinear solvers, `ga`, `intlinprog`, external GLPK/HiGHS/Gurobi/CPLEX command-line backends |
| Control and statistics | `tf`/`ss`/`zpk`, time/frequency responses, distributions, hypothesis tests, PCA/PCoA/UMAP |
| Data I/O | `.mat` v5, XLSX, delimited text, HDF5, NetCDF classic and NetCDF-4/HDF5 containers |
| Graphics and UI | headless SVG/PNG/PDF figures, axes/ticks/layout/handles, MATLAB-style editor colours and parameter hints |
| Platform bridges | COM automation through a safe PowerShell bridge, parallel CPU matmul and optional wgpu acceleration |

## Explicitly out of scope for this PR

These are tracked as post-1.0 depth work, not as incomplete blocks.

| Area | Status | Reason |
| --- | --- | --- |
| MCOS cyclic handle graphs | Depth guarded | Cyclic handle-object graphs exceed the reader depth guard and report an error; acyclic scalar/array/nested graphs are supported. |
| Deployment | macOS/Linux device QA, signing/notarization, public web hosting | Requires external machines, certificates and hosting credentials. |

## Evidence

The block gates are `cargo xtask ci` (format, clippy, workspace tests, layer and
WASM checks) plus `cargo test -p matlang --features gpu`. Current counts and the
PR link are recorded in the pull request.
