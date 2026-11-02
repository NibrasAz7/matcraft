% Control System Toolbox tour: LTI objects, response simulation and
% frequency response. All numerical work happens in Rust.

fprintf('--- transfer function ---\n');
s = tf('s');
G = 1 / (s + 1);
[num, den] = tfdata(G);
fprintf('Num = %s\n', mat2str(num));
fprintf('Den = %s\n', mat2str(den));
fprintf('pole = %g, dcgain = %g\n\n', pole(G), dcgain(G));

fprintf('--- unity feedback ---\n');
T = feedback(G, 1);
[tn, td] = tfdata(T);
fprintf('T(s) = %s / %s\n\n', mat2str(tn), mat2str(td));

fprintf('--- step response ---\n');
t = 0:0.05:3;
[y, ty] = step(G, t);
fprintf('y(1) = %.6f, y(end) = %.6f (exact 1 - e^-3 = %.6f)\n\n', y(1), y(end), 1 - exp(-3));

fprintf('--- state-space round-trip ---\n');
[A, B, C, D] = tf2ss(num, den);
S = ss(A, B, C, D);
[n2, d2] = tfdata(S);
fprintf('A = %s, C = %s\n', mat2str(A), mat2str(C));
fprintf('round-trip: %s / %s\n\n', mat2str(n2), mat2str(d2));

fprintf('--- frequency response ---\n');
w = [0.1 1 10];
[mag, phase] = bode(G, w);
fprintf('w = %s\n', mat2str(w));
fprintf('|G(jw)| = %s\n', mat2str(mag));
fprintf('phase(deg) = %s\n', mat2str(phase));

fprintf('\n--- margins and PID ---\n');
[Gm, Pm, Wcg, Wcp] = margin(2 / (s + 1));
fprintf('Gm = %g, Pm = %g deg, Wcp = %g rad/s\n', Gm, Pm, Wcp);
C = pid(2, 1, 0.5);
[cn, cd] = tfdata(C);
fprintf('PID: %s / %s\n', mat2str(cn), mat2str(cd));
fprintf('stable: %d, bandwidth: %g rad/s\n', isstable(G), bandwidth(G));

fprintf('\n--- MIMO and discrete responses ---\n');
A = [-1 0; 0 -2];
B = eye(2);
C = [1 1; 0 1];
D = zeros(2, 2);
Msys = ss(A, B, C, D);
[Ym, Tm] = step(Msys, 0:0.1:1);
szm = size(Ym);
fprintf('MIMO step size: %s, y(1,1) at t=1: %.4f\n', mat2str(szm), Ym(end,1,1));
Dsys = ss([0.5 0; 0 0.25], [1;1], [1 0.5], 0, 0.1);
[Yd, Td] = step(Dsys, 0:0.1:1);
fprintf('discrete final output: %.4f (samples: %d)\n', Yd(end), numel(Td));

fprintf('\n--- MIMO frequency response ---\n');
Fsys = ss([-1 0; 0 -2], eye(2), eye(2), zeros(2,2));
[magF, phF, wF] = bode(Fsys, [0 1]);
HF = freqresp(Fsys, [0 1]);
szb = size(magF);
szh = size(HF);
h11r = real(HF(1,2));
h11i = imag(HF(1,2));
fprintf('bode array size: %s\n', mat2str(szb));
fprintf('|H11(0)| = %.4f, |H22(j1)| = %.4f, phase H11(j1) = %.2f deg\n', ...
        magF(1,1,1), magF(2,2,2), phF(1,1,2));
fprintf('freqresp size: %s, H11(j1) = %.4f %+.4fi\n', mat2str(szh), h11r, h11i);
