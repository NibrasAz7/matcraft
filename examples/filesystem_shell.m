% Filesystem, search-path and shell helpers.

fprintf('pwd: %s\n', pwd);

fprintf('\n--- shell ---\n');
[st, out] = system('echo matcraft shell ok');
fprintf('status = %d, output = %s', st, out);
[sd, od] = dos('echo dos alias');
fprintf('dos status = %d, output = %s', sd, od);

fprintf('\n--- directories ---\n');
d = fullfile(tempdir, 'matcraft_fs_demo');
mkdir(d);
fprintf('created %s: %d\n', d, isfolder(d));
rmdir(d);
fprintf('removed: %d\n', ~isfolder(d));

fprintf('\n--- search path ---\n');
addpath(tempdir, '-end');
p = path;
has = contains(p, tempdir);
fprintf('tempdir on path: %d\n', has);
rmpath(tempdir);

fprintf('\n--- globals ---\n');
global gv;
gv = 7;
fprintf('global gv = %d\n', gv);
