% Distances, hierarchical clustering, cluster validation and graphs.

X = [0 0; 1 0; 0 2; 3 3; 3.2 3.1; 0.1 1.9];
fprintf('--- distances ---\n');
disp(pdist(X));
disp(pdist2(X(1:3,:), X(1:3,:)));
fprintf('jaccard([1 0 1 1],[1 1 0 1]) = %.4f\n\n', jaccard([1 0 1 1], [1 1 0 1]));

fprintf('--- clustering ---\n');
Z = linkage(pdist(X), 'average');
idx = cluster(Z, 'maxclust', 2);
fprintf('cluster labels: %s\n', mat2str(idx'));
[sil, silmean] = silhouette(X, idx);
fprintf('mean silhouette: %.4f\n', silmean);
truth = [1; 1; 1; 2; 2; 2];
m = clustermetrics(truth, idx);
fprintf('ARI=%.4f NMI=%.4f purity=%.4f\n\n', m.ARI, m.NMI, m.Purity);

fprintf('--- graphs ---\n');
G = graph([0 1 1 0; 1 0 1 0; 1 1 0 1; 0 0 1 0]);
fprintf('nodes=%d edges=%d\n', numnodes(G), numedges(G));
disp(degree(G)');
[p, d] = shortestpath(G, 1, 4);
fprintf('path %s, distance %d\n', strjoin(p, ' -> '), d);
fprintf('adjacency:\n');
disp(adjacency(G));
