% Corpus class helpers: inputParser, strings and table creation.

fprintf('--- inputParser ---\n');
p = inputParser;
addRequired(p, 'x', @isnumeric);
addOptional(p, 'scale', 1);
addParameter(p, 'Method', 'fast', @ischar);
parse(p, 3, 'method', 'slow');
px = p.Results.x;
ps = p.Results.scale;
pm = p.Results.Method;
fprintf('x = %g, scale = %g, Method = %s\n\n', px, ps, pm);

q = inputParser;
q.KeepUnmatched = true;
addParameter(q, 'Alpha', 0.05);
parse(q, 'Alph', 0.1, 'Extra', 7);
qa = q.Results.Alpha;
qu = numel(q.Unmatched);
fprintf('prefix Alpha = %g, unmatched fields = %d\n\n', qa, qu);

fprintf('--- strings ---\n');
s = string('abc');
e = strings(2, 3);
sl = strlength(s);
si = isstring(s);
sze = size(e);
fprintf('string length = %d, isstring = %d, empty block = %s\n\n', sl, si, mat2str(sze));

fprintf('--- table ---\n');
time = [1; 2; 3];
flux = [4.5; 5.5; 6.5];
T = table(time, flux);
ht = height(T);
wt = width(T);
vars = strjoin(T.Properties.VariableNames, ', ');
arr = table2array(T);
fprintf('height = %d, width = %d\n', ht, wt);
fprintf('variables = %s\n', vars);
fprintf('array = %s\n', mat2str(arr));
