% Chemistry, reaction, GPR and sequence-file tour.

fprintf('--- elemental formulas ---\n');
a = formulaatoms('C6H12O6');
fprintf('C6H12O6: C=%d H=%d O=%d, weight=%.4f\n', a.C, a.H, a.O, formulaweight('C6H12O6'));
fprintf('valid C55H80CoN15O11: %d, valid 6C: %d\n\n', formulacheck('C55H80CoN15O11'), formulacheck('6C'));

fprintf('--- reaction parsing ---\n');
r = parsereaction('2 H2O + ATP <=> ADP + Pi');
disp(r.Reactants);
disp(r.Products);
disp(r.Coefficients);
fprintf('%s\n', formatreaction(r));
fprintf('reverse match: %d\n', rxnequal(r, 'ADP + Pi <=> 2 H2O + ATP', 'Reverse', 1));
bal = rxnbalance('2 H2 + O2 -> 2 H2O', {'H2', 'O2', 'H2O'});
fprintf('H2 combustion balanced: %d\n\n', bal.Balanced);

fprintf('--- GPR rules ---\n');
g = gprparse('g1 & (g2 | g3)');
fprintf('g1,g3 active: %d; only g1 active: %d\n', gpreval(g, {'g1', 'g3'}), gpreval(g, {'g1'}));
disp(gprgenes(g));
fprintf('merged: %s\n\n', gprmerge(g, 'g4'));

fprintf('--- FASTA round trip ---\n');
fa = [tempname '.fasta'];
fid = fopen(fa, 'w');
fprintf(fid, '>seq1 demo\nMKV\n>seq2 demo\nGG\n');
fclose(fid);
[headers, seqs] = fastaread(fa);
disp(headers);
disp(seqs);
fastawrite([fa '.out'], headers, seqs);
fprintf('wrote %s\n', [fa '.out']);
