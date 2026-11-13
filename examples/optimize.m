% Optimization toolbox tour.
%
% Every solver is implemented in Rust in matcore/matlang; this script only
% exercises the public MATLAB-compatible interfaces.

fprintf('--- fminbnd ---\n');
[xb, fb] = fminbnd(@(t) (t - 2).^2 + 1, 0, 5);
fprintf('x = %.6f, f(x) = %.6f\n\n', xb, fb);

fprintf('--- fminsearch (2 parameters) ---\n');
[xs, fs] = fminsearch(@(v) (v(1) - 1).^2 + (v(2) - 3).^2, [0 0]);
fprintf('x = [%.6f %.6f], f(x) = %.3e\n\n', xs(1), xs(2), fs);

fprintf('--- lsqcurvefit ---\n');
t = 0:0.25:2;
y = 2.5 * exp(-1.3 * t);
p = lsqcurvefit(@(p, t) p(1) * exp(-p(2) * t), [1 1], t, y);
fprintf('amplitude = %.6f, decay = %.6f\n\n', p(1), p(2));

fprintf('--- linprog ---\n');
[xl, fl, flagl] = linprog([-1; -1], [1 2; 4 2], [4; 12], [], [], [0; 0], []);
fprintf('x = [%.6f %.6f], f(x) = %.6f, exitflag = %d\n\n', xl(1), xl(2), fl, flagl);

fprintf('--- quadprog ---\n');
[xq, fq, flagq] = quadprog([2 0; 0 2], [-2; -4], [], [], [], [], [0; 0], [1; 1]);
fprintf('x = [%.6f %.6f], f(x) = %.6f, exitflag = %d\n\n', xq(1), xq(2), fq, flagq);

fprintf('--- fmincon with a nonlinear constraint ---\n');
% Minimise x1^2 + x2^2 subject to x1 + x2 >= 1 and x >= 0.
[xc, fc, flagc] = fmincon(@(v) v(1).^2 + v(2).^2, [1; 1], ...
                          [-1 -1], -1, [], [], [0; 0], [], ...
                          @(v) [1 - v(1) - v(2)]);
fprintf('x = [%.6f %.6f], f(x) = %.6f, exitflag = %d\n', xc(1), xc(2), fc, flagc);
