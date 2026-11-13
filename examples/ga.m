% Genetic algorithm tour: bounded and constrained real-valued optimization.

rng(42);
opts = optimset('PopulationSize', 60, 'MaxGenerations', 200, 'Display', 'off');

fprintf('--- bounded 2-D quadratic ---\n');
[x, f, flag] = ga(@(v) (v(1)-2).^2 + (v(2)+1).^2, 2, ...
                  [], [], [], [], [-5;-5], [5;5], [], opts);
fprintf('x = [%.4f %.4f], f = %.3e, exitflag = %d\n\n', x(1), x(2), f, flag);

fprintf('--- linear constraint x1 + x2 >= 1 ---\n');
[xl, fl] = ga(@(v) v(1).^2 + v(2).^2, 2, [-1 -1], -1, [], [], [0;0], [1;1], [], opts);
fprintf('x = [%.4f %.4f], f = %.4f, sum = %.4f\n\n', xl(1), xl(2), fl, xl(1)+xl(2));

fprintf('--- nonlinear constraint ---\n');
[xn, fn] = ga(@(v) v(1).^2 + v(2).^2, 2, [], [], [], [], [0;0], [1;1], ...
              @(v) [1 - v(1) - v(2)], opts);
fprintf('x = [%.4f %.4f], f = %.4f, sum = %.4f\n\n', xn(1), xn(2), fn, xn(1)+xn(2));

fprintf('--- population output ---\n');
[xp, fp, flagp, out, pop, scores] = ga(@(v) (v(1)-2).^2 + (v(2)+1).^2, 2, ...
                                       [], [], [], [], [-5;-5], [5;5], [], opts);
fprintf('best = [%.4f %.4f], generations = %d, population = %s, scores = %s\n', ...
        xp(1), xp(2), out.generations, mat2str(size(pop)), mat2str(size(scores)));
