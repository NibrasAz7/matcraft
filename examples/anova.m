% ANOVA, post-hoc comparisons and the Friedman rank test.

fprintf('--- one-way ANOVA ---\n');
X1 = [1 4 7; 2 5 8; 3 6 9];
[p1, tbl1, st1] = anova1(X1);
fprintf('p = %.6f, pooled s = %.4f, df = %d\n', p1, st1.s, st1.df);
fprintf('group means: %s\n\n', mat2str(st1.means'));

fprintf('--- pairwise comparisons ---\n');
cb = multcompare(st1, 'CType', 'bonferroni');
fprintf('Bonferroni p-values: %s\n', mat2str(cb(:,6)'));
cl = multcompare(st1, 'CType', 'lsd');
fprintf('LSD p-values: %s\n', mat2str(cl(:,6)'));

fprintf('\n--- two-way ANOVA ---\n');
X2 = [1 2 4; 2 4 5; 3 5 7];
[p2, tbl2, st2] = anova2(X2);
fprintf('columns p = %.6f, error s = %.4f\n', p2, st2.s);
X3 = [1 2 3; 2 3 4; 4 5 6; 5 6 7];
[p3, tbl3] = anova2(X3, 2);
fprintf('with replication: columns p = %.6f, interaction row = %s\n\n', p3, tbl3{4,1});

fprintf('--- Friedman test ---\n');
[pf, tblf, stf] = friedman([1 2 3; 1 2 3; 1 2 3]);
fprintf('p = %.6f, chi2 = %.3f, df = %d\n', pf, stf.chi2, stf.df);
fprintf('mean ranks: %s\n', mat2str(stf.meanranks));
