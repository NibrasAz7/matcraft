% plot_demo.m — figure commands: plot, hold, spec strings, labels,
% legend, grid, then a headless SVG export.

t = 0:0.05:2*pi;
figure
plot(t, sin(t), 'b-')
hold on
plot(t, cos(t), 'r--')
plot(t, sin(3*t) .* 0.4, 'g:o')
title('Harmonics')
xlabel('t (rad)')
ylabel('amplitude')
legend('sin t', 'cos t', '0.4 sin 3t')
grid on
saveas('examples/exports/harmonics.svg')

figure
n = 0:19;
stem(n, sin(n * 0.5), 'k')
title('Sampled signal')
xlabel('n')
grid on
saveas('examples/exports/stem.svg')
close all
