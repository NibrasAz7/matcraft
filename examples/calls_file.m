% calls_file.m — calls polyval2.m, a function file in this directory.
% The CLI adds the script's folder to the function path.
% (commas separate entries: `[1 0 -1]` would parse 0-1 as subtraction,
% matching MATLAB's whitespace-minus rule)
y = polyval2([1, 0, -1], 3)   % 1*9 + 0*3 - 1 = 8
v = polyval2([2, 1, 0], 0:4)  % elementwise on a vector
