% HDF5 and NetCDF classic I/O tour.

h5 = fullfile(tempdir, 'matcraft_hdf5_demo.h5');
nc = fullfile(tempdir, 'matcraft_netcdf_demo.nc');

fprintf('--- HDF5 ---\n');
h5write(h5, '/temperature', [1 2 3; 4 5 6]);
h5write(h5, '/pressure', [9; 8; 7]);
t = h5read(h5, '/temperature');
p = h5read(h5, '/pressure');
szt = size(t);
szp = size(p);
fprintf('temperature size: %s, pressure size: %s\n', mat2str(szt), mat2str(szp));
h5sub = h5read(h5, '/temperature', [2 2], [1 2], [1 1]);
fprintf('subset: %s\n\n', mat2str(h5sub));

fprintf('--- NetCDF classic ---\n');
ncwrite(nc, 'temp', [1 2 3; 4 5 6]);
nt = ncread(nc, 'temp');
info = ncinfo(nc);
sznt = size(nt);
fprintf('temp size: %s, variable: %s, datatype: %s\n', ...
        mat2str(sznt), info.Variables.Name, info.Variables.Datatype);
ncsub = ncread(nc, 'temp', [2 2], [1 2], [1 1]);
fprintf('subset: %s\n', mat2str(ncsub));
ncwrite(nc, 'temp', [99], [2 3], [1 1], [1 1]);
ncfin = ncread(nc, 'temp');
fprintf('after subset write: %s\n', mat2str(ncfin));

delete(h5);
delete(nc);
