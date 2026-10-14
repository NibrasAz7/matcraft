% functions.m — user-defined functions: scalar args, multi-output,
% recursion, nargin defaults, and local scope.

% Script-local functions are hoisted: usable before their definition.
x = double_it(21)

[mn, mx] = minmax([3 1 4 1 5 9 2 6])

f5 = fact(5)

% nargin-driven default argument.
g = clamp(12)
g2 = clamp(12, 0, 10)

% --- function definitions live at the end of the script ---

function y = double_it(x)
y = 2 * x;
end

function [a, b] = minmax(v)
a = min(v);
b = max(v);
end

function r = fact(n)
if n <= 1
    r = 1;
    return;
end
r = n * fact(n - 1);
end

function r = clamp(v, lo, hi)
if nargin < 2
    lo = 0;
end
if nargin < 3
    hi = 10;
end
r = min(max(v, lo), hi);
end
