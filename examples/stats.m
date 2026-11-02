% Statistics toolbox tour: distributions, correlation, regression and k-means.

fprintf('--- normal distribution ---\n');
fprintf('normpdf(0) = %.6f\n', normpdf(0));
fprintf('normcdf(1.96) = %.6f\n', normcdf(1.96));
fprintf('norminv(0.975) = %.6f\n\n', norminv(0.975));

fprintf('--- discrete distributions ---\n');
fprintf('poisspdf(3,2) = %.6f, poisscdf(3,2) = %.6f\n', poisspdf(3, 2), poisscdf(3, 2));
fprintf('binopdf(3,10,0.5) = %.6f, binocdf(3,10,0.5) = %.6f\n\n', binopdf(3, 10, 0.5), binocdf(3, 10, 0.5));

fprintf('--- correlation and regression ---\n');
x = (1:5)';
y = 2*x + 1;
R = corrcoef(x, y);
[b, ~, r, ~, st] = regress(y, [ones(5,1) x]);
fprintf('corr(x,y) = %.6f\n', R(1,2));
fprintf('fit: y = %.3f + %.3f x, R^2 = %.6f\n\n', b(1), b(2), st(1));

fprintf('--- k-means ---\n');
rng(7);
X = [0 0; 0.1 0.2; 5 5; 5.2 4.8; 0 10; 0.2 10.1];
[idx, C, sumd] = kmeans(X, 3, 'Replicates', 5, 'Start', 'plus', 'Distance', 'sqeuclidean');
fprintf('cluster ids: %s\n', mat2str(idx'));
fprintf('within-cluster sums: %s\n', mat2str(sumd'));
fprintf('centers:\n');
disp(C);

fprintf('\n--- hypothesis tests ---\n');
[h1, p1, ci1, st1] = ttest([2 3 4], 0);
fprintf('one-sample: h = %d, p = %.4f, t = %.4f, df = %d\n', h1, p1, st1.tstat, st1.df);
[h2, p2] = ttest2([1 2 3 4 5], [2 3 4 5 6]);
fprintf('two-sample (Welch): h = %d, p = %.4f\n', h2, p2);
[hk, pk, dk] = kstest([-1 0 1]);
fprintf('KS vs standard normal: h = %d, D = %.4f, p = %.4f\n', hk, dk, pk);
[ha, pa, aa] = adtest([-1.2 -0.8 -0.5 -0.2 0.1 0.3 0.6 0.9 1.4 1.8]);
fprintf('Anderson-Darling: h = %d, p = %.4f, A2 = %.4f\n\n', ha, pa, aa);

fprintf('--- PCA ---\n');
Xp = [1 2; 2 4; 3 6];
[co, sc, la, ts, ex, mu] = pca(Xp);
fprintf('explained (%%): %s\n', mat2str(ex'));
fprintf('latent: %s\n', mat2str(la'));
fprintf('first component: %s\n', mat2str(co(:,1)'));
fprintf('mean: %s\n\n', mat2str(mu));

fprintf('--- distributions and fitting ---\n');
fprintf('gampdf(2,3,2) = %.4f, gamcdf(2,3,2) = %.4f\n', gampdf(2, 3, 2), gamcdf(2, 3, 2));
fprintf('betapdf(0.5,2,3) = %.4f, tinv(0.975,10) = %.4f\n', betapdf(0.5, 2, 3), tinv(0.975, 10));
pd = fitdist([1 2 3 4], 'Normal');
fprintf('Normal fit: mu = %.4f, sigma = %.4f\n', pd.mu, pd.sigma);
fprintf('pdf/cdf/icdf at 1/2.5/0.975: %.4f, %.4f, %.4f\n', pdf(pd, 1), cdf(pd, 2.5), icdf(pd, 0.975));
gd = fitdist([0.5 0.8 1.1 1.4 2.0 2.7 3.5 5.0], 'Gamma');
fprintf('Gamma fit: a = %.4f, b = %.4f, mean = %.4f\n', gd.a, gd.b, mean(gd));
rng(11);
samp = random(pd, 1, 5);
fprintf('five seeded draws: %s\n\n', mat2str(samp));

fprintf('--- multiple testing ---\n');
pv = [0.01 0.02 0.03 0.04];
fprintf('Bonferroni: %s\n', mat2str(bonferroni(pv)));
fprintf('Holm: %s\n', mat2str(holm(pv)));
fprintf('BH: %s\n\n', mat2str(benjamini_hochberg(pv)));

fprintf('--- rank tests and PCoA ---\n');
[pw, hw] = signrank([1 2 3 4 10], 0);
[pr, hr] = ranksum([1 2 3 4 5], [2 3 4 5 6]);
[pk, stk] = kruskalwallis([1 4 7; 2 5 8; 3 6 9]);
fprintf('signrank p = %.4f (h=%d), ranksum p = %.4f (h=%d)\n', pw, hw, pr, hr);
fprintf('Kruskal-Wallis p = %.4f, chi2 = %.2f\n', pk, stk.chi2);
D = [0 1 2; 1 0 1; 2 1 0];
[Y, eig] = pcoa(D);
fprintf('PCoA coordinates: %s, eigenvalues: %s\n', mat2str(Y'), mat2str(eig'));

fprintf('\n--- UMAP ---\n');
rng(4);
Xm = [0 0; 0.1 0; 0 0.1; 0.1 0.1; 5 5; 5.1 5; 5 5.1; 5.1 5.1];
emb = umap(Xm, 'NumNeighbors', 3, 'NumEpochs', 300);
c1 = mean(emb(1:4,:));
c2 = mean(emb(5:8,:));
fprintf('embedding size: %s\n', mat2str(size(emb)));
fprintf('centroid 1: %s\n', mat2str(c1));
fprintf('centroid 2: %s\n', mat2str(c2));
