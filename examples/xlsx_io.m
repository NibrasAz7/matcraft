% XLSX / table I/O tour. Workbooks are pure-Rust ZIP + OpenXML files.

f = fullfile(tempdir, 'matcraft_xlsx_demo.xlsx');
g = fullfile(tempdir, 'matcraft_xlsx_demo_table.xlsx');

fprintf('--- numeric workbook ---\n');
xlswrite(f, [1 2; 3 4]);
[num, txt, raw] = xlsread(f);
disp(num);
fprintf('txt cells: %d, raw cells: %d\n\n', numel(txt), numel(raw));

fprintf('--- range and text cells ---\n');
xlswrite(f, {'A', 'B'; 10, 20}, 'Sheet1', 'A1');
xlswrite(f, [9 8], 'Data', 'B2');
r = xlsread(f, 'Data', 'B2:C2');
disp(r);
C = readcell(f, 'Sheet1', 'A1:B2');
fprintf('C{2,2} = %g\n\n', C{2,2});

fprintf('--- table round-trip ---\n');
writecell({'name', 'value'; 'alpha', 1; 'beta', 2}, f);
T = readtable(f);
fprintf('height = %d, width = %d\n', height(T), width(T));
fprintf('variables: %s and %s\n', T.Properties.VariableNames{1}, T.Properties.VariableNames{2});
disp(T.value);
writetable(T, g);
T2 = readtable(g);
fprintf('round-trip values: %g %g\n', T2.value(1), T2.value(2));

delete(f);
delete(g);
