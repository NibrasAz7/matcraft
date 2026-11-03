# MatCraft function dictionary

This dictionary documents the Phase 7 additions first: distances and
clustering, graph objects, chemistry/sequence helpers, and the new plot
kinds. Each row gives the call shape, a one-line description, a runnable
example and the value the interpreter prints. Existing builtins are
indexed by group at the end of the file; their detailed rows will be
filled in as the dictionary grows.

## Distances

| Function | Description | Example | Expected output |
| --- | --- | --- | --- |
| `pdist(X)` | Pairwise distances between the rows of `X`, returned as a row vector. The second argument selects the metric (see the list below). | `pdist([0 0; 3 4])` | `5` |
| `pdist2(X,Y)` | All pairwise distances between rows of `X` and rows of `Y`; the result is `rows(X)`-by-`rows(Y)`. | `pdist2([0 0; 3 4], [0 0])` | `[0; 5]` |
| `squareform(d)` | Converts a condensed distance vector to a square symmetric matrix, and vice versa. | `squareform([1 2 3])` | `[0 1 2; 1 0 3; 2 3 0]` |
| `jaccard(A,B)` | Jaccard similarity. Binary vectors are compared elementwise; cellstr/numeric inputs are treated as sets. | `jaccard([1 0 1 1], [1 1 0 1])` | `0.5` |
| `jaccarddistance(A,B)` | `1 - jaccard(A,B)`. | `jaccarddistance({'a','b','c'}, {'b','c','d'})` | `0.5` |
| `setmetrics(A,B)` | Compares a reference set with a candidate set and returns `TP`, `FP`, `FN`, `Precision`, `Recall`, `F1`, `Jaccard`, `Coverage` and `Novelty`. | `setmetrics({'a','b','c'}, {'b','c','d'})` | `Precision = 0.6667`, `Recall = 0.6667`, `F1 = 0.6667` |
| `confusionmat(true,pred)` | Confusion matrix with one row/column per label. The second output is the sorted label vector. | `[C,ord] = confusionmat([1 1 2 2], [1 2 2 2])` | `C = [1 1; 0 2]`, `ord = {'1','2'}` |
| `pdist` / `pdist2` metrics | `euclidean`, `squaredeuclidean`, `cityblock`/`manhattan`, `chebychev`/`chebyshev`, `minkowski` (pass `p`), `cosine`, `correlation`, `spearman`, `hamming`, `jaccard`, `dice`, `matching`, `canberra`, `braycurtis`, `seuclidean` (`Scale`), `mahalanobis` (`Cov`). | `pdist([1 0; 0 1; 1 1], 'jaccard')` | `[1 0.5 0.5]` |

## Hierarchical clustering

| Function | Description | Example | Expected output |
| --- | --- | --- | --- |
| `linkage(Y,method)` | Agglomerative tree from a condensed distance vector or square distance matrix. Methods: `single`, `complete`, `average`, `weighted`, `centroid`, `median`, `ward`. | `Z = linkage([1 2 3], 'single')` | `[1 2 1; 4 3 2]` |
| `cluster(Z,cutoff)` | Cuts a linkage tree at a height and returns one cluster label per leaf. | `cluster(Z, 1.5)` | `[1; 1; 2]` |
| `cluster(Z,'maxclust',k)` | Cuts the tree so at most `k` clusters remain. | `cluster(Z, 'maxclust', 2)` | `[1; 1; 2]` |
| `clusterdata(X,k)` | One-step distance + linkage + cut; `k` is the target cluster count (a non-integer value is treated as a cutoff). | `clusterdata([0 0; 1 0; 10 0; 11 0], 2)` | `[1; 1; 2; 2]` |
| `cophenet(Z,Y)` | Correlation between the cophenetic distances implied by `Z` and the original condensed distances `Y`. One argument returns the cophenetic distances. | `cophenet(Z, [1 3 2])` | `0.866` |
| `silhouette(X,labels)` | Per-point silhouette scores in `[-1,1]`; the second output is the mean score. | `s = silhouette([0 0; 1 0; 10 0; 11 0], [1;1;2;2])` | `s = [0.9048; 0.8947; 0.8947; 0.9048]` |
| `clustermetrics(true,pred)` | Compares predicted clusters with ground-truth labels and returns `ARI`, `NMI` and `Purity`. | `m = clustermetrics([1 1 2 2], [1 1 2 3])` | `ARI = 0.5714`, `NMI = 0.8165`, `Purity = 0.75` |
| `optimalleaforder(Z,Y)` | Optimal leaf ordering for a linkage tree (kept from the compatibility sweep). | `optimalleaforder(Z, [1 3 2])` | a permutation of `1:n` |

## Graph objects

| Function | Description | Example | Expected output |
| --- | --- | --- | --- |
| `graph(A)` | Undirected graph from a square adjacency matrix. | `G = graph([0 1 1; 1 0 0; 1 0 0])` | `graph` object with `Adjacency`, `NodeNames`, `Directed = 0` |
| `graph(s,t)` / `graph(s,t,w)` | Builds a graph from edge endpoint lists and optional weights. | `G = graph([1 2], [2 3])` | a 3-node path graph |
| `graph(A,names)` / `graph(s,t,w,names)` | Supplies node names (cellstr). `'omitselfloops'` drops diagonal edges. | `G = graph([0 1; 0 0], {'a','b'})` | node names `{'a','b'}` |
| `digraph(...)` | Directed version of the same constructors. | `D = digraph([1 2], [2 3])` | `Directed = 1` |
| `plot(G)` | Draws the graph with a circular layout and node labels. | `plot(G)` | current figure number |
| `gplot(A,XY)` | Draws a graph from an adjacency matrix and an `n`-by-2 coordinate matrix. | `gplot([0 1; 1 0], [0 0; 1 0])` | current figure number |
| `numnodes(G)` / `numedges(G)` | Node and edge counts. | `[numnodes(G) numedges(G)]` | `[3 2]` |
| `degree(G)` | Degree of every node. Directed graphs return in-degree + out-degree; a self-loop counts twice. | `degree(G)` | `[2; 1; 1]` |
| `conncomp(G)` | Weakly connected component labels; second output is the component count. | `[c,k] = conncomp(G)` | `c = [1;1;1]`, `k = 1` |
| `neighbors(G,u)` | Names/IDs of the nodes adjacent to `u`. | `neighbors(G, 1)` | `{'2','3'}` |
| `shortestpath(G,s,t)` | Dijkstra shortest path; first output is a cellstr of node names, second is the distance. | `[p,d] = shortestpath(G, 2, 3)` | `p = {'2','1','3'}`, `d = 2` |
| `distances(G)` | All-pairs shortest-path distance matrix (or one source row when a source is given). | `distances(G)` | `[0 1 1; 1 0 2; 1 2 0]` |
| `bfsearch(G,s)` / `dfsearch(G,s)` | Breadth-first / depth-first node order from `s`. | `bfsearch(G, 1)` | `[1; 2; 3]` |
| `adjacency(G)` | Returns the underlying adjacency matrix. | `adjacency(G)` | the original matrix |
| `isgraph(G)` | True for `graph` and `digraph` objects. | `isgraph(G)` | `1` |

## Chemistry, reactions and GPR rules

| Function | Description | Example | Expected output |
| --- | --- | --- | --- |
| `formulaatoms(f)` | Parses an elemental formula into a struct of element counts. Dot-separated adducts are summed. | `a = formulaatoms('C6H12O6')` | `a.C = 6`, `a.H = 12`, `a.O = 6` |
| `formulaweight(f)` | Molecular weight from standard atomic weights. | `formulaweight('C6H12O6')` | `180.1559` |
| `formulacheck(f)` | Syntax check; returns `1` for a valid formula and `0` otherwise. | `formulacheck('C55H80CoN15O11')` | `1` |
| `inferformula(coeffs,formulas,target)` | Infers the formula of the species at index `target` from reaction coefficients and the other species' formulas. Empty formulas are skipped. | `i = inferformula([-1 -1 1 1 1], {'C10H16N5O13P3','H2O','C10H16N5O10P2','','H+'}, 4)` | `i.Formula = 'HO4P'` |
| `parsereaction(eq)` | Parses an equation into `Reactants`, `Products`, `Species`, `Coefficients`, `Reversible` and `Text`. Arrows: `->`, `=>`, `<=>`, `<->`. | `r = parsereaction('2 H2O + ATP <=> ADP + Pi')` | 2 reactants, 2 products, reversible |
| `formatreaction(r)` | Canonical text form of a reaction struct or string. | `formatreaction(r)` | `2 H2O + ATP <=> ADP + Pi` |
| `canonicalreaction(r)` | Sorts participants and normalises orientation. | `canonicalreaction(r)` | `ADP + Pi <=> ATP + 2 H2O` |
| `rxnequal(a,b,'Reverse',1)` | Compares reactions; `'Reverse'` allows the reverse orientation and `'ProtonMismatch'` ignores proton species. | `rxnequal(r, 'ADP + Pi <=> 2 H2O + ATP', 'Reverse', 1)` | `1` |
| `rxnbalance(r,formulas)` | Checks elemental balance; returns `Balanced` and the per-element `Residual`. | `b = rxnbalance('2 H2 + O2 -> 2 H2O', {'H2','O2','H2O'})` | `b.Balanced = 1`, residuals zero |
| `gprparse(rule)` | Parses a gene-protein-reaction Boolean rule (`&`, `or`, `|`, `and`, parentheses) into a nested struct. | `g = gprparse('g1 & (g2 | g3)')` | `Type = 'and'`, left gene `g1`, right `or` node |
| `gpreval(g,genes)` | Evaluates a GPR rule against active genes. | `gpreval(g, {'g1','g3'})` | `1` |
| `gprgenes(g)` | Cellstr of every gene mentioned in the rule. | `gprgenes(g)` | `{'g1','g2','g3'}` |
| `gprmerge(a,b)` | ORs two rules together and returns the combined text. | `gprmerge(g, 'g4')` | `((g1 & (g2 | g3)) | g4)` |

Ion charges such as `H+`, `OH-` and `Na+` are kept as part of the species
name; a `+` surrounded by whitespace or another species name is still a
separator.

## Sequence files

| Function | Description | Example | Expected output |
| --- | --- | --- | --- |
| `fastaread(file)` | Reads FASTA records. One output returns a struct array with `Header`/`Sequence`; two outputs return header and sequence cellstrs. | `[h,s] = fastaread('demo.fasta')` | `h = {'seq1 first sequence','seq2 second sequence'}`, `s = {'MKV','GG'}` |
| `fastawrite(file,headers,sequences)` | Writes FASTA, wrapping sequence lines at 80 characters. A struct array from `fastaread` is also accepted. | `fastawrite('out.fasta', h, s)` | file written; no value returned |
| `genbankread(file)` | Parses a GenBank flat file into one row per coding feature. Fields include `Locus`, `Definition`, `Feature`, `Location`, `LocusTag`, `OldLocusTag`, `Gene`, `ProteinID`, `Product`, `ECNumber`, `Translation` and the genome `Sequence`. | `g = genbankread('demo.gbk')` | `g(1).LocusTag = 'gene1'`, `g(2).LocusTag = 'gene2'` |

## New plot kinds

| Function | Description | Example | Expected output |
| --- | --- | --- | --- |
| `dendrogram(Z)` | Draws a linkage tree. `'Labels'` sets leaf labels; `'ColorThreshold'` chooses the cut height used to colour branches. | `dendrogram(Z, 'Labels', {'a','b','c'}, 'ColorThreshold', 1.5)` | figure with a coloured tree |
| `venn(A,B,...)` | Two-to-four-set Venn diagram with counts in each region. `'Labels'` names the sets. | `venn({'a','b','c'}, {'b','c','d'}, {'c','e'})` | three overlapping circles with counts |
| `venncounts(A,B,...)` | `2^n`-by-`n+1` table of membership masks and counts for 2-10 sets. | `venncounts({'a','b','c'}, {'b','c','d'}, {'c','e'})` | `8`-by-`4` numeric matrix |
| `quiver(U,V)` / `quiver(X,Y,U,V)` | 2-D vector-field arrows. | `quiver(1:3, 1:3, ones(1,3), zeros(1,3))` | three right-pointing arrows |
| `quiver3(X,Y,Z,U,V,W)` | 3-D vector-field arrows. | `quiver3(1:2,1:2,1:2,ones(1,2),zeros(1,2),ones(1,2))` | two arrows in 3-D |
| `volcano(log2fc,p)` | Volcano plot; points with `p < Alpha` and `abs(log2fc) >= FoldChange` are coloured up/down. Options: `'Alpha'`, `'FoldChange'`. | `volcano([-3 -1 0 1 3], [0.001 0.2 0.5 0.01 0.0001])` | scatter plot with threshold lines |
| `heatmap(C)` | Heatmap of `C` with a colorbar and a `heatmap` title. | `heatmap(magic(5))` | coloured 5-by-5 image |
| `surfl(X,Y,Z)` | `surf` with Lambert lighting enabled. | `[X,Y] = meshgrid(-1:1); surfl(X.^2 + Y.^2)` | lit surface |

All plot functions return the current figure number and obey `hold`;
`saveas(gcf, 'plot.svg')` exports the rendered SVG.

## Complete builtin index

<details>
<summary>All builtins, grouped exactly as the engine catalogue lists them</summary>

### construction

`zeros`, `ones`, `true`, `false`, `eye`, `magic`, `peaks`, `rand`, `randn`, `randi`, `randperm`, `linspace`, `logspace`, `inf`, `Inf`, `nan`, `NaN`, `eps`, `blkdiag`, `vecnorm`, `expm`, `mat2cell`, `gradient`

### size/shape

`size`, `numel`, `length`, `ndims`, `reshape`, `squeeze`, `transpose`, `ctranspose`, `repmat`, `flip`, `permute`, `ipermute`, `shiftdim`, `repelem`, `ndgrid`

### matrix ops

`diag`, `triu`, `tril`, `flipud`, `fliplr`, `kron`, `cat`, `horzcat`, `vertcat`, `circshift`, `rot90`, `meshgrid`

### indexing

`ind2sub`, `sub2ind`, `accumarray`, `bsxfun`

### cells

`cell`, `iscell`, `iscellstr`, `cell2mat`, `num2cell`, `celldisp`, `cellstr`

### structs

`struct`, `isstruct`, `fieldnames`, `isfield`, `rmfield`, `orderfields`, `getfield`, `setfield`, `deal`

### sparse

`sparse`, `full`, `issparse`, `nnz`, `nzmax`, `nonzeros`, `speye`, `spalloc`, `spones`

### linear algebra

`det`, `inv`, `norm`, `dot`, `cross`, `eig`, `svd`, `qr`, `lu`, `chol`, `rref`, `pinv`, `rank`, `cond`, `trace`, `null`, `orth`

### reductions

`sum`, `mean`, `prod`, `min`, `max`, `std`, `var`, `median`, `any`, `all`, `cov`, `prctile`, `quantile`, `wmean`, `nanmax`, `nanmean`, `nanmedian`, `nanmin`, `nanprod`, `nanstd`, `nansum`, `nanvar`

### running/sorting

`cumsum`, `cumprod`, `diff`, `sort`, `sortrows`, `unique`, `find`, `hist`, `histc`

### sets

`union`, `intersect`, `setdiff`, `setxor`, `ismember`

### bits

`bitand`, `bitor`, `bitxor`, `bitcmp`, `bitget`, `bitset`, `bitshift`

### casts

`int8`, `int16`, `int32`, `int64`, `uint8`, `uint16`, `uint32`, `uint64`, `single`, `class`

### elementwise

`abs`, `sqrt`, `exp`, `log`, `log10`, `log2`, `expm1`, `log1p`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `sind`, `cosd`, `tand`, `asind`, `acosd`, `atand`, `atan2d`, `sinh`, `cosh`, `tanh`, `sech`, `csch`, `coth`, `floor`, `ceil`, `round`, `fix`, `sign`, `mod`, `rem`, `hypot`, `nthroot`, `nextpow2`, `pow2`, `deg2rad`, `rad2deg`, `db2pow`, `pow2db`

### types/predicates

`isnan`, `isinf`, `isfinite`, `isempty`, `isscalar`, `isvector`, `isrow`, `iscolumn`, `ismatrix`, `isnumeric`, `ischar`, `islogical`, `isreal`, `isa`, `isequal`, `isequaln`

### complex

`complex`, `real`, `imag`, `conj`, `angle`

### poly/signal

`polyval`, `polyfit`, `roots`, `poly`, `conv`, `deconv`, `interp1`, `trapz`, `histcounts`

### dsp

`fft`, `ifft`, `fft2`, `ifft2`, `fftshift`, `ifftshift`, `dftmtx`, `filter`, `xcorr`, `freqz`, `unwrap`, `hamming`, `hann`, `blackman`, `bartlett`, `rectwin`, `triang`, `kaiser`, `gausswin`, `sinc`, `rectpuls`, `tripuls`, `fir1`

### solvers

`fzero`, `fminbnd`, `fminsearch`, `fminunc`, `fsolve`, `lsqnonlin`, `lsqcurvefit`, `linprog`, `quadprog`, `intlinprog`, `fmincon`, `patternsearch`, `ga`, `integral`, `quad`, `optimset`, `optimoptions`, `optimget`

### control

`tf`, `ss`, `zpk`, `tfdata`, `ssdata`, `zpkdata`, `pole`, `zero`, `dcgain`, `series`, `parallel`, `feedback`, `tf2ss`, `ss2tf`, `step`, `impulse`, `initial`, `lsim`, `bode`, `freqresp`, `pid`, `pidtune`, `margin`, `nyquist`, `isstable`, `damp`, `bandwidth`

### graph objects

`graph`, `digraph`, `gplot`, `numnodes`, `numedges`, `degree`, `conncomp`, `neighbors`, `shortestpath`, `distances`, `bfsearch`, `dfsearch`, `adjacency`, `isgraph`

### distances and clustering

`pdist`, `pdist2`, `squareform`, `jaccard`, `jaccarddistance`, `setmetrics`, `confusionmat`, `clustermetrics`, `cluster`, `clusterdata`, `cophenet`, `silhouette`, `linkage`, `optimalleaforder`

### chemistry and sequences

`formulaatoms`, `formulaweight`, `formulacheck`, `inferformula`, `parsereaction`, `formatreaction`, `canonicalreaction`, `rxnequal`, `rxnbalance`, `gprparse`, `gpreval`, `gprgenes`, `gprmerge`, `fastaread`, `fastawrite`, `genbankread`

### statistics

`normpdf`, `normcdf`, `norminv`, `exppdf`, `expcdf`, `expinv`, `unifpdf`, `unifcdf`, `unifinv`, `poisspdf`, `poisscdf`, `poissinv`, `binopdf`, `binocdf`, `binoinv`, `wblpdf`, `wblcdf`, `wblinv`, `lognpdf`, `logncdf`, `logninv`, `gampdf`, `gamcdf`, `gaminv`, `betapdf`, `betacdf`, `betainv`, `chi2pdf`, `chi2cdf`, `chi2inv`, `tpdf`, `tcdf`, `tinv`, `fpdf`, `fcdf`, `finv`, `exprnd`, `unifrnd`, `poissrnd`, `binornd`, `wblrnd`, `lognrnd`, `fitdist`, `pdf`, `cdf`, `icdf`, `random`, `iqr`, `corrcoef`, `regress`, `kmeans`, `ttest`, `ttest2`, `kstest`, `adtest`, `chi2gof`, `pca`, `cmdscale`, `pcoa`, `bonferroni`, `holm`, `sidak`, `benjamini_hochberg`, `mafdr`, `signrank`, `ranksum`, `kruskalwallis`, `umap`, `tsne`, `corr`, `tabulate`, `gammaln`, `hygecdf`, `nchoosek`, `partialcorr`, `isoutlier`, `isbetween`, `fitlm`

### ode

`ode45`, `ode15s`, `ode23`, `ode23s`, `dde23`, `odeset`, `odeget`, `deval`

### corpus classes

`inputParser`, `addRequired`, `addOptional`, `addParameter`, `addParamValue`, `addparameter`, `parse`, `string`, `strings`, `isstring`, `table`, `table2array`, `isKey`, `keys`, `values`, `remove`, `count`, `parpool`, `maxNumCompThreads`, `gpuArray`, `gather`, `canUseGPU`, `existsOnGPU`, `gpuDevice`, `parfeval`, `fetchOutputs`, `onCleanup`, `categorical`, `reordercats`, `categories`, `countcats`, `addcats`, `removecats`, `mergecats`, `renamecats`, `isundefined`, `isdatetime`, `isduration`, `istimetable`, `datetime`, `duration`, `seconds`

### anova

`anova1`, `anova2`, `multcompare`, `friedman`

### xlsx/table

`xlsread`, `xlswrite`, `readcell`, `writecell`, `readtable`, `writetable`, `height`, `width`, `istable`, `rows`, `array2table`, `ismissing`, `table2cell`, `cell2table`, `struct2table`, `struct2cell`, `cell2struct`, `addvars`, `setvartype`, `convertvars`, `removevars`, `renamevars`, `summary`, `findgroups`, `splitapply`, `varfun`, `rowfun`, `join`, `innerjoin`, `outerjoin`, `groupsummary`, `stack`, `unstack`, `fillmissing`, `rmmissing`, `standardizeMissing`, `fileread`, `isfolder`, `contains`, `startswith`, `endswith`, `startsWith`, `endsWith`, `strsplit`, `strjoin`, `strlength`, `erase`, `replace`, `findstr`, `jsondecode`, `jsonencode`, `importdata`, `detectImportOptions`, `webread`, `urlread`, `web`, `winopen`, `uigetdir`, `questdlg`, `uimenu`, `actxserver`, `extractBefore`, `extractAfter`, `insertBefore`, `isstrprop`, `isletter`, `compose`, `splitlines`, `readlines`, `rethrow`, `verLessThan`, `inputname`, `validateattributes`, `validatestring`, `mustBeNumeric`, `mustBeNonempty`, `mustBePositive`, `mustBeInteger`, `mustBeMember`, `mustBeText`, `mustBeA`, `mustBeScalarOrEmpty`, `mustBeFinite`, `mustBeReal`

### hdf5/netcdf

`h5read`, `h5write`, `h5create`, `h5disp`, `ncread`, `ncwrite`, `ncinfo`, `ncdisp`

### image processing

`rgb2gray`, `imresize`, `imfilter`, `medfilt2`, `edge`, `bwlabel`, `regionprops`, `graythresh`, `imbinarize`, `imdilate`, `imerode`, `imopen`, `imclose`, `imrotate`, `imcrop`, `strel`

### function handles

`feval`, `builtin`, `func2str`, `str2func`, `functions`, `arrayfun`, `cellfun`, `inline`

### objects

`isobject`, `methods`, `properties`, `events`, `metaclass`, `ismethod`, `isprop`, `addlistener`, `notify`, `addprop`, `dynamicprops`, `removeprop`

### debugging

`dbstop`, `dbclear`, `dbcont`, `dbstatus`, `dbstep`, `dbquit`, `run`

### strings

`num2str`, `int2str`, `mat2str`, `str2double`, `str2num`, `str2mat`, `strcmp`, `strcmpi`, `strncmp`, `strncmpi`, `strmatch`, `strcat`, `upper`, `lower`, `strtrim`, `strrep`, `strfind`, `strtok`, `blanks`, `deblank`, `double`, `logical`, `char`, `regexp`, `regexpi`, `regexprep`

### text scan

`fscanf`, `sscanf`, `textscan`, `strread`, `dlmread`, `dlmwrite`, `csvwrite`, `readmatrix`, `writematrix`

### file io

`fopen`, `fclose`, `fread`, `fwrite`, `fseek`, `ftell`, `feof`, `frewind`, `ferror`, `fgetl`, `fgets`, `delete`, `copyfile`, `movefile`, `rmdir`

### dates

`now`, `clock`, `datenum`, `datestr`, `etime`

### system

`version`, `computer`, `ispc`, `isunix`, `ismac`, `getenv`, `setenv`, `system`, `dos`, `unix`, `ver`, `license`, `diary`, `beep`

### graphics objects

`get`, `set`, `drawnow`, `waitbar`, `gca`, `ishandle`, `findobj`, `findall`, `ishold`

### image io

`imread`, `imwrite`, `im2double`, `fspecial`

### workspace/env

`eval`, `evalc`, `evalin`, `assignin`, `error`, `warning`, `lasterror`, `exist`, `which`, `clc`, `home`, `tic`, `toc`, `pause`, `nargin`, `nargout`, `narginchk`, `nargoutchk`, `nargchk`, `disp`, `display`, `format`, `fprintf`, `sprintf`, `input`, `who`, `whos`, `clear`, `assert`, `rng`, `normrnd`, `mfilename`, `help`, `doc`, `global`, `persistent`, `import`, `argv`

### files/path

`save`, `load`, `type`, `addpath`, `rmpath`, `savepath`, `path`, `pwd`, `cd`, `dir`, `ls`, `genpath`, `fullfile`, `fileparts`, `filesep`, `isdir`, `isfile`, `tempdir`, `tempname`, `csvread`, `mkdir`

### plotting

`figure`, `gcf`, `clf`, `close`, `subplot`, `plot`, `scatter`, `stem`, `bar`, `histogram`, `area`, `stairs`, `errorbar`, `contourf`, `contour3`, `semilogx`, `semilogy`, `loglog`, `hold`, `grid`, `title`, `xlabel`, `ylabel`, `zlabel`, `legend`, `xlim`, `ylim`, `zlim`, `xscale`, `yscale`, `view`, `colormap`, `colorbar`, `lighting`, `light`, `lines`, `volcano`, `dendrogram`, `venn`, `venncounts`, `quiver`, `quiver3`, `heatmap`, `validatecolor`, `caxis`, `clim`, `axis`, `box`, `daspect`, `imagesc`, `image`, `imshow`, `text`, `rectangle`, `patch`, `axes`, `boxplot`, `xticks`, `yticks`, `xticklabels`, `yticklabels`, `xtickangle`, `ytickangle`, `sgtitle`, `tiledlayout`, `nexttile`, `yline`, `barh`, `annotation`, `gobjects`, `isgraphics`, `allchild`, `ancestor`, `getappdata`, `setappdata`, `gcp`, `cla`, `colororder`, `getpixelposition`, `savefig`, `hsv`, `parula`, `cool`, `pie`, `contour`, `plot3`, `surf`, `surfl`, `surfc`, `mesh`, `vline`, `xline`, `saveas`, `exportgraphics`, `print`


</details>
