% solve.m — linear system, determinant, inverse check.
% Run: cargo run -p matcraft-cli -- run examples/solve.m

A = [2 1; 1 3]
b = [5; 5]
x = A\b          % expect x = [2; 1]

d = det(A)       % expect 5
Ai = inv(A)
check = A * Ai   % back to identity

% Larger system built by indexing.
n = 4;
H = zeros(n, n);
for r = 1:n
  for c = 1:n
    H(r, c) = 1 / (r + c - 1);   % Hilbert-style fill
  end
end
det_H = det(H)
