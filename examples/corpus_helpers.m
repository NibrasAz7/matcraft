% Generic string, table and file helpers.

labels = {'alpha', 'beta', 'gamma'};
fprintf('--- string predicates ---\n');
has = contains(labels, 'et');
fprintf('contains(labels, "et"): %s\n', mat2str(has));
starts = startsWith(labels, 'a');
fprintf('startsWith(labels, "a"): %s\n', mat2str(starts));
ends = endsWith(labels, 'a');
fprintf('endsWith(labels, "a"): %s\n', mat2str(ends));

fprintf('\n--- split and join ---\n');
parts = strsplit('carbon,nitrogen,sulfur', ',');
joined = strjoin(parts, ' | ');
fprintf('parts: %s\n', joined);
lens = strlength(parts);
fprintf('lengths: %s\n', mat2str(lens));

fprintf('\n--- tables from matrices ---\n');
T = array2table([1 2; 3 4], 'VariableNames', {'time', 'flux'});
nr = rows(T);
vnames = strjoin(T.Properties.VariableNames, ', ');
tcol = T.time';
fprintf('rows: %d, variables: %s\n', nr, vnames);
fprintf('time column: %s\n', mat2str(tcol));

fprintf('\n--- missing values ---\n');
miss = ismissing([1 NaN 3]);
fprintf('ismissing([1 NaN 3]): %s\n', mat2str(miss));
