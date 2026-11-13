# MatCraft assets

Every non-code file in this repo is original work, OSI open source, public
domain or CC0, and is listed here. No MathWorks icons, images, artwork, fonts,
templates or sample files — ever.

| Path | Author | Source | License | Notes |
| --- | --- | --- | --- | --- |
| `assets/app-icon/matcraft-*.png`, `matcraft.ico` | ArtCraft contributors | generated in code (plotted-curve motif, navy/cyan) | MIT OR Apache-2.0 | Original art; regenerate by drawing, never copied. |
| `crates/matlang/tests/fixtures/h5_v0_gzip.h5` | MatCraft contributors | generated from deterministic 0..23 test vectors with h5py 3.16 / HDF5 2.0 | CC0-1.0 | HDF5 v0, gzip, B-tree v1 chunks |
| `crates/matlang/tests/fixtures/h5_v2_contiguous.h5` | MatCraft contributors | generated from deterministic 0..5 test vectors with h5py 3.16 / HDF5 2.0 | CC0-1.0 | HDF5 v2, contiguous dataset |
| `crates/matlang/tests/fixtures/netcdf4_sample.nc` | MatCraft contributors | generated with netCDF4 1.7.5.1 / netCDF-C 4.10.1 from deterministic 1..12, 10..30 and 0..11 test vectors | CC0-1.0 | NetCDF-4/HDF5 container: 2-D variable, coordinate variable, scalar variable and int16 variable |
| `crates/matlang/tests/fixtures/mat_v5_mcos.mat` | MatCraft contributors | generated from a deterministic MCOS layout by the committed Python fixture recipe (public format notes) | CC0-1.0 | MAT v5 opaque object `obj` (class `MyClass`) with `x=42`, `label='hi'` in an MCOS subsystem |
| `crates/matlang/tests/fixtures/h5_writer_v2.h5` | MatCraft contributors | written by MatCraft's HDF5 writer from deterministic 1..6 and 9..5 vectors; contents verified with h5py 3.16 | CC0-1.0 | HDF5 superblock v2 writer fixture, two datasets |
| `crates/matlang/tests/fixtures/h5_v3_gzip.h5` | MatCraft contributors | generated from deterministic 0..23 test vectors with h5py 3.16 / HDF5 2.0 | CC0-1.0 | HDF5 v3, gzip+shuffle, fixed-array chunks |
| `crates/matlang/tests/fixtures/h5_v3_bt2.h5` | MatCraft contributors | generated from deterministic 0..11 test vectors with h5py 3.16 / HDF5 2.0 | CC0-1.0 | HDF5 v3, gzip+shuffle, B-tree v2 chunks |
| `crates/matlang/tests/fixtures/h5_ea_plain.h5` | MatCraft contributors | generated from deterministic 0..9999 test vectors with h5py 3.16 / HDF5 2.0 | CC0-1.0 | HDF5 v3, one unlimited dimension, extensible-array index |
| `crates/matlang/tests/fixtures/h5_ea_gzip.h5` | MatCraft contributors | generated from deterministic 0..9999 test vectors with h5py 3.16 / HDF5 2.0 | CC0-1.0 | HDF5 v3, gzip+shuffle, extensible-array index |
| `crates/matlang/tests/fixtures/h5_ea_col.h5` | MatCraft contributors | generated from deterministic 0..9999 test vectors with h5py 3.16 / HDF5 2.0 | CC0-1.0 | HDF5 v3, second dimension unlimited, extensible-array index |
| `crates/matlang/tests/fixtures/h5_fa_paged.h5` | MatCraft contributors | generated from deterministic 0..9999 test vectors with h5py 3.16 / HDF5 2.0 | CC0-1.0 | HDF5 v3, 2500 fixed-array entries, paged index |
| `crates/matlang/tests/fixtures/h5_bt2_multi.h5` | MatCraft contributors | generated from deterministic 0..399 test vectors with h5py 3.16 / HDF5 2.0 | CC0-1.0 | HDF5 v3, two unlimited dimensions, B-tree v2 index |
| `crates/matlang/tests/fixtures/h5_vds_src.h5` | MatCraft contributors | generated from deterministic 0..99 test vectors with h5py 3.16 / HDF5 2.0 | CC0-1.0 | Virtual-dataset source |
| `crates/matlang/tests/fixtures/h5_vds.h5` | MatCraft contributors | generated with h5py 3.16 / HDF5 2.0 | CC0-1.0 | Virtual dataset, relative external source, two mappings |
| `crates/matlang/tests/fixtures/h5_vds_sel.h5` | MatCraft contributors | generated with h5py 3.16 / HDF5 2.0 | CC0-1.0 | Virtual dataset, offset hyperslab mapping |
