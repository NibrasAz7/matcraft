% ODE integration tour: non-stiff, stiff, options and a kinetic system.

fprintf('--- ode45: non-stiff decay ---\n');
opts = odeset('RelTol', 1e-8, 'AbsTol', 1e-10);
[t, y] = ode45(@(t,y) -2*y, [0 2], 1, opts);
fprintf('y(2) = %.8f, exact = %.8f, steps = %d\n\n', y(end), exp(-4), numel(t));

fprintf('--- ode45: harmonic oscillator ---\n');
[t2, Y] = ode45(@(t,y) [y(2); -y(1)], [0 2*pi], [1; 0]);
fprintf('final state: [%.6f %.6f]\n\n', Y(end,1), Y(end,2));

fprintf('--- ode15s: stiff relaxation to cos(t) ---\n');
[t3, y3] = ode15s(@(t,y) -1000*(y-cos(t))-sin(t), [0 1], 1, ...
                  odeset('RelTol', 1e-6, 'AbsTol', 1e-9));
fprintf('y(1) = %.8f, exact = %.8f, steps = %d\n\n', y3(end), cos(1), numel(t3));

fprintf('--- ode15s: Robertson kinetics ---\n');
rob = @(t,y) [-0.04*y(1) + 1e4*y(2)*y(3); ...
              0.04*y(1) - 1e4*y(2)*y(3) - 3e7*y(2)^2; ...
              3e7*y(2)^2];
[t4, Y4] = ode15s(rob, [0 10], [1; 0; 0], odeset('RelTol', 1e-4, 'AbsTol', 1e-6));
fprintf('y(10) = [%.6f %.3e %.6f], steps = %d\n', Y4(end,1), Y4(end,2), Y4(end,3), numel(t4));
