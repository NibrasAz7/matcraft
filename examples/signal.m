% signal.m — ranges, elementwise math, reductions, conditionals.
% Run: cargo run -p matcraft-cli -- run examples/signal.m

t = 0:0.01:1;                    % 101 samples
f = 5;
y = sin(2 * pi * f * t);         % elementwise

peak = max(y)
energy = sum(y .* y)
rms = sqrt(mean(y .* y))

% Piecewise via while.
count = 0;
while count < 3
  disp('tick')
  count = count + 1;
end

% Threshold with if/elseif/else.
if rms > 0.7
  level = 'hot'
elseif rms > 0.3
  level = 'warm'
else
  level = 'cold'
end
disp(level)
