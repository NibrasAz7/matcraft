# MatCraft function dictionary

Every builtin in the engine catalogue is listed here with a usage
signature, a one-line description, a runnable example and the output
the interpreter produces. Examples assign their result to `r`; the
**Output / effect** column shows that value (or the visible effect for
interactive, file and platform functions). Blank lines in matrix
outputs are shown as `<br>`.

The catalogue is generated from `crates/engine/src/cmd.rs`; the
example outputs were captured by running the examples in the
interpreter. Functions that open dialogs, touch the network, write
files or depend on the host platform use a descriptive output.

## Contents

- [construction](#construction) — 23 functions
- [size/shape](#size-shape) — 15 functions
- [matrix ops](#matrix-ops) — 12 functions
- [indexing](#indexing) — 4 functions
- [cells](#cells) — 7 functions
- [structs](#structs) — 9 functions
- [sparse](#sparse) — 9 functions
- [linear algebra](#linear-algebra) — 17 functions
- [reductions](#reductions) — 22 functions
- [running/sorting](#running-sorting) — 9 functions
- [sets](#sets) — 5 functions
- [bits](#bits) — 7 functions
- [casts](#casts) — 10 functions
- [elementwise](#elementwise) — 43 functions
- [types/predicates](#types-predicates) — 16 functions
- [complex](#complex) — 5 functions
- [poly/signal](#poly-signal) — 9 functions
- [dsp](#dsp) — 23 functions
- [solvers](#solvers) — 18 functions
- [control](#control) — 27 functions
- [graph objects](#graph-objects) — 15 functions
- [distances and clustering](#distances-and-clustering) — 17 functions
- [chemistry and sequences](#chemistry-and-sequences) — 16 functions
- [statistics](#statistics) — 79 functions
- [ode](#ode) — 8 functions
- [corpus classes](#corpus-classes) — 42 functions
- [anova](#anova) — 4 functions
- [xlsx/table](#xlsx-table) — 84 functions
- [hdf5/netcdf](#hdf5-netcdf) — 8 functions
- [image processing](#image-processing) — 16 functions
- [function handles](#function-handles) — 8 functions
- [objects](#objects) — 12 functions
- [debugging](#debugging) — 7 functions
- [strings](#strings) — 26 functions
- [text scan](#text-scan) — 9 functions
- [file io](#file-io) — 15 functions
- [dates](#dates) — 5 functions
- [system](#system) — 14 functions
- [graphics objects](#graphics-objects) — 9 functions
- [image io](#image-io) — 4 functions
- [workspace/env](#workspace-env) — 38 functions
- [files/path](#files-path) — 21 functions
- [plotting](#plotting) — 98 functions

## construction

<a id="construction"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`zeros`** <code>zeros(n) &#124; zeros(m,n,...)</code> | Array of zeros. | <code>r = zeros(2,3);</code> | <code>[0 0 0;0 0 0]</code> |
| **`ones`** <code>ones(n) &#124; ones(m,n,...)</code> | Array of ones. | <code>r = ones(2,2);</code> | <code>[1 1;1 1]</code> |
| **`true`** <code>true(n) &#124; true(m,n)</code> | Logical true array. | <code>r = true(2);</code> | <code>[1 1;1 1]</code> |
| **`false`** <code>false(n) &#124; false(m,n)</code> | Logical false array. | <code>r = false(2);</code> | <code>[0 0;0 0]</code> |
| **`eye`** <code>eye(n) &#124; eye(m,n)</code> | Identity matrix. | <code>r = eye(2);</code> | <code>[1 0;0 1]</code> |
| **`magic`** <code>magic(n)</code> | Magic square. | <code>r = magic(3);</code> | <code>[8 1 6;3 5 7;4 9 2]</code> |
| **`peaks`** <code>peaks(n) &#124; [X,Y,Z] = peaks(n)</code> | Classic peaks demo surface. | <code>r = size(peaks(3));</code> | <code>[3 3]</code> |
| **`rand`** <code>rand(n) &#124; rand(m,n)</code> | Uniform random numbers in [0,1]. | <code>rng(1); r = rand(1,3);</code> | <code>[0.5666 0.7458 0.971]</code> |
| **`randn`** <code>randn(n) &#124; randn(m,n)</code> | Standard normal random numbers. | <code>rng(1); r = randn(1,3);</code> | <code>[1.4243 -0.6189 -0.5908]</code> |
| **`randi`** <code>randi(imax,sz)</code> | Uniform random integers from 1 to imax. | <code>rng(1); r = randi(5,1,4);</code> | <code>[3 4 5 3]</code> |
| **`randperm`** <code>randperm(n)</code> | Random permutation of 1:n. | <code>rng(1); r = sort(randperm(4));</code> | <code>[1 2 3 4]</code> |
| **`linspace`** <code>linspace(a,b,n)</code> | n evenly spaced points from a to b. | <code>r = linspace(0,1,5);</code> | <code>[0 0.25 0.5 0.75 1]</code> |
| **`logspace`** <code>logspace(a,b,n)</code> | n logarithmically spaced points from 10^a to 10^b. | <code>r = logspace(0,2,3);</code> | <code>[1 10 100]</code> |
| **`inf`** <code>inf &#124; Inf(n,m)</code> | Infinity constant or array. | <code>r = Inf;</code> | <code>[Inf]</code> |
| **`Inf`** <code>Inf &#124; Inf(n,m)</code> | Infinity constant or array. | <code>r = Inf(2,1);</code> | <code>[Inf;Inf]</code> |
| **`nan`** <code>nan &#124; NaN(n,m)</code> | Not-a-number constant or array. | <code>r = NaN;</code> | <code>[NaN]</code> |
| **`NaN`** <code>NaN &#124; NaN(n,m)</code> | Not-a-number constant or array. | <code>r = NaN(1,2);</code> | <code>[NaN NaN]</code> |
| **`eps`** <code>eps &#124; eps(x)</code> | Floating-point spacing. | <code>r = eps(1);</code> | <code>[2.2204e-16]</code> |
| **`blkdiag`** <code>blkdiag(A,B,...)</code> | Block diagonal matrix. | <code>r = blkdiag([1 2;3 4],5);</code> | <code>[1 2 0;3 4 0;0 0 5]</code> |
| **`vecnorm`** <code>vecnorm(A[,p,dim])</code> | Vector-wise p-norm. | <code>r = vecnorm([3 4;6 8]);</code> | <code>[6.7082 8.9443]</code> |
| **`expm`** <code>expm(A)</code> | Matrix exponential. | <code>r = expm([0 1;0 0]);</code> | <code>[1 1;0 1]</code> |
| **`mat2cell`** <code>mat2cell(A,rows,cols)</code> | Split a matrix into a cell array of blocks. | <code>r = size(mat2cell(ones(2,3),[1 1],[1 2]));</code> | <code>[2 2]</code> |
| **`gradient`** <code>gradient(F[,h])</code> | Numerical gradient. | <code>r = gradient([1 4 9]);</code> | <code>[3 4 5]</code> |

## size/shape

<a id="size-shape"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`size`** <code>size(A[,dim]) &#124; [m,n] = size(A)</code> | Array dimensions. | <code>r = size([1 2;3 4]);</code> | <code>[2 2]</code> |
| **`numel`** <code>numel(A)</code> | Number of elements. | <code>r = numel([1 2;3 4]);</code> | <code>[4]</code> |
| **`length`** <code>length(A)</code> | Length of the longest dimension. | <code>r = length([1 2 3]);</code> | <code>[3]</code> |
| **`ndims`** <code>ndims(A)</code> | Number of array dimensions. | <code>r = ndims(ones(2,3,4));</code> | <code>[3]</code> |
| **`reshape`** <code>reshape(A,m,n,...)</code> | Reshape an array without changing its data order. | <code>r = reshape(1:6,2,3);</code> | <code>[1 3 5;2 4 6]</code> |
| **`squeeze`** <code>squeeze(A)</code> | Remove singleton dimensions. | <code>r = size(squeeze(ones(1,3,1)));</code> | <code>[3 1]</code> |
| **`transpose`** <code>transpose(A) &#124; A.'</code> | Non-conjugating transpose. | <code>r = transpose([1 2;3 4]);</code> | <code>[1 3;2 4]</code> |
| **`ctranspose`** <code>ctranspose(A) &#124; A'</code> | Conjugating transpose. | <code>r = ctranspose([1+2i 3]);</code> | <code>1 - 2i<br>                 3 - 0i</code> |
| **`repmat`** <code>repmat(A,m,n)</code> | Replicate an array. | <code>r = repmat([1 2],2,1);</code> | <code>[1 2;1 2]</code> |
| **`flip`** <code>flip(A[,dim])</code> | Reverse an array along a dimension. | <code>r = flip([1 2 3]);</code> | <code>[3 2 1]</code> |
| **`permute`** <code>permute(A,order)</code> | Reorder array dimensions. | <code>r = size(permute(ones(2,3,4),[2 1 3]));</code> | <code>[3 2 4]</code> |
| **`ipermute`** <code>ipermute(A,order)</code> | Inverse dimension permutation. | <code>r = size(ipermute(ones(3,2,4),[2 1 3]));</code> | <code>[2 3 4]</code> |
| **`shiftdim`** <code>shiftdim(A,n)</code> | Shift array dimensions. | <code>r = size(shiftdim(ones(2,3,4),1));</code> | <code>[3 4 2]</code> |
| **`repelem`** <code>repelem(A,n)</code> | Repeat each element n times. | <code>r = repelem([1 2],2);</code> | <code>[1 1 2 2]</code> |
| **`ndgrid`** <code>ndgrid(x,y,...)</code> | N-dimensional grid arrays. | <code>r = size(ndgrid(1:2,1:3));</code> | <code>[2 3]</code> |

## matrix ops

<a id="matrix-ops"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`diag`** <code>diag(v) &#124; diag(A[,k])</code> | Diagonal matrix or diagonal extraction. | <code>r = diag([1 2 3]);</code> | <code>[1 0 0;0 2 0;0 0 3]</code> |
| **`triu`** <code>triu(A[,k])</code> | Upper triangular part. | <code>r = triu(ones(2,3));</code> | <code>[1 1 1;0 1 1]</code> |
| **`tril`** <code>tril(A[,k])</code> | Lower triangular part. | <code>r = tril(ones(2,3));</code> | <code>[1 0 0;1 1 0]</code> |
| **`flipud`** <code>flipud(A)</code> | Flip a matrix up-down. | <code>r = flipud([1 2;3 4]);</code> | <code>[3 4;1 2]</code> |
| **`fliplr`** <code>fliplr(A)</code> | Flip a matrix left-right. | <code>r = fliplr([1 2;3 4]);</code> | <code>[2 1;4 3]</code> |
| **`kron`** <code>kron(A,B)</code> | Kronecker product. | <code>r = kron([1 2],[1;2]);</code> | <code>[1 2;2 4]</code> |
| **`cat`** <code>cat(dim,A,B,...)</code> | Concatenate arrays along a dimension. | <code>r = size(cat(3,ones(2),zeros(2)));</code> | <code>[2 2 2]</code> |
| **`horzcat`** <code>horzcat(A,B,...) &#124; [A B]</code> | Horizontal concatenation. | <code>r = horzcat([1 2],[3 4]);</code> | <code>[1 2 3 4]</code> |
| **`vertcat`** <code>vertcat(A,B,...) &#124; [A; B]</code> | Vertical concatenation. | <code>r = vertcat([1 2],[3 4]);</code> | <code>[1 2;3 4]</code> |
| **`circshift`** <code>circshift(A,k)</code> | Circular shift. | <code>r = circshift([1 2 3],1);</code> | <code>[3 1 2]</code> |
| **`rot90`** <code>rot90(A[,k])</code> | Rotate a matrix by 90 degrees. | <code>r = rot90([1 2;3 4]);</code> | <code>[2 4;1 3]</code> |
| **`meshgrid`** <code>meshgrid(x,y)</code> | 2-D grid arrays. | <code>r = size(meshgrid(1:2,1:3));</code> | <code>[3 2]</code> |

## indexing

<a id="indexing"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`ind2sub`** <code>ind2sub(sz,ind)</code> | Linear indices to subscripts. | <code>[i,j] = ind2sub([2 3],4); r = [i j];</code> | <code>[2 2]</code> |
| **`sub2ind`** <code>sub2ind(sz,i,j)</code> | Subscripts to linear indices. | <code>r = sub2ind([2 3],2,2);</code> | <code>[4]</code> |
| **`accumarray`** <code>accumarray(subs,vals)</code> | Accumulate values into bins. | <code>r = accumarray([1;2;2],[3;4;5]);</code> | <code>[3;9]</code> |
| **`bsxfun`** <code>bsxfun(fun,A,B)</code> | Apply an elementwise operation with singleton expansion. | <code>r = bsxfun(@plus,[1;2],[10 20]);</code> | <code>[11 21;12 22]</code> |

## cells

<a id="cells"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`cell`** <code>cell(m,n)</code> | Empty cell array. | <code>r = size(cell(2,3));</code> | <code>[2 3]</code> |
| **`iscell`** <code>iscell(A)</code> | True for cell arrays. | <code>r = iscell({1,2});</code> | <code>[1]</code> |
| **`iscellstr`** <code>iscellstr(A)</code> | True for cell arrays of strings. | <code>r = iscellstr({'a','b'});</code> | <code>[1]</code> |
| **`cell2mat`** <code>cell2mat(C)</code> | Concatenate cell contents into a matrix. | <code>r = cell2mat({1,2;3,4});</code> | <code>[1 2;3 4]</code> |
| **`num2cell`** <code>num2cell(A[,dim])</code> | Convert a numeric array to a cell array. | <code>r = num2cell([1 2;3 4]);</code> | <code>[1]                 [2]                 <br>   [3]                 [4]</code> |
| **`celldisp`** <code>celldisp(C)</code> | Display cell contents recursively. | <code>celldisp({1,2})</code> | <code>c = {1,2}; celldisp(c) prints the two entries.</code> |
| **`cellstr`** <code>cellstr(S)</code> | Convert a character array to a cellstr. | <code>r = cellstr('abc');</code> | <code>'abc'</code> |

## structs

<a id="structs"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`struct`** <code>struct('field',value,...)</code> | Create a struct. | <code>s = struct('a',1,'b',2); r = s.a;</code> | <code>[1]</code> |
| **`isstruct`** <code>isstruct(A)</code> | True for structs. | <code>r = isstruct(struct('a',1));</code> | <code>[1]</code> |
| **`fieldnames`** <code>fieldnames(S)</code> | Field names of a struct. | <code>r = fieldnames(struct('b',2,'a',1));</code> | <code>'b'                 <br>   'a'</code> |
| **`isfield`** <code>isfield(S,name)</code> | True when a struct has a field. | <code>r = isfield(struct('a',1),'a');</code> | <code>[1]</code> |
| **`rmfield`** <code>rmfield(S,name)</code> | Remove a struct field. | <code>s = rmfield(struct('a',1,'b',2),'b'); r = fieldnames(s);</code> | <code>'a'</code> |
| **`orderfields`** <code>orderfields(S)</code> | Sort struct fields. | <code>r = fieldnames(orderfields(struct('b',2,'a',1)));</code> | <code>'a'                 <br>   'b'</code> |
| **`getfield`** <code>getfield(S,name)</code> | Read a struct field. | <code>r = getfield(struct('a',5),'a');</code> | <code>[5]</code> |
| **`setfield`** <code>setfield(S,name,value)</code> | Write a struct field. | <code>s = setfield(struct('a',1),'a',7); r = s.a;</code> | <code>[7]</code> |
| **`deal`** <code>deal(v,...)</code> | Copy inputs to outputs. | <code>[a,b] = deal(1,2); r = [a b];</code> | <code>[1 2]</code> |

## sparse

<a id="sparse"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`sparse`** <code>sparse(i,j,v[,m,n])</code> | Sparse matrix from triplets. | <code>S = sparse([1 2],[1 2],[3 4]); r = full(S);</code> | <code>[3 0;0 4]</code> |
| **`full`** <code>full(S)</code> | Convert a sparse matrix to dense. | <code>S = sparse([1 2],[1 2],[3 4]); r = full(S);</code> | <code>[3 0;0 4]</code> |
| **`issparse`** <code>issparse(A)</code> | True for sparse matrices. | <code>r = issparse(sparse(1,1,1));</code> | <code>[1]</code> |
| **`nnz`** <code>nnz(S)</code> | Number of stored nonzeros. | <code>r = nnz(sparse([1 2],[1 2],[3 4]));</code> | <code>[2]</code> |
| **`nzmax`** <code>nzmax(S)</code> | Storage allocated for nonzeros. | <code>r = nzmax(sparse([1 2],[1 2],[3 4]));</code> | <code>[2]</code> |
| **`nonzeros`** <code>nonzeros(S)</code> | Stored nonzero values. | <code>r = nonzeros(sparse([1 2],[1 2],[3 4]));</code> | <code>[3;4]</code> |
| **`speye`** <code>speye(n)</code> | Sparse identity matrix. | <code>r = full(speye(2));</code> | <code>[1 0;0 1]</code> |
| **`spalloc`** <code>spalloc(m,n,nz)</code> | Allocate an empty sparse matrix. | <code>r = size(spalloc(3,3,5));</code> | <code>[3 3]</code> |
| **`spones`** <code>spones(S)</code> | Replace nonzeros with ones. | <code>r = full(spones(sparse([1 2],[1 2],[3 4])));</code> | <code>[1 0;0 1]</code> |

## linear algebra

<a id="linear-algebra"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`det`** <code>det(A)</code> | Determinant. | <code>r = det([2 1;1 3]);</code> | <code>[5]</code> |
| **`inv`** <code>inv(A)</code> | Matrix inverse. | <code>r = inv([2 1;1 3]);</code> | <code>[0.6 -0.2;-0.2 0.4]</code> |
| **`norm`** <code>norm(A[,p])</code> | Matrix or vector norm. | <code>r = norm([3 4]);</code> | <code>[5]</code> |
| **`dot`** <code>dot(a,b)</code> | Vector dot product. | <code>r = dot([1 2],[3 4]);</code> | <code>[11]</code> |
| **`cross`** <code>cross(a,b)</code> | Vector cross product. | <code>r = cross([1 0 0],[0 1 0]);</code> | <code>[0 0 1]</code> |
| **`eig`** <code>eig(A) &#124; [V,D] = eig(A)</code> | Eigenvalues or eigenvectors and eigenvalues. | <code>r = sort(eig([2 0;0 3]));</code> | <code>[2;3]</code> |
| **`svd`** <code>svd(A) &#124; [U,S,V] = svd(A)</code> | Singular values or singular value decomposition. | <code>r = svd([1 0;0 2]);</code> | <code>[2;1]</code> |
| **`qr`** <code>qr(A) &#124; [Q,R] = qr(A)</code> | Orthogonal-triangular decomposition. | <code>[Q,R] = qr([1 2;3 4]); r = size(Q);</code> | <code>[2 2]</code> |
| **`lu`** <code>lu(A) &#124; [L,U,P] = lu(A)</code> | LU factorization. | <code>[L,U,P] = lu([2 1;1 3]); r = size(L);</code> | <code>[2 2]</code> |
| **`chol`** <code>chol(A)</code> | Cholesky factorization. | <code>r = chol([4 2;2 3]);</code> | <code>[2 1;0 1.4142]</code> |
| **`rref`** <code>rref(A)</code> | Reduced row echelon form. | <code>r = rref([1 2;2 4]);</code> | <code>[1 2;0 0]</code> |
| **`pinv`** <code>pinv(A)</code> | Moore-Penrose pseudoinverse. | <code>r = pinv([1 2;3 4]);</code> | <code>[-2 1;1.5 -0.5]</code> |
| **`rank`** <code>rank(A)</code> | Matrix rank. | <code>r = rank([1 2;2 4]);</code> | <code>[1]</code> |
| **`cond`** <code>cond(A)</code> | Matrix condition number. | <code>r = cond(eye(2));</code> | <code>[1]</code> |
| **`trace`** <code>trace(A)</code> | Sum of diagonal elements. | <code>r = trace([1 2;3 4]);</code> | <code>[5]</code> |
| **`null`** <code>null(A)</code> | Orthonormal basis for the null space. | <code>r = size(null([1 1]));</code> | <code>[2 0]</code> |
| **`orth`** <code>orth(A)</code> | Orthonormal basis for the range. | <code>r = size(orth([1 0;0 1]));</code> | <code>[2 2]</code> |

## reductions

<a id="reductions"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`sum`** <code>sum(A[,dim])</code> | Sum of elements. | <code>r = sum([1 2;3 4]);</code> | <code>[4 6]</code> |
| **`mean`** <code>mean(A[,dim])</code> | Arithmetic mean. | <code>r = mean([1 2;3 4]);</code> | <code>[2 3]</code> |
| **`prod`** <code>prod(A[,dim])</code> | Product of elements. | <code>r = prod([1 2;3 4]);</code> | <code>[3 8]</code> |
| **`min`** <code>min(A) &#124; [m,i] = min(A)</code> | Minimum value and index. | <code>r = min([3 1 2]);</code> | <code>[1]</code> |
| **`max`** <code>max(A) &#124; [m,i] = max(A)</code> | Maximum value and index. | <code>r = max([3 1 2]);</code> | <code>[3]</code> |
| **`std`** <code>std(A[,flag,dim])</code> | Standard deviation. | <code>r = std([1 2 3]);</code> | <code>[1]</code> |
| **`var`** <code>var(A[,flag,dim])</code> | Variance. | <code>r = var([1 2 3]);</code> | <code>[1]</code> |
| **`median`** <code>median(A[,dim])</code> | Median value. | <code>r = median([1 2 3 4]);</code> | <code>[2.5]</code> |
| **`any`** <code>any(A[,dim])</code> | True when any element is nonzero. | <code>r = any([0 0 1]);</code> | <code>[1]</code> |
| **`all`** <code>all(A[,dim])</code> | True when all elements are nonzero. | <code>r = all([1 1 0]);</code> | <code>[0]</code> |
| **`cov`** <code>cov(X) &#124; cov(x,y)</code> | Covariance matrix. | <code>r = cov([1 2;3 4]);</code> | <code>[2 2;2 2]</code> |
| **`prctile`** <code>prctile(X,p)</code> | Percentile. | <code>r = prctile([1 2 3 4],50);</code> | <code>[2.5]</code> |
| **`quantile`** <code>quantile(X,q)</code> | Quantile. | <code>r = quantile([1 2 3 4],0.5);</code> | <code>[2.5]</code> |
| **`wmean`** <code>wmean(A,W)</code> | Weighted mean. | <code>r = wmean([1 2],[1 1]);</code> | <code>[1.5]</code> |
| **`nanmax`** <code>nanmax(A)</code> | Maximum ignoring NaNs. | <code>r = nanmax([1 NaN 3]);</code> | <code>[3]</code> |
| **`nanmean`** <code>nanmean(A)</code> | Mean ignoring NaNs. | <code>r = nanmean([1 NaN 3]);</code> | <code>[2]</code> |
| **`nanmedian`** <code>nanmedian(A)</code> | Median ignoring NaNs. | <code>r = nanmedian([1 NaN 3]);</code> | <code>[2]</code> |
| **`nanmin`** <code>nanmin(A)</code> | Minimum ignoring NaNs. | <code>r = nanmin([1 NaN 3]);</code> | <code>[1]</code> |
| **`nanprod`** <code>nanprod(A)</code> | Product ignoring NaNs. | <code>r = nanprod([1 NaN 3]);</code> | <code>[3]</code> |
| **`nanstd`** <code>nanstd(A)</code> | Standard deviation ignoring NaNs. | <code>r = nanstd([1 NaN 3]);</code> | <code>[2]</code> |
| **`nansum`** <code>nansum(A)</code> | Sum ignoring NaNs. | <code>r = nansum([1 NaN 3]);</code> | <code>[4]</code> |
| **`nanvar`** <code>nanvar(A)</code> | Variance ignoring NaNs. | <code>r = nanvar([1 NaN 3]);</code> | <code>[2]</code> |

## running/sorting

<a id="running-sorting"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`cumsum`** <code>cumsum(A)</code> | Cumulative sum. | <code>r = cumsum([1 2 3]);</code> | <code>[1 3 6]</code> |
| **`cumprod`** <code>cumprod(A)</code> | Cumulative product. | <code>r = cumprod([1 2 3]);</code> | <code>[1 2 6]</code> |
| **`diff`** <code>diff(A)</code> | Differences between adjacent elements. | <code>r = diff([1 3 6]);</code> | <code>[2 3]</code> |
| **`sort`** <code>sort(A[,dir])</code> | Sort elements. | <code>r = sort([3 1 2]);</code> | <code>[1 2 3]</code> |
| **`sortrows`** <code>sortrows(A[,cols])</code> | Sort matrix rows. | <code>r = sortrows([2 1;1 3]);</code> | <code>[1 3;2 1]</code> |
| **`unique`** <code>unique(A)</code> | Unique values. | <code>r = unique([1 2 2 3]);</code> | <code>[1 2 3]</code> |
| **`find`** <code>find(A)</code> | Indices of nonzero elements. | <code>r = find([0 1 0 1]);</code> | <code>[2;4]</code> |
| **`hist`** <code>hist(x[,n&#124;edges])</code> | Histogram counts and centers. | <code>[n,c] = hist([1 2 2 3],2); r = n;</code> | <code>[3 1]</code> |
| **`histc`** <code>histc(x,edges)</code> | Histogram counts with explicit edges. | <code>r = histc([1 2 3],[1 2 3]);</code> | <code>[1 1 1]</code> |

## sets

<a id="sets"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`union`** <code>union(A,B)</code> | Set union. | <code>r = union([1 2],[2 3]);</code> | <code>[1 2 3]</code> |
| **`intersect`** <code>intersect(A,B)</code> | Set intersection. | <code>r = intersect([1 2],[2 3]);</code> | <code>[2]</code> |
| **`setdiff`** <code>setdiff(A,B)</code> | Set difference. | <code>r = setdiff([1 2],[2 3]);</code> | <code>[1]</code> |
| **`setxor`** <code>setxor(A,B)</code> | Symmetric set difference. | <code>r = setxor([1 2],[2 3]);</code> | <code>[1 3]</code> |
| **`ismember`** <code>ismember(A,B)</code> | Membership test. | <code>r = ismember([1 3],[1 2]);</code> | <code>[1 0]</code> |

## bits

<a id="bits"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`bitand`** <code>bitand(a,b)</code> | Bitwise AND. | <code>r = bitand(5,3);</code> | <code>[1]</code> |
| **`bitor`** <code>bitor(a,b)</code> | Bitwise OR. | <code>r = bitor(5,3);</code> | <code>[7]</code> |
| **`bitxor`** <code>bitxor(a,b)</code> | Bitwise XOR. | <code>r = bitxor(5,3);</code> | <code>[6]</code> |
| **`bitcmp`** <code>bitcmp(a[,n])</code> | Bitwise complement. | <code>r = bitcmp(5,3);</code> | <code>[2]</code> |
| **`bitget`** <code>bitget(a,bit)</code> | Read a bit. | <code>r = bitget(5,1);</code> | <code>[1]</code> |
| **`bitset`** <code>bitset(a,bit[,value])</code> | Set a bit. | <code>r = bitset(0,1,1);</code> | <code>[1]</code> |
| **`bitshift`** <code>bitshift(a,n)</code> | Shift bits. | <code>r = bitshift(1,3);</code> | <code>[8]</code> |

## casts

<a id="casts"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`int8`** <code>int8(x)</code> | Round and saturate to int8 range. | <code>r = int8(1.6);</code> | <code>[2]</code> |
| **`int16`** <code>int16(x)</code> | Round and saturate to int16 range. | <code>r = int16(1.6);</code> | <code>[2]</code> |
| **`int32`** <code>int32(x)</code> | Round and saturate to int32 range. | <code>r = int32(1.6);</code> | <code>[2]</code> |
| **`int64`** <code>int64(x)</code> | Round and saturate to int64 range. | <code>r = int64(1.6);</code> | <code>[2]</code> |
| **`uint8`** <code>uint8(x)</code> | Round and saturate to uint8 range. | <code>r = uint8(300);</code> | <code>[255]</code> |
| **`uint16`** <code>uint16(x)</code> | Round and saturate to uint16 range. | <code>r = uint16(1.6);</code> | <code>[2]</code> |
| **`uint32`** <code>uint32(x)</code> | Round and saturate to uint32 range. | <code>r = uint32(1.6);</code> | <code>[2]</code> |
| **`uint64`** <code>uint64(x)</code> | Round and saturate to uint64 range. | <code>r = uint64(1.6);</code> | <code>[2]</code> |
| **`single`** <code>single(x)</code> | Round to single precision (stored as double). | <code>r = single(1.23456789);</code> | <code>[1.2346]</code> |
| **`class`** <code>class(x)</code> | Class name of a value. | <code>r = class(1);</code> | <code>double</code> |

## elementwise

<a id="elementwise"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`abs`** <code>abs(x)</code> | Absolute value. | <code>r = abs(-3);</code> | <code>[3]</code> |
| **`sqrt`** <code>sqrt(x)</code> | Square root. | <code>r = sqrt(9);</code> | <code>[3]</code> |
| **`exp`** <code>exp(x)</code> | Exponential. | <code>r = exp(0);</code> | <code>[1]</code> |
| **`log`** <code>log(x)</code> | Natural logarithm. | <code>r = log(exp(1));</code> | <code>[1]</code> |
| **`log10`** <code>log10(x)</code> | Base-10 logarithm. | <code>r = log10(100);</code> | <code>[2]</code> |
| **`log2`** <code>log2(x)</code> | Base-2 logarithm. | <code>r = log2(8);</code> | <code>[3]</code> |
| **`expm1`** <code>expm1(x)</code> | exp(x)-1 accurate near zero. | <code>r = expm1(0);</code> | <code>[0]</code> |
| **`log1p`** <code>log1p(x)</code> | log(1+x) accurate near zero. | <code>r = log1p(0);</code> | <code>[0]</code> |
| **`sin`** <code>sin(x)</code> | Sine (radians). | <code>r = sin(0);</code> | <code>[0]</code> |
| **`cos`** <code>cos(x)</code> | Cosine (radians). | <code>r = cos(0);</code> | <code>[1]</code> |
| **`tan`** <code>tan(x)</code> | Tangent (radians). | <code>r = tan(0);</code> | <code>[0]</code> |
| **`asin`** <code>asin(x)</code> | Inverse sine (radians). | <code>r = asin(0);</code> | <code>[0]</code> |
| **`acos`** <code>acos(x)</code> | Inverse cosine (radians). | <code>r = acos(1);</code> | <code>[0]</code> |
| **`atan`** <code>atan(x)</code> | Inverse tangent (radians). | <code>r = atan(1);</code> | <code>[0.7854]</code> |
| **`atan2`** <code>atan2(y,x)</code> | Four-quadrant inverse tangent. | <code>r = atan2(1,1);</code> | <code>[0.7854]</code> |
| **`sind`** <code>sind(x)</code> | Sine in degrees. | <code>r = sind(30);</code> | <code>[0.5]</code> |
| **`cosd`** <code>cosd(x)</code> | Cosine in degrees. | <code>r = cosd(60);</code> | <code>[0.5]</code> |
| **`tand`** <code>tand(x)</code> | Tangent in degrees. | <code>r = tand(45);</code> | <code>[1]</code> |
| **`asind`** <code>asind(x)</code> | Inverse sine in degrees. | <code>r = asind(0.5);</code> | <code>[30]</code> |
| **`acosd`** <code>acosd(x)</code> | Inverse cosine in degrees. | <code>r = acosd(0.5);</code> | <code>[60]</code> |
| **`atand`** <code>atand(x)</code> | Inverse tangent in degrees. | <code>r = atand(1);</code> | <code>[45]</code> |
| **`atan2d`** <code>atan2d(y,x)</code> | Four-quadrant inverse tangent in degrees. | <code>r = atan2d(1,1);</code> | <code>[45]</code> |
| **`sinh`** <code>sinh(x)</code> | Hyperbolic sine. | <code>r = sinh(0);</code> | <code>[0]</code> |
| **`cosh`** <code>cosh(x)</code> | Hyperbolic cosine. | <code>r = cosh(0);</code> | <code>[1]</code> |
| **`tanh`** <code>tanh(x)</code> | Hyperbolic tangent. | <code>r = tanh(0);</code> | <code>[0]</code> |
| **`sech`** <code>sech(x)</code> | Hyperbolic secant. | <code>r = sech(0);</code> | <code>[1]</code> |
| **`csch`** <code>csch(x)</code> | Hyperbolic cosecant. | <code>r = csch(1);</code> | <code>[0.8509]</code> |
| **`coth`** <code>coth(x)</code> | Hyperbolic cotangent. | <code>r = coth(1);</code> | <code>[1.313]</code> |
| **`floor`** <code>floor(x)</code> | Round toward minus infinity. | <code>r = floor(1.9);</code> | <code>[1]</code> |
| **`ceil`** <code>ceil(x)</code> | Round toward plus infinity. | <code>r = ceil(1.1);</code> | <code>[2]</code> |
| **`round`** <code>round(x)</code> | Round to nearest integer. | <code>r = round(1.5);</code> | <code>[2]</code> |
| **`fix`** <code>fix(x)</code> | Round toward zero. | <code>r = fix(-1.9);</code> | <code>[-1]</code> |
| **`sign`** <code>sign(x)</code> | Sign of a value. | <code>r = sign(-5);</code> | <code>[-1]</code> |
| **`mod`** <code>mod(a,b)</code> | Modulus with the sign of b. | <code>r = mod(5,3);</code> | <code>[2]</code> |
| **`rem`** <code>rem(a,b)</code> | Remainder with the sign of a. | <code>r = rem(-5,3);</code> | <code>[-2]</code> |
| **`hypot`** <code>hypot(a,b)</code> | Stable sqrt(a^2+b^2). | <code>r = hypot(3,4);</code> | <code>[5]</code> |
| **`nthroot`** <code>nthroot(x,n)</code> | Real n-th root. | <code>r = nthroot(27,3);</code> | <code>[3]</code> |
| **`nextpow2`** <code>nextpow2(x)</code> | Smallest power of two &gt;= &#124;x&#124;. | <code>r = nextpow2(5);</code> | <code>[3]</code> |
| **`pow2`** <code>pow2(x)</code> | Power of two. | <code>r = pow2(3);</code> | <code>[8]</code> |
| **`deg2rad`** <code>deg2rad(x)</code> | Degrees to radians. | <code>r = deg2rad(180);</code> | <code>[3.1416]</code> |
| **`rad2deg`** <code>rad2deg(x)</code> | Radians to degrees. | <code>r = rad2deg(pi);</code> | <code>[180]</code> |
| **`db2pow`** <code>db2pow(x)</code> | Decibels to power. | <code>r = db2pow(10);</code> | <code>[10]</code> |
| **`pow2db`** <code>pow2db(x)</code> | Power to decibels. | <code>r = pow2db(10);</code> | <code>[10]</code> |

## types/predicates

<a id="types-predicates"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`isnan`** <code>isnan(x)</code> | True for NaN entries. | <code>r = isnan(NaN);</code> | <code>[1]</code> |
| **`isinf`** <code>isinf(x)</code> | True for infinite entries. | <code>r = isinf(Inf);</code> | <code>[1]</code> |
| **`isfinite`** <code>isfinite(x)</code> | True for finite entries. | <code>r = isfinite(1);</code> | <code>[1]</code> |
| **`isempty`** <code>isempty(x)</code> | True for empty arrays. | <code>r = isempty([]);</code> | <code>[1]</code> |
| **`isscalar`** <code>isscalar(x)</code> | True for 1-by-1 arrays. | <code>r = isscalar(1);</code> | <code>[1]</code> |
| **`isvector`** <code>isvector(x)</code> | True for vectors. | <code>r = isvector([1 2]);</code> | <code>[1]</code> |
| **`isrow`** <code>isrow(x)</code> | True for row vectors. | <code>r = isrow([1 2]);</code> | <code>[1]</code> |
| **`iscolumn`** <code>iscolumn(x)</code> | True for column vectors. | <code>r = iscolumn([1;2]);</code> | <code>[1]</code> |
| **`ismatrix`** <code>ismatrix(x)</code> | True for 2-D arrays. | <code>r = ismatrix([1 2;3 4]);</code> | <code>[1]</code> |
| **`isnumeric`** <code>isnumeric(x)</code> | True for numeric arrays. | <code>r = isnumeric(1);</code> | <code>[1]</code> |
| **`ischar`** <code>ischar(x)</code> | True for character arrays. | <code>r = ischar('a');</code> | <code>[1]</code> |
| **`islogical`** <code>islogical(x)</code> | True for logical arrays. | <code>r = islogical(true);</code> | <code>[1]</code> |
| **`isreal`** <code>isreal(x)</code> | True when a value has no imaginary part. | <code>r = isreal(1);</code> | <code>[1]</code> |
| **`isa`** <code>isa(x,class)</code> | True when a value is of a class. | <code>r = isa(1,'double');</code> | <code>[1]</code> |
| **`isequal`** <code>isequal(a,b,...)</code> | Exact equality test. | <code>r = isequal(1,1);</code> | <code>[1]</code> |
| **`isequaln`** <code>isequaln(a,b,...)</code> | Equality test treating NaNs as equal. | <code>r = isequaln(NaN,NaN);</code> | <code>[1]</code> |

## complex

<a id="complex"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`complex`** <code>complex(a,b)</code> | Construct a complex value. | <code>r = complex(1,2);</code> | <code>1 + 2i</code> |
| **`real`** <code>real(z)</code> | Real part. | <code>r = real(1+2i);</code> | <code>[1]</code> |
| **`imag`** <code>imag(z)</code> | Imaginary part. | <code>r = imag(1+2i);</code> | <code>[2]</code> |
| **`conj`** <code>conj(z)</code> | Complex conjugate. | <code>r = conj(1+2i);</code> | <code>1 - 2i</code> |
| **`angle`** <code>angle(z)</code> | Phase angle in radians. | <code>r = angle(1+1i);</code> | <code>[0.7854]</code> |

## poly/signal

<a id="poly-signal"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`polyval`** <code>polyval(p,x)</code> | Evaluate a polynomial. | <code>r = polyval([1 2],3);</code> | <code>[5]</code> |
| **`polyfit`** <code>polyfit(x,y,n)</code> | Least-squares polynomial fit. | <code>r = polyfit([1 2 3],[2 4 6],1);</code> | <code>[2 1.3567e-15]</code> |
| **`roots`** <code>roots(p)</code> | Polynomial roots. | <code>r = sort(roots([1 -3 2]));</code> | <code>[1;2]</code> |
| **`poly`** <code>poly(r)</code> | Polynomial from roots. | <code>r = poly([1 2]);</code> | <code>[1 -3 2]</code> |
| **`conv`** <code>conv(a,b)</code> | Polynomial convolution. | <code>r = conv([1 2],[1 3]);</code> | <code>[1 5 6]</code> |
| **`deconv`** <code>deconv(a,b)</code> | Polynomial division. | <code>r = deconv([1 5 6],[1 2]);</code> | <code>[1 3]</code> |
| **`interp1`** <code>interp1(x,y,xi)</code> | One-dimensional interpolation. | <code>r = interp1([1 2],[10 20],1.5);</code> | <code>[15]</code> |
| **`trapz`** <code>trapz(x,y)</code> | Trapezoidal integration. | <code>r = trapz([1 2 3],[1 4 9]);</code> | <code>[9]</code> |
| **`histcounts`** <code>histcounts(x,n&#124;edges)</code> | Histogram bin counts. | <code>[n,e] = histcounts([1 2 2 3],2); r = n;</code> | <code>[1 3]</code> |

## dsp

<a id="dsp"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`fft`** <code>fft(x[,n])</code> | Discrete Fourier transform. | <code>r = fft([1 2 3 4]);</code> | <code>10 + 0i             -2 + 2i    -2 - 7.3479e-16i             -2 - 2i</code> |
| **`ifft`** <code>ifft(X[,n])</code> | Inverse discrete Fourier transform. | <code>r = ifft(fft([1 2 3 4]));</code> | <code>1 - 4.0574e-16i     2 - 9.3859e-17i     3 - 8.4117e-17i     4 + 2.1632e-16i</code> |
| **`fft2`** <code>fft2(A)</code> | Two-dimensional discrete Fourier transform. | <code>r = fft2([1 2;3 4]);</code> | <code>10 + 0i    -2 - 7.3479e-16i<br>       -4 - 8.5725e-16i     0 + 3.6739e-16i</code> |
| **`ifft2`** <code>ifft2(A)</code> | Two-dimensional inverse Fourier transform. | <code>r = ifft2(fft2([1 2;3 4]));</code> | <code>1 - 3.0616e-16i     2 - 1.8370e-16i<br>        3 - 1.8370e-16i     4 + 3.0616e-16i</code> |
| **`fftshift`** <code>fftshift(X)</code> | Shift zero-frequency components to the center. | <code>r = fftshift([1 2 3 4]);</code> | <code>[3 4 1 2]</code> |
| **`ifftshift`** <code>ifftshift(X)</code> | Undo fftshift. | <code>r = ifftshift([1 2 3 4]);</code> | <code>[3 4 1 2]</code> |
| **`dftmtx`** <code>dftmtx(n)</code> | Discrete Fourier transform matrix. | <code>r = size(dftmtx(4));</code> | <code>[4 4]</code> |
| **`filter`** <code>filter(b,a,x)</code> | One-dimensional digital filter. | <code>r = filter([1 1],1,[1 2 3]);</code> | <code>[1 3 5]</code> |
| **`xcorr`** <code>xcorr(x,y[,maxlag])</code> | Cross-correlation. | <code>r = xcorr([1 2 3]);</code> | <code>[3;8;14;8;3]</code> |
| **`freqz`** <code>freqz(b,a[,n])</code> | Digital filter frequency response. | <code>[h,w] = freqz([1],1,4); r = h;</code> | <code>[1;1;1;1]</code> |
| **`unwrap`** <code>unwrap(phase)</code> | Unwrap phase angles. | <code>r = unwrap([0 pi 3*pi]);</code> | <code>[0 3.1416 3.1416]</code> |
| **`hamming`** <code>hamming(n)</code> | Hamming window. | <code>r = hamming(4);</code> | <code>[0.08;0.77;0.77;0.08]</code> |
| **`hann`** <code>hann(n)</code> | Hann window. | <code>r = hann(4);</code> | <code>[0;0.75;0.75;0]</code> |
| **`blackman`** <code>blackman(n)</code> | Blackman window. | <code>r = blackman(4);</code> | <code>[-1.3878e-17;0.63;0.63;-1.3878e-17]</code> |
| **`bartlett`** <code>bartlett(n)</code> | Bartlett window. | <code>r = bartlett(4);</code> | <code>[0;0.6667;0.6667;0]</code> |
| **`rectwin`** <code>rectwin(n)</code> | Rectangular window. | <code>r = rectwin(4);</code> | <code>[1;1;1;1]</code> |
| **`triang`** <code>triang(n)</code> | Triangular window. | <code>r = triang(4);</code> | <code>[0.25;0.75;0.75;0.25]</code> |
| **`kaiser`** <code>kaiser(n,beta)</code> | Kaiser window. | <code>r = kaiser(4,2);</code> | <code>[0.4387;0.9243;0.9243;0.4387]</code> |
| **`gausswin`** <code>gausswin(n,alpha)</code> | Gaussian window. | <code>r = gausswin(4);</code> | <code>[0.0439;0.7066;0.7066;0.0439]</code> |
| **`sinc`** <code>sinc(x)</code> | Normalized sinc function. | <code>r = sinc(0);</code> | <code>[1]</code> |
| **`rectpuls`** <code>rectpuls(t[,width])</code> | Rectangular pulse. | <code>r = rectpuls(0.5);</code> | <code>[1]</code> |
| **`tripuls`** <code>tripuls(t[,width,skew])</code> | Triangular pulse. | <code>r = tripuls(0.5);</code> | <code>[0.5]</code> |
| **`fir1`** <code>fir1(n,wn[,type])</code> | Windowed-sinc FIR filter design. | <code>r = fir1(3,0.5);</code> | <code>[0.012 0.3466 0.3466 0.012]</code> |

## solvers

<a id="solvers"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`fzero`** <code>fzero(fun,x0)</code> | Root of a scalar function. | <code>r = fzero(@(x) x.^2-2, 1);</code> | <code>[1.4142]</code> |
| **`fminbnd`** <code>fminbnd(fun,a,b)</code> | Bounded scalar minimization. | <code>r = fminbnd(@(x) (x-1).^2, 0, 2);</code> | <code>[1]</code> |
| **`fminsearch`** <code>fminsearch(fun,x0)</code> | Derivative-free unconstrained minimization. | <code>r = fminsearch(@(x) (x(1)-1).^2+(x(2)-2).^2, [0 0]);</code> | <code>[1;2]</code> |
| **`fminunc`** <code>fminunc(fun,x0)</code> | Unconstrained minimization. | <code>r = fminunc(@(x) (x(1)-1).^2+(x(2)-2).^2, [0 0]);</code> | <code>[1;2]</code> |
| **`fsolve`** <code>fsolve(fun,x0)</code> | Solve a nonlinear equation. | <code>r = fsolve(@(x) x.^2-4, 1);</code> | <code>[2]</code> |
| **`lsqnonlin`** <code>lsqnonlin(fun,x0)</code> | Nonlinear least squares. | <code>r = lsqnonlin(@(x) x.^2-4, 1);</code> | <code>[2]</code> |
| **`lsqcurvefit`** <code>lsqcurvefit(fun,x0,xdata,ydata)</code> | Fit a nonlinear curve. | <code>r = lsqcurvefit(@(x,t) x(1)*t, 1, [1 2 3], [2 4 6]);</code> | <code>[2]</code> |
| **`linprog`** <code>linprog(f,A,b,Aeq,beq,lb,ub)</code> | Linear programming. | <code>[x,f] = linprog([-1;-1],[1 2;4 2],[4;12],[],[],[0;0],[]); r = x;</code> | <code>[2.6667;0.6667]</code> |
| **`quadprog`** <code>quadprog(H,f,A,b,Aeq,beq,lb,ub)</code> | Quadratic programming. | <code>r = quadprog([2 0;0 2],[-2;-4],[],[],[],[],[0;0],[1;1]);</code> | <code>[1;1]</code> |
| **`intlinprog`** <code>intlinprog(f,intcon,A,b,Aeq,beq,lb,ub)</code> | Mixed-integer linear programming. | <code>r = intlinprog([-1;-1],[1 2],[2 2],3,[],[],[0;0],[1;1]);</code> | <code>[0;1]</code> |
| **`fmincon`** <code>fmincon(fun,x0,A,b,Aeq,beq,lb,ub,nonlcon)</code> | Constrained nonlinear minimization. | <code>r = fmincon(@(v) (v(1)-2).^2+(v(2)+1).^2,[0 0]);</code> | <code>[2;-1]</code> |
| **`patternsearch`** <code>patternsearch(fun,x0,...)</code> | Derivative-free constrained minimization. | <code>r = round(patternsearch(@(v) (v(1)-1.3).^2+(v(2)+0.7).^2,[0 0])*10)/10;</code> | <code>[1.3;-0.7]</code> |
| **`ga`** <code>ga(fun,nvars,...)</code> | Genetic algorithm. | <code>rng(1); r = round(ga(@(v) (v-2).^2,1,[],[],[],[],-5,5));</code> | <code>[2]</code> |
| **`integral`** <code>integral(fun,a,b)</code> | Adaptive numerical integration. | <code>r = integral(@(x) x.^2, 0, 1);</code> | <code>[0.3333]</code> |
| **`quad`** <code>quad(fun,a,b)</code> | Adaptive Simpson integration. | <code>r = quad(@(x) x.^2, 0, 1);</code> | <code>[0.3333]</code> |
| **`optimset`** <code>optimset(name,value,...)</code> | Create an options struct. | <code>o = optimset('Display','off'); r = optimget(o,'Display');</code> | <code>off</code> |
| **`optimoptions`** <code>optimoptions(solver,name,value,...)</code> | Create solver options. | <code>o = optimoptions('linprog','Display','off'); r = optimget(o,'Display');</code> | <code>off</code> |
| **`optimget`** <code>optimget(options,name)</code> | Read an option value. | <code>o = optimset('Display','off'); r = optimget(o,'Display');</code> | <code>off</code> |

## control

<a id="control"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`tf`** <code>tf(num,den)</code> | Create a transfer-function model. | <code>r = class(tf(1,[1 1]));</code> | <code>tf</code> |
| **`ss`** <code>ss(A,B,C,D)</code> | Create a state-space model. | <code>r = class(ss([-1],[1],[1],[0]));</code> | <code>ss</code> |
| **`zpk`** <code>zpk(z,p,k)</code> | Create a zero-pole-gain model. | <code>r = class(zpk([],[-1],1));</code> | <code>zpk</code> |
| **`tfdata`** <code>tfdata(sys)</code> | Transfer-function numerator and denominator. | <code>[num,den] = tfdata(tf(1,[1 1])); r = den;</code> | <code>[1 1]</code> |
| **`ssdata`** <code>ssdata(sys)</code> | State-space matrices. | <code>[A,B,C,D] = ssdata(ss([-1],[1],[1],[0])); r = A;</code> | <code>[-1]</code> |
| **`zpkdata`** <code>zpkdata(sys)</code> | Zero-pole-gain data. | <code>[z,p,k] = zpkdata(zpk([],[-1],1)); r = p;</code> | <code>[-1]</code> |
| **`pole`** <code>pole(sys)</code> | System poles. | <code>r = pole(tf(1,[1 2 1]));</code> | <code>[-1;-1]</code> |
| **`zero`** <code>zero(sys)</code> | System zeros. | <code>r = zero(tf([1 2],[1 3]));</code> | <code>[-2]</code> |
| **`dcgain`** <code>dcgain(sys)</code> | Steady-state gain. | <code>r = dcgain(tf(2,[1 1]));</code> | <code>[2]</code> |
| **`series`** <code>series(sys1,sys2)</code> | Series connection. | <code>r = dcgain(series(tf(1,[1 1]),tf(2,[1 2])));</code> | <code>[1]</code> |
| **`parallel`** <code>parallel(sys1,sys2)</code> | Parallel connection. | <code>r = dcgain(parallel(tf(1,[1 1]),tf(2,[1 2])));</code> | <code>[2]</code> |
| **`feedback`** <code>feedback(sys,fb)</code> | Feedback connection. | <code>r = dcgain(feedback(tf(1,[1 1]),1));</code> | <code>[0.5]</code> |
| **`tf2ss`** <code>tf2ss(num,den)</code> | Transfer function to state space. | <code>[A,B,C,D] = tf2ss(1,[1 2 1]); r = size(A);</code> | <code>[2 2]</code> |
| **`ss2tf`** <code>ss2tf(A,B,C,D)</code> | State space to transfer function. | <code>[num,den] = ss2tf([-1],[1],[1],[0]); r = den;</code> | <code>[1 1]</code> |
| **`step`** <code>step(sys[,t])</code> | Step response. | <code>r = size(step(tf(1,[1 1]),0:0.1:1));</code> | <code>[11 1]</code> |
| **`impulse`** <code>impulse(sys[,t])</code> | Impulse response. | <code>r = size(impulse(tf(1,[1 1]),0:0.1:1));</code> | <code>[11 1]</code> |
| **`initial`** <code>initial(sys,x0[,t])</code> | Initial-condition response. | <code>r = size(initial(ss([-1],[1],[1],[0]),1,0:0.1:1));</code> | <code>[11 1]</code> |
| **`lsim`** <code>lsim(sys,u,t)</code> | Simulate a linear system. | <code>r = size(lsim(tf(1,[1 1]),ones(3,1),0:2));</code> | <code>[3 1]</code> |
| **`bode`** <code>bode(sys[,w])</code> | Bode magnitude and phase. | <code>[mag,phase] = bode(tf(1,[1 1]),1); r = mag;</code> | <code>[0.7071]</code> |
| **`freqresp`** <code>freqresp(sys,w)</code> | Frequency response. | <code>r = freqresp(tf(1,[1 1]),1);</code> | <code>0.5 - 0.5i</code> |
| **`pid`** <code>pid(Kp,Ki,Kd[,Tf])</code> | Create a PID controller. | <code>r = class(pid(1,2,3));</code> | <code>tf</code> |
| **`pidtune`** <code>pidtune(sys,type[,wc])</code> | Tune a PID controller. | <code>[C,info] = pidtune(tf(1,[1 1]),'PI'); r = info.Kp;</code> | <code>[0.366]</code> |
| **`margin`** <code>margin(sys)</code> | Gain and phase margins. | <code>[Gm,Pm,Wcg,Wcp] = margin(tf(1,[1 1 0])); r = Pm;</code> | <code>[51.8273]</code> |
| **`nyquist`** <code>nyquist(sys[,w])</code> | Nyquist response. | <code>[re,im,w] = nyquist(tf(1,[1 1]),1); r = re;</code> | <code>[0.5]</code> |
| **`isstable`** <code>isstable(sys)</code> | True when all poles are stable. | <code>r = isstable(tf(1,[1 1]));</code> | <code>[1]</code> |
| **`damp`** <code>damp(sys)</code> | Natural frequency and damping. | <code>[wn,zeta,p] = damp(tf(1,[1 2 1])); r = zeta;</code> | <code>[1;1]</code> |
| **`bandwidth`** <code>bandwidth(sys)</code> | Closed-loop bandwidth. | <code>r = bandwidth(tf(1,[1 1]));</code> | <code>[1]</code> |

## graph objects

<a id="graph-objects"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`graph`** <code>graph(A) &#124; graph(s,t[,w])</code> | Undirected graph. | <code>G = graph([0 1 1;1 0 0;1 0 0]); r = numnodes(G);</code> | <code>[3]</code> |
| **`digraph`** <code>digraph(A) &#124; digraph(s,t[,w])</code> | Directed graph. | <code>D = digraph([1 2],[2 3]); r = numnodes(D);</code> | <code>[3]</code> |
| **`gplot`** <code>gplot(A,XY)</code> | Plot a graph from an adjacency matrix and coordinates. | <code>r = gplot([0 1;1 0],[0 0;1 0]);</code> | <code>[1]</code> |
| **`numnodes`** <code>numnodes(G)</code> | Number of graph nodes. | <code>G = graph([0 1;1 0]); r = numnodes(G);</code> | <code>[2]</code> |
| **`numedges`** <code>numedges(G)</code> | Number of graph edges. | <code>G = graph([0 1;1 0]); r = numedges(G);</code> | <code>[1]</code> |
| **`degree`** <code>degree(G)</code> | Node degrees. | <code>G = graph([0 1 1;1 0 0;1 0 0]); r = degree(G);</code> | <code>[2;1;1]</code> |
| **`conncomp`** <code>conncomp(G)</code> | Connected-component labels. | <code>G = graph([0 1;1 0]); [c,k] = conncomp(G); r = k;</code> | <code>[1]</code> |
| **`neighbors`** <code>neighbors(G,node)</code> | Adjacent nodes. | <code>G = graph([0 1;1 0]); r = neighbors(G,1);</code> | <code>'2'</code> |
| **`shortestpath`** <code>shortestpath(G,s,t)</code> | Shortest path and distance. | <code>G = graph([0 1 1;1 0 0;1 0 0]); [p,d] = shortestpath(G,2,3); r = d;</code> | <code>[2]</code> |
| **`distances`** <code>distances(G[,s])</code> | All-pairs or single-source shortest distances. | <code>G = graph([0 1 1;1 0 0;1 0 0]); r = distances(G);</code> | <code>[0 1 1;1 0 2;1 2 0]</code> |
| **`bfsearch`** <code>bfsearch(G,s)</code> | Breadth-first search order. | <code>G = graph([0 1 1;1 0 0;1 0 0]); r = bfsearch(G,1);</code> | <code>[1 2 3]</code> |
| **`dfsearch`** <code>dfsearch(G,s)</code> | Depth-first search order. | <code>G = graph([0 1 1;1 0 0;1 0 0]); r = dfsearch(G,1);</code> | <code>[1 2 3]</code> |
| **`adjacency`** <code>adjacency(G)</code> | Graph adjacency matrix. | <code>G = graph([0 1;1 0]); r = adjacency(G);</code> | <code>[0 1;1 0]</code> |
| **`incidence`** <code>incidence(G)</code> | Node-edge incidence matrix. | <code>G = graph([0 1 1;1 0 0;1 0 0]); r = incidence(G);</code> | <code>[1 1;1 0;0 1]</code> |
| **`isgraph`** <code>isgraph(x)</code> | True for graph objects. | <code>r = isgraph(graph([0 1;1 0]));</code> | <code>[1]</code> |

## distances and clustering

<a id="distances-and-clustering"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`pdist`** <code>pdist(X[,metric])</code> | Pairwise distances between rows. | <code>r = pdist([0 0;3 4]);</code> | <code>[5]</code> |
| **`pdist2`** <code>pdist2(X,Y[,metric])</code> | Pairwise distances between two point sets. | <code>r = pdist2([0 0;3 4],[0 0]);</code> | <code>[0;5]</code> |
| **`squareform`** <code>squareform(d)</code> | Convert between condensed and square distance forms. | <code>r = squareform([1 2 3]);</code> | <code>[0 1 2;1 0 3;2 3 0]</code> |
| **`jaccard`** <code>jaccard(A,B)</code> | Jaccard similarity. | <code>r = jaccard([1 0 1 1],[1 1 0 1]);</code> | <code>[0.5]</code> |
| **`jaccarddistance`** <code>jaccarddistance(A,B)</code> | Jaccard distance. | <code>r = jaccarddistance([1 0 1 1],[1 1 0 1]);</code> | <code>[0.5]</code> |
| **`setmetrics`** <code>setmetrics(A,B)</code> | Set precision, recall, F1 and related scores. | <code>m = setmetrics({'a','b','c'},{'b','c','d'}); r = m.F1;</code> | <code>[0.6667]</code> |
| **`confusionmat`** <code>confusionmat(true,pred)</code> | Confusion matrix and class labels. | <code>r = confusionmat([1 1 2 2],[1 2 2 2]);</code> | <code>[1 1;0 2]</code> |
| **`clustermetrics`** <code>clustermetrics(true,pred)</code> | Adjusted Rand index, NMI and purity. | <code>m = clustermetrics([1 1 2 2],[1 1 2 2]); r = m.ARI;</code> | <code>[1]</code> |
| **`cluster`** <code>cluster(Z,cutoff) &#124; cluster(Z,'maxclust',k)</code> | Cut a linkage tree into labels. | <code>Z = linkage([1 2 3],'single'); r = cluster(Z,'maxclust',2);</code> | <code>[1;1;2]</code> |
| **`clusterdata`** <code>clusterdata(X,k&#124;cutoff)</code> | Distance, linkage and cut in one call. | <code>r = clusterdata([0 0;1 0;10 0;11 0],2);</code> | <code>[1;1;2;2]</code> |
| **`cophenet`** <code>cophenet(Z[,Y])</code> | Cophenetic distances or cophenetic correlation. | <code>Z = linkage([1 2 3],'single'); r = cophenet(Z,[1 3 2]);</code> | <code>[0.866]</code> |
| **`silhouette`** <code>silhouette(X,labels)</code> | Silhouette scores and mean. | <code>[s,m] = silhouette([0 0;1 0;10 0;11 0],[1;1;2;2]); r = m;</code> | <code>[0.8997]</code> |
| **`randindex`** <code>randindex(true,pred)</code> | Rand index; the second output is the adjusted Rand index. | <code>[ri,ari] = randindex([1 1 2 2],[1 1 2 3]); r = [ri ari];</code> | <code>[0.8333 0.5714]</code> |
| **`mutualinfo`** <code>mutualinfo(true,pred)</code> | Mutual information (nats); the second output is normalised MI. | <code>[mi,nmi] = mutualinfo([1 1 2 2],[1 1 2 3]); r = [mi nmi];</code> | <code>[0.6931 0.8165]</code> |
| **`evalclusters`** <code>evalclusters(X, labels&#124;K, criterion)</code> | Evaluate a clustering using silhouette, Davies-Bouldin or Calinski-Harabasz. | <code>ev = evalclusters([0 0;1 0;10 0;11 0],[1;1;2;2],'silhouette'); r = ev.CriterionValues;</code> | <code>[0.8997]</code> |
| **`linkage`** <code>linkage(Y,method)</code> | Agglomerative linkage tree. | <code>r = linkage([1 2 3],'single');</code> | <code>[1 2 1;4 3 2]</code> |
| **`optimalleaforder`** <code>optimalleaforder(Z,Y)</code> | Optimal leaf ordering for a tree. | <code>Z = linkage([1 2 3],'single'); r = optimalleaforder(Z,[1 3 2]);</code> | <code>[1;2;3;4]</code> |

## chemistry and sequences

<a id="chemistry-and-sequences"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`formulaatoms`** <code>formulaatoms(formula)</code> | Element counts from an elemental formula. | <code>r = formulaatoms('C6H12O6').C;</code> | <code>[6]</code> |
| **`formulaweight`** <code>formulaweight(formula)</code> | Molecular weight from a formula. | <code>r = formulaweight('C6H12O6');</code> | <code>[180.1559]</code> |
| **`formulacheck`** <code>formulacheck(formula)</code> | Validate an elemental formula. | <code>r = formulacheck('C6H12O6');</code> | <code>[1]</code> |
| **`inferformula`** <code>inferformula(coeffs,formulas,target)</code> | Infer a missing reaction species formula. | <code>i = inferformula([-1 -1 1 1 1],{'C10H16N5O13P3','H2O','C10H16N5O10P2','','H+'},4); r = i.Formula;</code> | <code>HO4P</code> |
| **`parsereaction`** <code>parsereaction(equation)</code> | Parse a reaction equation. | <code>r = parsereaction('2 H2O + ATP &lt;=&gt; ADP + Pi').Coefficients;</code> | <code>[-2 -1 1 1]</code> |
| **`formatreaction`** <code>formatreaction(r)</code> | Canonical reaction text. | <code>r = formatreaction(parsereaction('2 H2O + ATP &lt;=&gt; ADP + Pi'));</code> | <code>2 H2O + ATP &lt;=&gt; ADP + Pi</code> |
| **`canonicalreaction`** <code>canonicalreaction(r)</code> | Normalize reaction orientation. | <code>r = formatreaction(canonicalreaction(parsereaction('2 H2O + ATP &lt;=&gt; ADP + Pi')));</code> | <code>ADP + Pi &lt;=&gt; ATP + 2 H2O</code> |
| **`rxnequal`** <code>rxnequal(a,b[,options])</code> | Compare two reactions. | <code>r = rxnequal(parsereaction('A -&gt; B'),parsereaction('B -&gt; A'),'Reverse',1);</code> | <code>[1]</code> |
| **`rxnbalance`** <code>rxnbalance(r,formulas)</code> | Check elemental reaction balance. | <code>b = rxnbalance('2 H2 + O2 -&gt; 2 H2O',{'H2','O2','H2O'}); r = b.Balanced;</code> | <code>[1]</code> |
| **`gprparse`** <code>gprparse(rule)</code> | Parse a GPR Boolean rule. | <code>r = gprparse('g1 &amp; g2').Type;</code> | <code>and</code> |
| **`gpreval`** <code>gpreval(rule,genes)</code> | Evaluate a GPR rule. | <code>r = gpreval(gprparse('g1 &amp; g2'),{'g1','g2'});</code> | <code>[1]</code> |
| **`gprgenes`** <code>gprgenes(rule)</code> | Genes mentioned by a GPR rule. | <code>r = gprgenes(gprparse('g1 &amp; (g2 &#124; g3)'));</code> | <code>'g1'                'g2'                'g3'</code> |
| **`gprmerge`** <code>gprmerge(a,b)</code> | Combine two GPR rules with OR. | <code>r = gprmerge(gprparse('g1'),gprparse('g2'));</code> | <code>(g1 &#124; g2)</code> |
| **`fastaread`** <code>fastaread(file)</code> | Read a FASTA file. | <code>fastaread('demo.fasta')</code> | <code>fastaread('demo.fasta') returns a struct array with Header and Sequence fields.</code> |
| **`fastawrite`** <code>fastawrite(file,headers,sequences)</code> | Write a FASTA file. | <code>fastawrite('out.fasta', {'seq1'}, {'MKV'})</code> | <code>fastawrite('out.fasta',{'seq1'},{'MKV'}) writes one record.</code> |
| **`genbankread`** <code>genbankread(file)</code> | Read coding features from a GenBank file. | <code>genbankread('demo.gbk')</code> | <code>genbankread('demo.gbk') returns one struct row per coding feature.</code> |

## statistics

<a id="statistics"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`normpdf`** <code>normpdf(x[,mu,sigma])</code> | Normal probability density. | <code>r = normpdf(0);</code> | <code>[0.3989]</code> |
| **`normcdf`** <code>normcdf(x[,mu,sigma])</code> | Normal cumulative distribution. | <code>r = normcdf(0);</code> | <code>[0.5]</code> |
| **`norminv`** <code>norminv(p[,mu,sigma])</code> | Normal inverse CDF. | <code>r = norminv(0.5);</code> | <code>[0]</code> |
| **`exppdf`** <code>exppdf(x[,mu])</code> | Exponential probability density. | <code>r = exppdf(1);</code> | <code>[0.3679]</code> |
| **`expcdf`** <code>expcdf(x[,mu])</code> | Exponential cumulative distribution. | <code>r = expcdf(1);</code> | <code>[0.6321]</code> |
| **`expinv`** <code>expinv(p[,mu])</code> | Exponential inverse CDF. | <code>r = expinv(0.5);</code> | <code>[0.6931]</code> |
| **`unifpdf`** <code>unifpdf(x[,a,b])</code> | Uniform probability density. | <code>r = unifpdf(0.5);</code> | <code>[1]</code> |
| **`unifcdf`** <code>unifcdf(x[,a,b])</code> | Uniform cumulative distribution. | <code>r = unifcdf(0.5);</code> | <code>[0.5]</code> |
| **`unifinv`** <code>unifinv(p[,a,b])</code> | Uniform inverse CDF. | <code>r = unifinv(0.5);</code> | <code>[0.5]</code> |
| **`poisspdf`** <code>poisspdf(k,lambda)</code> | Poisson probability mass. | <code>r = poisspdf(2,3);</code> | <code>[0.224]</code> |
| **`poisscdf`** <code>poisscdf(k,lambda)</code> | Poisson cumulative distribution. | <code>r = poisscdf(2,3);</code> | <code>[0.4232]</code> |
| **`poissinv`** <code>poissinv(p,lambda)</code> | Poisson inverse CDF. | <code>r = poissinv(0.5,3);</code> | <code>[3]</code> |
| **`binopdf`** <code>binopdf(k,n,p)</code> | Binomial probability mass. | <code>r = binopdf(2,5,0.5);</code> | <code>[0.3125]</code> |
| **`binocdf`** <code>binocdf(k,n,p)</code> | Binomial cumulative distribution. | <code>r = binocdf(2,5,0.5);</code> | <code>[0.5]</code> |
| **`binoinv`** <code>binoinv(p,n,prob)</code> | Binomial inverse CDF. | <code>r = binoinv(0.5,5,0.5);</code> | <code>[3]</code> |
| **`wblpdf`** <code>wblpdf(x,a,b)</code> | Weibull probability density. | <code>r = wblpdf(1,2,3);</code> | <code>[0.1989]</code> |
| **`wblcdf`** <code>wblcdf(x,a,b)</code> | Weibull cumulative distribution. | <code>r = wblcdf(1,2,3);</code> | <code>[0.1052]</code> |
| **`wblinv`** <code>wblinv(p,a,b)</code> | Weibull inverse CDF. | <code>r = wblinv(0.5,2,3);</code> | <code>[2.4977]</code> |
| **`lognpdf`** <code>lognpdf(x,mu,sigma)</code> | Lognormal probability density. | <code>r = lognpdf(1,0,1);</code> | <code>[0.3989]</code> |
| **`logncdf`** <code>logncdf(x,mu,sigma)</code> | Lognormal cumulative distribution. | <code>r = logncdf(1,0,1);</code> | <code>[0.5]</code> |
| **`logninv`** <code>logninv(p,mu,sigma)</code> | Lognormal inverse CDF. | <code>r = logninv(0.5,0,1);</code> | <code>[1]</code> |
| **`gampdf`** <code>gampdf(x,a,b)</code> | Gamma probability density. | <code>r = gampdf(2,2,1);</code> | <code>[0.2707]</code> |
| **`gamcdf`** <code>gamcdf(x,a,b)</code> | Gamma cumulative distribution. | <code>r = gamcdf(2,2,1);</code> | <code>[0.594]</code> |
| **`gaminv`** <code>gaminv(p,a,b)</code> | Gamma inverse CDF. | <code>r = gaminv(0.5,2,1);</code> | <code>[1.6783]</code> |
| **`betapdf`** <code>betapdf(x,a,b)</code> | Beta probability density. | <code>r = betapdf(0.5,2,2);</code> | <code>[1.5]</code> |
| **`betacdf`** <code>betacdf(x,a,b)</code> | Beta cumulative distribution. | <code>r = betacdf(0.5,2,2);</code> | <code>[0.5]</code> |
| **`betainv`** <code>betainv(p,a,b)</code> | Beta inverse CDF. | <code>r = betainv(0.5,2,2);</code> | <code>[0.5]</code> |
| **`chi2pdf`** <code>chi2pdf(x,df)</code> | Chi-square probability density. | <code>r = chi2pdf(2,3);</code> | <code>[0.2076]</code> |
| **`chi2cdf`** <code>chi2cdf(x,df)</code> | Chi-square cumulative distribution. | <code>r = chi2cdf(2,3);</code> | <code>[0.4276]</code> |
| **`chi2inv`** <code>chi2inv(p,df)</code> | Chi-square inverse CDF. | <code>r = chi2inv(0.5,3);</code> | <code>[2.366]</code> |
| **`tpdf`** <code>tpdf(x,df)</code> | Student t probability density. | <code>r = tpdf(0,5);</code> | <code>[0.3796]</code> |
| **`tcdf`** <code>tcdf(x,df)</code> | Student t cumulative distribution. | <code>r = tcdf(0,5);</code> | <code>[0.5]</code> |
| **`tinv`** <code>tinv(p,df)</code> | Student t inverse CDF. | <code>r = tinv(0.975,5);</code> | <code>[2.5706]</code> |
| **`fpdf`** <code>fpdf(x,d1,d2)</code> | F probability density. | <code>r = fpdf(1,3,5);</code> | <code>[0.3612]</code> |
| **`fcdf`** <code>fcdf(x,d1,d2)</code> | F cumulative distribution. | <code>r = fcdf(1,3,5);</code> | <code>[0.5351]</code> |
| **`finv`** <code>finv(p,d1,d2)</code> | F inverse CDF. | <code>r = finv(0.5,3,5);</code> | <code>[0.9071]</code> |
| **`exprnd`** <code>exprnd(mu,sz)</code> | Exponential random numbers. | <code>rng(1); r = exprnd(1,1,3);</code> | <code>[0.836 1.3696 3.5406]</code> |
| **`unifrnd`** <code>unifrnd(a,b,sz)</code> | Uniform random numbers. | <code>rng(1); r = unifrnd(0,1,1,3);</code> | <code>[0.5666 0.7458 0.971]</code> |
| **`poissrnd`** <code>poissrnd(lambda,sz)</code> | Poisson random numbers. | <code>rng(1); r = poissrnd(3,1,3);</code> | <code>[7 4 2]</code> |
| **`binornd`** <code>binornd(n,p,sz)</code> | Binomial random numbers. | <code>rng(1); r = binornd(5,0.5,1,3);</code> | <code>[2 1 3]</code> |
| **`wblrnd`** <code>wblrnd(a,b,sz)</code> | Weibull random numbers. | <code>rng(1); r = wblrnd(2,3,1,3);</code> | <code>[2.743 3.5108 5.6449]</code> |
| **`lognrnd`** <code>lognrnd(mu,sigma,sz)</code> | Lognormal random numbers. | <code>rng(1); r = lognrnd(0,1,1,3);</code> | <code>[0.9721 0.7962 1.1086]</code> |
| **`fitdist`** <code>fitdist(x,dist)</code> | Fit a probability distribution. | <code>pd = fitdist([1 2 3 4],'Normal'); r = pd.mu;</code> | <code>[2.5]</code> |
| **`pdf`** <code>pdf(pd,x)</code> | Probability density of a fitted distribution. | <code>pd = fitdist([1 2 3 4],'Normal'); r = pdf(pd,1);</code> | <code>[0.1451]</code> |
| **`cdf`** <code>cdf(pd,x)</code> | CDF of a fitted distribution. | <code>pd = fitdist([1 2 3 4],'Normal'); r = cdf(pd,2.5);</code> | <code>[0.5]</code> |
| **`icdf`** <code>icdf(pd,p)</code> | Inverse CDF of a fitted distribution. | <code>pd = fitdist([1 2 3 4],'Normal'); r = icdf(pd,0.5);</code> | <code>[2.5]</code> |
| **`random`** <code>random(name,...)</code> | Random values from a named distribution. | <code>rng(1); pd = fitdist([1 2 3 4],'Normal'); r = random(pd,1,3);</code> | <code>[2.4684 2.2452 2.6153]</code> |
| **`iqr`** <code>iqr(x)</code> | Interquartile range. | <code>r = iqr([1 2 3 4]);</code> | <code>[1.5]</code> |
| **`corrcoef`** <code>corrcoef(x,y)</code> | Correlation coefficients and p-values. | <code>[R,P] = corrcoef([1;2;3],[2;4;6]); r = R(1,2);</code> | <code>[1]</code> |
| **`regress`** <code>regress(y,X)</code> | Multiple linear regression. | <code>r = regress([1;2;3],[ones(3,1) [1;2;3]]);</code> | <code>[8.8818e-16;1]</code> |
| **`kmeans`** <code>kmeans(X,k,...)</code> | k-means clustering. | <code>rng(1); [idx,C] = kmeans([0 0;0.1 0;5 5;5.1 5],2); r = idx;</code> | <code>[2;2;1;1]</code> |
| **`ttest`** <code>ttest(x[,m,alpha,tail])</code> | One-sample t-test. | <code>[h,p,ci,st] = ttest([2 3 4],0); r = p;</code> | <code>[0.0351]</code> |
| **`ttest2`** <code>ttest2(x,y,...)</code> | Two-sample t-test. | <code>[h,p] = ttest2([1 2 3],[2 3 4]); r = p;</code> | <code>[0.2879]</code> |
| **`kstest`** <code>kstest(x[,'CDF',fun])</code> | Kolmogorov-Smirnov test. | <code>[h,p] = kstest([-1 0 1]); r = p;</code> | <code>[1]</code> |
| **`adtest`** <code>adtest(x)</code> | Anderson-Darling test. | <code>[h,p] = adtest([-2 -1 0 1 2 3 4 5]); r = p;</code> | <code>[0.9615]</code> |
| **`chi2gof`** <code>chi2gof(x,...)</code> | Chi-square goodness-of-fit test. | <code>y = norminv(linspace(0.005,0.995,200),0,1); [h,p] = chi2gof(y); r = p;</code> | <code>[1]</code> |
| **`pca`** <code>pca(X[,'NumComponents',k])</code> | Principal component analysis. | <code>r = size(pca([1 2;2 4;3 6]));</code> | <code>[2 2]</code> |
| **`cmdscale`** <code>cmdscale(D)</code> | Classical multidimensional scaling. | <code>r = cmdscale([0 1;1 0]);</code> | <code>[0.5;-0.5]</code> |
| **`pcoa`** <code>pcoa(D)</code> | Principal coordinates analysis. | <code>r = pcoa([0 1;1 0]);</code> | <code>[0.5;-0.5]</code> |
| **`bonferroni`** <code>bonferroni(p)</code> | Bonferroni p-value adjustment. | <code>r = bonferroni([0.01 0.04]);</code> | <code>[0.02 0.08]</code> |
| **`holm`** <code>holm(p)</code> | Holm p-value adjustment. | <code>r = holm([0.01 0.04]);</code> | <code>[0.02 0.04]</code> |
| **`sidak`** <code>sidak(p)</code> | Sidak p-value adjustment. | <code>r = sidak([0.01 0.04]);</code> | <code>[0.0199 0.0784]</code> |
| **`benjamini_hochberg`** <code>benjamini_hochberg(p)</code> | Benjamini-Hochberg p-value adjustment. | <code>r = benjamini_hochberg([0.01 0.04]);</code> | <code>[0.02 0.04]</code> |
| **`mafdr`** <code>mafdr(p)</code> | Multiple-testing adjustment (BH by default). | <code>r = mafdr([0.01 0.04]);</code> | <code>[0.02 0.04]</code> |
| **`signrank`** <code>signrank(x[,m])</code> | Signed-rank test. | <code>[p,h] = signrank([1 2 3 4]); r = p;</code> | <code>[0.125]</code> |
| **`ranksum`** <code>ranksum(x,y)</code> | Mann-Whitney rank-sum test. | <code>[p,h] = ranksum([1 2 3],[2 3 4]); r = p;</code> | <code>[0.3687]</code> |
| **`kruskalwallis`** <code>kruskalwallis(x[,groups])</code> | Kruskal-Wallis test. | <code>r = kruskalwallis([1 2 3;2 3 4;3 4 5]);</code> | <code>[0.1385]</code> |
| **`umap`** <code>umap(X,Name,Value,...)</code> | UMAP embedding. | <code>rng(1); r = size(umap([0 0;0.1 0;5 5;5.1 5],'NumNeighbors',2,'NumComponents',2,'NumEpochs',5));</code> | <code>[4 2]</code> |
| **`tsne`** <code>tsne(X,Name,Value,...)</code> | t-SNE embedding. | <code>rng(1); r = size(tsne([0 0;0.1 0;5 5;5.1 5],'Perplexity',2,'NumDimensions',2,'NumIterations',50));</code> | <code>[4 2]</code> |
| **`corr`** <code>corr(X)</code> | Correlation matrix. | <code>r = corr([1 2 3;2 4 6;3 6 9]);</code> | <code>[1 1 1;1 1 1;1 1 1]</code> |
| **`tabulate`** <code>tabulate(x)</code> | Frequency table. | <code>r = tabulate([1 1 2]);</code> | <code>[1 2 66.6667;2 1 33.3333]</code> |
| **`gammaln`** <code>gammaln(x)</code> | Logarithm of the gamma function. | <code>r = gammaln(5);</code> | <code>[3.1781]</code> |
| **`hygecdf`** <code>hygecdf(k,N,K,n)</code> | Hypergeometric cumulative distribution. | <code>r = hygecdf(2,50,5,10);</code> | <code>[0.9517]</code> |
| **`nchoosek`** <code>nchoosek(n,k) &#124; nchoosek(v,k)</code> | Binomial coefficient or combinations. | <code>r = nchoosek(5,2);</code> | <code>[10]</code> |
| **`partialcorr`** <code>partialcorr(X)</code> | Partial correlation matrix. | <code>r = partialcorr([1 2 4;2 5 3;3 1 6;4 7 2;5 4 8]);</code> | <code>[-1 0.8625 0.8718;0.8625 -1 -0.8856;0.8718 -0.8856 -1]</code> |
| **`isoutlier`** <code>isoutlier(x)</code> | Flag outliers. | <code>r = isoutlier([1 2 3 100]);</code> | <code>[0 0 0 1]</code> |
| **`isbetween`** <code>isbetween(x,lo,hi)</code> | True for values inside a closed interval. | <code>r = isbetween([1 5 10],2,8);</code> | <code>[0 1 0]</code> |
| **`zscore`** <code>zscore(X[,flag])</code> | Column-wise standardisation (flag=1 uses the population standard deviation). | <code>r = zscore([1 2;3 4]);</code> | <code>[-0.7071 -0.7071;0.7071 0.7071]</code> |
| **`fitlm`** <code>fitlm(X,y)</code> | Fit a linear model. | <code>m = fitlm((1:10)',2*(1:10)'+1); r = m.Rsquared.Ordinary;</code> | <code>[1]</code> |

## ode

<a id="ode"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`ode45`** <code>ode45(fun,tspan,y0)</code> | Non-stiff ODE solver (Dormand-Prince). | <code>sol = ode45(@(t,y) -y,[0 1],1); r = sol.y(end);</code> | <code>[0.3679]</code> |
| **`ode15s`** <code>ode15s(fun,tspan,y0)</code> | Stiff ODE solver (TR-BDF2). | <code>sol = ode15s(@(t,y) -y,[0 1],1); r = sol.y(end);</code> | <code>[0.3678]</code> |
| **`ode23`** <code>ode23(fun,tspan,y0)</code> | Bogacki-Shampine 3(2) ODE solver. | <code>sol = ode23(@(t,y) -y,[0 1],1); r = sol.y(end);</code> | <code>[0.3679]</code> |
| **`ode23s`** <code>ode23s(fun,tspan,y0)</code> | Stiff ODE solver (TR-BDF2). | <code>sol = ode23s(@(t,y) -100*y,[0 0.1],1); r = sol.y(end);</code> | <code>[4.3890e-5]</code> |
| **`dde23`** <code>dde23(fun,lags,history,tspan)</code> | Delay differential equation solver. | <code>sol = dde23(@(t,y,Z) -Z,1,1,[0 1]); r = sol.y(end);</code> | <code>[-9.7145e-17]</code> |
| **`odeset`** <code>odeset(name,value,...)</code> | Create ODE options. | <code>o = odeset('RelTol',1e-6); r = odeget(o,'RelTol');</code> | <code>[1.0000e-6]</code> |
| **`odeget`** <code>odeget(options,name)</code> | Read an ODE option. | <code>o = odeset('RelTol',1e-6); r = odeget(o,'RelTol');</code> | <code>[1.0000e-6]</code> |
| **`deval`** <code>deval(sol,t)</code> | Evaluate an ODE solution. | <code>sol = ode45(@(t,y) -y,[0 1],1); r = deval(sol,0.5);</code> | <code>[0.6065]</code> |

## corpus classes

<a id="corpus-classes"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`inputParser`** <code>inputParser</code> | Create an input parser. | <code>p = inputParser; r = class(p);</code> | <code>inputParser</code> |
| **`addRequired`** <code>addRequired(p,name[,validator])</code> | Add a required argument. | <code>p = inputParser; addRequired(p,'x'); r = class(p);</code> | <code>inputParser</code> |
| **`addOptional`** <code>addOptional(p,name,default[,validator])</code> | Add an optional positional argument. | <code>p = inputParser; addOptional(p,'y',10); parse(p); r = p.Results.y;</code> | <code>[10]</code> |
| **`addParameter`** <code>addParameter(p,name,default[,validator])</code> | Add a name-value parameter. | <code>p = inputParser; addParameter(p,'Method','fast'); parse(p,'Method','slow'); r = p.Results.Method;</code> | <code>slow</code> |
| **`addParamValue`** <code>addParamValue(p,name,default[,validator])</code> | Legacy alias for addParameter. | <code>p = inputParser; addParamValue(p,'Beta',2); parse(p); r = p.Results.Beta;</code> | <code>[2]</code> |
| **`addparameter`** <code>addparameter(p,name,default[,validator])</code> | Case-insensitive alias for addParameter. | <code>p = inputParser; addparameter(p,'Beta',2); parse(p); r = p.Results.Beta;</code> | <code>[2]</code> |
| **`parse`** <code>parse(p,varargin)</code> | Parse arguments into p.Results. | <code>p = inputParser; addRequired(p,'x'); parse(p,3); r = p.Results.x;</code> | <code>[3]</code> |
| **`string`** <code>string(x)</code> | Create a string scalar/array. | <code>r = isstring(string('abc'));</code> | <code>[1]</code> |
| **`strings`** <code>strings(n) &#124; strings(m,n)</code> | Array of empty strings. | <code>r = size(strings(2,2));</code> | <code>[2 2]</code> |
| **`isstring`** <code>isstring(x)</code> | True for strings. | <code>r = isstring(string('a'));</code> | <code>[1]</code> |
| **`table`** <code>table(var1,var2,...)</code> | Create a table. | <code>T = table([1;2],{'a';'b'},'VariableNames',{'x','name'}); r = T.x;</code> | <code>[1;2]</code> |
| **`table2array`** <code>table2array(T)</code> | Convert table variables to a numeric array. | <code>T = table([1;2],[3;4],'VariableNames',{'x','y'}); r = table2array(T);</code> | <code>[1 3;2 4]</code> |
| **`isKey`** <code>isKey(map,key)</code> | True when a map contains a key. | <code>m = containers.Map({'a'},{1}); r = isKey(m,'a');</code> | <code>[1]</code> |
| **`keys`** <code>keys(map)</code> | Map keys. | <code>m = containers.Map({'a','b'},{1,2}); r = keys(m);</code> | <code>'a'                 <br>   'b'</code> |
| **`values`** <code>values(map)</code> | Map values. | <code>m = containers.Map({'a'},{5}); r = values(m);</code> | <code>[5]</code> |
| **`remove`** <code>remove(map,key)</code> | Remove a map entry. | <code>m = containers.Map({'a','b'},{1,2}); remove(m,'a'); r = m.Count;</code> | <code>[1]</code> |
| **`count`** <code>count(map,key)</code> | Count occurrences of a map key. | <code>m = containers.Map({'a','b'},{1,2}); r = count(m,'a');</code> | <code>[1]</code> |
| **`parpool`** <code>parpool([n])</code> | Start a parallel worker pool. | <code>parpool(2)</code> | <code>parpool(2) creates a pool with two workers.</code> |
| **`maxNumCompThreads`** <code>maxNumCompThreads([n])</code> | Get or set the worker thread count. | <code>r = maxNumCompThreads;</code> | <code>[16]</code> |
| **`gpuArray`** <code>gpuArray(A)</code> | Move an array to the GPU (or CPU fallback). | <code>r = gather(gpuArray([1 2]));</code> | <code>[1 2]</code> |
| **`gather`** <code>gather(A)</code> | Bring a GPU array back to the CPU. | <code>r = gather(gpuArray([1 2]));</code> | <code>[1 2]</code> |
| **`canUseGPU`** <code>canUseGPU</code> | True when a GPU backend is available. | <code>r = canUseGPU();</code> | <code>[0]</code> |
| **`existsOnGPU`** <code>existsOnGPU(A)</code> | True when an array lives on the GPU. | <code>r = existsOnGPU(gpuArray(1));</code> | <code>[0]</code> |
| **`gpuDevice`** <code>gpuDevice</code> | GPU device information. | <code>gpuDevice()</code> | <code>gpuDevice returns a struct describing the selected device.</code> |
| **`parfeval`** <code>parfeval(fun,nout,varargin)</code> | Run a function asynchronously. | <code>parfeval(@(x)x+1, 1, 2)</code> | <code>parfeval(@(x)x+1,1,2) returns a future; fetchOutputs reads it.</code> |
| **`fetchOutputs`** <code>fetchOutputs(future)</code> | Read asynchronous results. | <code>fetchOutputs(f)</code> | <code>fetchOutputs(f) returns the value produced by parfeval.</code> |
| **`onCleanup`** <code>onCleanup(fun)</code> | Run cleanup code when an object is destroyed. | <code>r = class(onCleanup(@() 1));</code> | <code>onCleanup</code> |
| **`categorical`** <code>categorical(values)</code> | Create a categorical array. | <code>c = categorical({'a','b','a'}); r = categories(c);</code> | <code>'a'                 <br>   'b'</code> |
| **`reordercats`** <code>reordercats(c,order)</code> | Compatibility pass-through; this build derives categories from the values. | <code>c = reordercats(categorical({'a','b'}),{'b','a'}); r = categories(c);</code> | <code>'a'                 <br>   'b'</code> |
| **`categories`** <code>categories(c)</code> | Category levels. | <code>r = categories(categorical({'a','b','a'}));</code> | <code>'a'                 <br>   'b'</code> |
| **`countcats`** <code>countcats(c)</code> | Count values in each category. | <code>r = countcats(categorical({'a','b','a'}));</code> | <code>[2;1]</code> |
| **`addcats`** <code>addcats(c,newcats)</code> | Compatibility pass-through; this build derives categories from the values. | <code>c = addcats(categorical({'a'}),{'b'}); r = categories(c);</code> | <code>'a'</code> |
| **`removecats`** <code>removecats(c,names)</code> | Compatibility pass-through; this build derives categories from the values. | <code>c = removecats(categorical({'a','b'}),{'b'}); r = categories(c);</code> | <code>'a'                 <br>   'b'</code> |
| **`mergecats`** <code>mergecats(c,names,newName)</code> | Compatibility pass-through; this build derives categories from the values. | <code>c = mergecats(categorical({'a','b'}),{'a','b'},'ab'); r = categories(c);</code> | <code>'a'                 <br>   'b'</code> |
| **`renamecats`** <code>renamecats(c,names)</code> | Compatibility pass-through; this build derives categories from the values. | <code>c = renamecats(categorical({'a'}),{'b'}); r = categories(c);</code> | <code>'a'</code> |
| **`isundefined`** <code>isundefined(c)</code> | True for undefined categorical entries. | <code>r = isundefined(categorical({'a','b'}));</code> | <code>[0;0]</code> |
| **`isdatetime`** <code>isdatetime(x)</code> | Compatibility stub; currently returns 0 (datetime is stored as text). | <code>r = isdatetime(datetime(2024,1,1));</code> | <code>[0]</code> |
| **`isduration`** <code>isduration(x)</code> | Compatibility stub; currently returns 0 (durations are numeric). | <code>r = isduration(duration(1,0,0));</code> | <code>[0]</code> |
| **`istimetable`** <code>istimetable(x)</code> | Compatibility stub; currently returns 0 (no timetable type). | <code>r = istimetable(1);</code> | <code>[0]</code> |
| **`datetime`** <code>datetime('now'[,'Format',fmt])</code> | Formatted date text (datetime('now',...)). | <code>r = class(datetime('now'));</code> | <code>char</code> |
| **`duration`** <code>duration(x)</code> | Compatibility helper; returns its first numeric argument. | <code>r = class(duration(1,2,3));</code> | <code>double</code> |
| **`seconds`** <code>seconds(x)</code> | Duration in seconds; returns its numeric input. | <code>r = seconds(5);</code> | <code>[5]</code> |

## anova

<a id="anova"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`anova1`** <code>anova1(X[,groups])</code> | One-way ANOVA. | <code>[p,tbl,st] = anova1([1 2;2 4;3 6]); r = p;</code> | <code>[0.1963]</code> |
| **`anova2`** <code>anova2(X,reps)</code> | Two-way ANOVA. | <code>r = anova2([1 2;3 4;5 6;7 8],2);</code> | <code>[0.3739]</code> |
| **`multcompare`** <code>multcompare(stats)</code> | Multiple pairwise comparisons. | <code>[p,tbl,st] = anova1([1 2;2 4;3 6]); c = multcompare(st); r = size(c);</code> | <code>[1 6]</code> |
| **`friedman`** <code>friedman(X[,reps])</code> | Friedman two-way rank test. | <code>r = friedman([1 2 3;2 3 1;3 1 2]);</code> | <code>[1]</code> |

## xlsx/table

<a id="xlsx-table"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`xlsread`** <code>xlsread(file)</code> | Read numeric data from an XLSX file. | <code>xlsread('data.xlsx')</code> | <code>xlsread reads a workbook into numeric, text and raw outputs.</code> |
| **`xlswrite`** <code>xlswrite(file,A[,sheet,range])</code> | Write an array to an XLSX file. | <code>xlswrite('out.xlsx', [1 2;3 4])</code> | <code>xlswrite writes a workbook sheet or range.</code> |
| **`readcell`** <code>readcell(file)</code> | Read spreadsheet/text data into a cell array. | <code>readcell('data.xlsx')</code> | <code>readcell returns cell contents from a workbook or text file.</code> |
| **`writecell`** <code>writecell(C,file)</code> | Write a cell array to a spreadsheet/text file. | <code>writecell({1,'a'}, 'out.xlsx')</code> | <code>writecell writes cells to a workbook or text file.</code> |
| **`readtable`** <code>readtable(file)</code> | Read a table from a spreadsheet or delimited text. | <code>readtable('data.csv')</code> | <code>readtable returns a table object with variable names and rows.</code> |
| **`writetable`** <code>writetable(T,file)</code> | Write a table to a spreadsheet or delimited text. | <code>writetable(T, 'out.csv')</code> | <code>writetable writes a table object to a file.</code> |
| **`height`** <code>height(T)</code> | Number of table rows. | <code>T = table([1;2],[3;4]); r = height(T);</code> | <code>[2]</code> |
| **`width`** <code>width(T)</code> | Number of table variables. | <code>T = table([1;2],[3;4]); r = width(T);</code> | <code>[2]</code> |
| **`istable`** <code>istable(x)</code> | True for table objects. | <code>r = istable(table([1;2]));</code> | <code>[1]</code> |
| **`rows`** <code>rows(T)</code> | Number of rows in a table, cell or matrix. | <code>r = rows(table([1;2]));</code> | <code>[2]</code> |
| **`array2table`** <code>array2table(A[,Name,Value])</code> | Convert a matrix to a table. | <code>T = array2table([1 2;3 4],'VariableNames',{'x','y'}); r = T.x;</code> | <code>[1;3]</code> |
| **`ismissing`** <code>ismissing(x)</code> | True for missing values. | <code>r = ismissing([1 NaN 3]);</code> | <code>[0 1 0]</code> |
| **`table2cell`** <code>table2cell(T)</code> | Convert a table to a cell array. | <code>T = table([1;2],{'a';'b'}); r = table2cell(T);</code> | <code>[1]                 'a'                 <br>   [2]                 'b'</code> |
| **`cell2table`** <code>cell2table(C[,Name,Value])</code> | Convert a cell array to a table. | <code>r = height(cell2table({1,'a';2,'b'},'VariableNames',{'x','name'}));</code> | <code>[2]</code> |
| **`struct2table`** <code>struct2table(S)</code> | Convert a struct to a table. | <code>r = height(struct2table(struct('x',[1;2],'y',[3;4])));</code> | <code>[2]</code> |
| **`struct2cell`** <code>struct2cell(S)</code> | Convert a struct to a cell array. | <code>r = struct2cell(struct('x',1,'y',2));</code> | <code>[1]                 <br>   [2]</code> |
| **`cell2struct`** <code>cell2struct(C,fields,dim)</code> | Convert a cell array to a struct. | <code>r = cell2struct({'a',1;'b',2},{'name','v'},2);</code> | <code>struct with fields: name, v</code> |
| **`addvars`** <code>addvars(T,var[,Name,Value])</code> | Add variables to a table. | <code>T = table([1;2]); T = addvars(T,[3;4],'NewVariableNames',{'y'}); r = width(T);</code> | <code>[2]</code> |
| **`setvartype`** <code>setvartype(T,vars,type)</code> | Set table variable types. | <code>T = table([1;2]); T = setvartype(T,{'Var1'},'char'); r = class(T.Var1);</code> | <code>cell</code> |
| **`convertvars`** <code>convertvars(T,vars,type)</code> | Convert table variable types. | <code>T = table([1;2]); T = convertvars(T,{'Var1'},'char'); r = class(T.Var1);</code> | <code>cell</code> |
| **`removevars`** <code>removevars(T,vars)</code> | Remove table variables. | <code>T = table([1;2],[3;4]); T = removevars(T,{'Var2'}); r = width(T);</code> | <code>[1]</code> |
| **`renamevars`** <code>renamevars(T,vars,newNames)</code> | Rename table variables. | <code>T = table([1;2]); T = renamevars(T,{'Var1'},{'x'}); r = T.Properties.VariableNames;</code> | <code>'x'</code> |
| **`summary`** <code>summary(T)</code> | Summarize table variables. | <code>T = table([1;2],[3;4]); s = summary(T); r = class(s);</code> | <code>struct</code> |
| **`findgroups`** <code>findgroups(vars)</code> | Group identifiers for table/cell data. | <code>T = table({'a';'b';'a'},[1;2;3]); [g,id] = findgroups(T.Var1); r = numel(id);</code> | <code>[2]</code> |
| **`splitapply`** <code>splitapply(fun,X,G)</code> | Apply a function to groups. | <code>r = splitapply(@mean,[1;2;3;4],[1;1;2;2]);</code> | <code>[1.5;3.5]</code> |
| **`varfun`** <code>varfun(fun,T)</code> | Apply a function to table variables. | <code>T = table([1;2],[3;4]); r = varfun(@mean,T);</code> | <code>table object</code> |
| **`rowfun`** <code>rowfun(fun,T)</code> | Apply a function to table rows. | <code>T = table([1;2],[3;4]); r = rowfun(@sum,T);</code> | <code>[4;6]</code> |
| **`join`** <code>join(A,B[,Name,Value])</code> | Join two tables on key variables. | <code>join(A, B, 'Keys', {'k'})</code> | <code>join(A,B,'Keys',{'k'}) returns the joined table.</code> |
| **`innerjoin`** <code>innerjoin(A,B)</code> | Inner join of two tables. | <code>innerjoin(A, B)</code> | <code>innerjoin(A,B) returns matching rows only.</code> |
| **`outerjoin`** <code>outerjoin(A,B)</code> | Outer join of two tables. | <code>outerjoin(A, B)</code> | <code>outerjoin(A,B) keeps unmatched rows with missing values.</code> |
| **`groupsummary`** <code>groupsummary(T,groupvars,method)</code> | Grouped summary statistics. | <code>T = table({'a';'b';'a'},[1;2;3],'VariableNames',{'k','x'}); r = height(groupsummary(T,'k','mean'));</code> | <code>[2]</code> |
| **`stack`** <code>stack(T,vars)</code> | Stack table variables into rows. | <code>stack(T, {'x','y'})</code> | <code>stack(T,vars) returns a table with one stacked value column.</code> |
| **`unstack`** <code>unstack(T,var,ivar)</code> | Unstack a table variable into columns. | <code>unstack(T, 'x', 'k')</code> | <code>unstack(T,var,ivar) spreads a variable across columns.</code> |
| **`fillmissing`** <code>fillmissing(T&#124;A,method)</code> | Fill missing values. | <code>r = fillmissing([1 NaN 3],'previous');</code> | <code>[1 1 3]</code> |
| **`rmmissing`** <code>rmmissing(T&#124;A)</code> | Remove rows with missing values. | <code>r = rmmissing([1 NaN; 3 4]);</code> | <code>[3 4]</code> |
| **`standardizeMissing`** <code>standardizeMissing(A,indicator)</code> | Replace indicator values with missing. | <code>r = standardizeMissing([1 2 3],2);</code> | <code>[1 NaN 3]</code> |
| **`fileread`** <code>fileread(file)</code> | Read a text file into a string. | <code>fileread('notes.txt')</code> | <code>fileread returns the file contents as a character vector.</code> |
| **`isfolder`** <code>isfolder(path)</code> | True when a path is a folder. | <code>r = isfolder(tempdir);</code> | <code>[1]</code> |
| **`contains`** <code>contains(str,pattern)</code> | True when text contains a pattern. | <code>r = contains('alpha','ph');</code> | <code>[1]</code> |
| **`startswith`** <code>startswith(str,pattern)</code> | True when text starts with a pattern (lowercase alias). | <code>r = startswith('alpha','al');</code> | <code>[1]</code> |
| **`endswith`** <code>endswith(str,pattern)</code> | True when text ends with a pattern (lowercase alias). | <code>r = endswith('alpha','ha');</code> | <code>[1]</code> |
| **`startsWith`** <code>startsWith(str,pattern)</code> | True when text starts with a pattern. | <code>r = startsWith('alpha','al');</code> | <code>[1]</code> |
| **`endsWith`** <code>endsWith(str,pattern)</code> | True when text ends with a pattern. | <code>r = endsWith('alpha','ha');</code> | <code>[1]</code> |
| **`strsplit`** <code>strsplit(str,delim)</code> | Split text into a cellstr. | <code>r = strsplit('a,b,c',',');</code> | <code>'a'                 'b'                 'c'</code> |
| **`strjoin`** <code>strjoin(C,delim)</code> | Join a cellstr into one string. | <code>r = strjoin({'a','b'},'-');</code> | <code>a-b</code> |
| **`strlength`** <code>strlength(str)</code> | Number of characters. | <code>r = strlength('alpha');</code> | <code>[5]</code> |
| **`erase`** <code>erase(str,pattern)</code> | Remove occurrences of a pattern. | <code>r = erase('a-b-c','-');</code> | <code>abc</code> |
| **`replace`** <code>replace(str,old,new)</code> | Replace occurrences of a pattern. | <code>r = replace('abc','b','X');</code> | <code>aXc</code> |
| **`findstr`** <code>findstr(str,pattern)</code> | Find a substring (legacy). | <code>r = findstr('ababa','aba');</code> | <code>[1 3]</code> |
| **`jsondecode`** <code>jsondecode(text)</code> | Decode JSON text. | <code>r = jsondecode('{"a":1}').a;</code> | <code>[1]</code> |
| **`jsonencode`** <code>jsonencode(value)</code> | Encode a value as JSON. | <code>r = jsonencode(struct('a',1));</code> | <code>{"a":1.0}</code> |
| **`importdata`** <code>importdata(file)</code> | Import numeric or text data. | <code>importdata('data.csv')</code> | <code>importdata reads numbers, text or a struct from a file.</code> |
| **`detectImportOptions`** <code>detectImportOptions(file)</code> | Infer import options from a file. | <code>detectImportOptions('data.csv')</code> | <code>detectImportOptions returns an options object for readtable.</code> |
| **`webread`** <code>webread(url)</code> | Read content from a URL. | <code>webread('https://example.com')</code> | <code>webread downloads and decodes a URL response.</code> |
| **`urlread`** <code>urlread(url)</code> | Read a URL (legacy). | <code>urlread('https://example.com')</code> | <code>urlread returns the response text and status.</code> |
| **`web`** <code>web(url)</code> | Open a URL in the browser. | <code>web('https://example.com')</code> | <code>web opens the system browser.</code> |
| **`winopen`** <code>winopen(path)</code> | Open a file with the system application. | <code>winopen('report.pdf')</code> | <code>winopen opens a file in its associated application.</code> |
| **`uigetdir`** <code>uigetdir([start,title])</code> | Prompt for a folder. | <code>uigetdir(pwd, 'Pick a folder')</code> | <code>uigetdir opens a folder-selection dialog.</code> |
| **`questdlg`** <code>questdlg(question,...)</code> | Prompt with a modal question dialog. | <code>questdlg('Continue?')</code> | <code>questdlg returns the selected button label.</code> |
| **`uimenu`** <code>uimenu(...)</code> | Create a menu item. | <code>uimenu('Label', 'Tools')</code> | <code>uimenu creates a menu attached to a figure.</code> |
| **`actxserver`** <code>actxserver(progid)</code> | Start a COM automation server (Windows). | <code>actxserver('Excel.Application')</code> | <code>actxserver('Excel.Application') starts the COM server.</code> |
| **`extractBefore`** <code>extractBefore(str,pat)</code> | Text before a marker. | <code>r = extractBefore('hello.world','.');</code> | <code>hello</code> |
| **`extractAfter`** <code>extractAfter(str,pat)</code> | Text after a marker. | <code>r = extractAfter('hello.world','.');</code> | <code>world</code> |
| **`insertBefore`** <code>insertBefore(str,pat,new)</code> | Insert text before a marker. | <code>r = insertBefore('abc','b','X');</code> | <code>aXbc</code> |
| **`isstrprop`** <code>isstrprop(str,prop)</code> | Character-class predicate. | <code>r = isstrprop('a1 ','alpha');</code> | <code>[1 0 0]</code> |
| **`isletter`** <code>isletter(str)</code> | True for letters. | <code>r = isletter('a1B');</code> | <code>[1 0 1]</code> |
| **`compose`** <code>compose(format,varargin)</code> | Format text into a cellstr. | <code>r = compose('%d',5);</code> | <code>5</code> |
| **`splitlines`** <code>splitlines(str)</code> | Split text into lines. | <code>r = splitlines(sprintf('one\ntwo\n'));</code> | <code>'one'               <br>   'two'</code> |
| **`readlines`** <code>readlines(file)</code> | Read a file as lines. | <code>readlines('notes.txt')</code> | <code>readlines returns a string array of file lines.</code> |
| **`rethrow`** <code>rethrow(e)</code> | Rethrow a caught error. | <code>rethrow(e)</code> | <code>rethrow(e) re-raises the exception in a catch block.</code> |
| **`verLessThan`** <code>verLessThan(toolbox,version)</code> | True when a toolbox is older than a version. | <code>verLessThan('matlab', '9.0')</code> | <code>verLessThan compares the toolbox version.</code> |
| **`inputname`** <code>inputname(n)</code> | Name of the n-th caller argument. | <code>inputname(1)</code> | <code>inputname returns the caller variable name inside a function.</code> |
| **`validateattributes`** <code>validateattributes(A,classes,attributes)</code> | Validate an array's class and attributes. | <code>validateattributes(3,{'numeric'},{'scalar'}); r = 'valid';</code> | <code>valid</code> |
| **`validatestring`** <code>validatestring(str,valid)</code> | Match a string against valid choices. | <code>r = validatestring('fas',{'fast','slow'});</code> | <code>fast</code> |
| **`mustBeNumeric`** <code>mustBeNumeric(x)</code> | Error unless x is numeric. | <code>mustBeNumeric(1); r = 'numeric';</code> | <code>numeric</code> |
| **`mustBeNonempty`** <code>mustBeNonempty(x)</code> | Error unless x is nonempty. | <code>mustBeNonempty(1); r = 'nonempty';</code> | <code>nonempty</code> |
| **`mustBePositive`** <code>mustBePositive(x)</code> | Error unless x is positive. | <code>mustBePositive(1); r = 'positive';</code> | <code>positive</code> |
| **`mustBeInteger`** <code>mustBeInteger(x)</code> | Error unless x is integer-valued. | <code>mustBeInteger(2); r = 'integer';</code> | <code>integer</code> |
| **`mustBeMember`** <code>mustBeMember(x,valid)</code> | Error unless x is a member of a set. | <code>mustBeMember(2,[1 2 3]); r = 'member';</code> | <code>member</code> |
| **`mustBeText`** <code>mustBeText(x)</code> | Error unless x is text. | <code>mustBeText('a'); r = 'text';</code> | <code>text</code> |
| **`mustBeA`** <code>mustBeA(x,classes)</code> | Error unless x has one of the classes. | <code>mustBeA(1,{'double','single'}); r = 'double';</code> | <code>double</code> |
| **`mustBeScalarOrEmpty`** <code>mustBeScalarOrEmpty(x)</code> | Error unless x is scalar or empty. | <code>mustBeScalarOrEmpty(1); r = 'scalar';</code> | <code>scalar</code> |
| **`mustBeFinite`** <code>mustBeFinite(x)</code> | Error unless x is finite. | <code>mustBeFinite(1); r = 'finite';</code> | <code>finite</code> |
| **`mustBeReal`** <code>mustBeReal(x)</code> | Error unless x is real. | <code>mustBeReal(1); r = 'real';</code> | <code>real</code> |

## hdf5/netcdf

<a id="hdf5-netcdf"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`h5read`** <code>h5read(file,dataset)</code> | Read an HDF5 dataset. | <code>h5read('data.h5', '/dataset')</code> | <code>h5read returns a numeric dataset from an HDF5 file.</code> |
| **`h5write`** <code>h5write(file,dataset,data)</code> | Write an HDF5 dataset. | <code>h5write('data.h5', '/dataset', [1 2 3])</code> | <code>h5write stores a numeric dataset in an HDF5 file.</code> |
| **`h5create`** <code>h5create(file,dataset,size)</code> | Create an HDF5 dataset. | <code>h5create('data.h5', '/dataset', [3 1])</code> | <code>h5create defines a dataset before writing.</code> |
| **`h5disp`** <code>h5disp(file)</code> | Display HDF5 file metadata. | <code>h5disp('data.h5')</code> | <code>h5disp prints datasets, dimensions and attributes.</code> |
| **`ncread`** <code>ncread(file,var)</code> | Read a NetCDF variable. | <code>ncread('data.nc', 'temp')</code> | <code>ncread returns a variable from a NetCDF file.</code> |
| **`ncwrite`** <code>ncwrite(file,var,data)</code> | Write a NetCDF variable. | <code>ncwrite('data.nc', 'temp', [1 2 3])</code> | <code>ncwrite writes a classic CDF-1 variable.</code> |
| **`ncinfo`** <code>ncinfo(file)</code> | NetCDF file metadata. | <code>ncinfo('data.nc')</code> | <code>ncinfo returns dimensions, variables and attributes.</code> |
| **`ncdisp`** <code>ncdisp(file)</code> | Display NetCDF metadata. | <code>ncdisp('data.nc')</code> | <code>ncdisp prints the NetCDF header.</code> |

## image processing

<a id="image-processing"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`rgb2gray`** <code>rgb2gray(RGB)</code> | Convert RGB to grayscale. | <code>r = rgb2gray(reshape(1:12,2,2,3));</code> | <code>[4.2603 6.2603;5.2603 7.2603]</code> |
| **`imresize`** <code>imresize(A,scale&#124;size)</code> | Resize an image. | <code>r = size(imresize(ones(2,2),2));</code> | <code>[4 4]</code> |
| **`imfilter`** <code>imfilter(A,h)</code> | Correlate an image with a filter. | <code>r = size(imfilter(ones(3,3),[1 1;1 1]));</code> | <code>[3 3]</code> |
| **`medfilt2`** <code>medfilt2(A[,m,n])</code> | 2-D median filter. | <code>r = size(medfilt2(ones(3,3)));</code> | <code>[3 3]</code> |
| **`edge`** <code>edge(A,method)</code> | Detect image edges. | <code>r = size(edge([0 0 0;1 1 1;0 0 0],'sobel'));</code> | <code>[3 3]</code> |
| **`bwlabel`** <code>bwlabel(BW[,conn])</code> | Label connected components. | <code>[L,n] = bwlabel([1 0;0 1]); r = n;</code> | <code>[1]</code> |
| **`regionprops`** <code>regionprops(L,props)</code> | Measure connected components. | <code>r = regionprops([1 0;0 1],'Area');</code> | <code>struct with fields: Area</code> |
| **`graythresh`** <code>graythresh(A)</code> | Otsu threshold. | <code>r = graythresh([0 0 1 1]);</code> | <code>[0.0039]</code> |
| **`imbinarize`** <code>imbinarize(A[,method])</code> | Convert an image to binary. | <code>r = imbinarize([0 0.2 0.8 1]);</code> | <code>[0 0 1 1]</code> |
| **`imdilate`** <code>imdilate(A,se)</code> | Morphological dilation. | <code>r = imdilate([0 1 0;0 0 0],[1 1;1 1]);</code> | <code>[0 1 1;0 1 1]</code> |
| **`imerode`** <code>imerode(A,se)</code> | Morphological erosion. | <code>r = imerode([1 1 1;1 1 1],[1 1;1 1]);</code> | <code>[1 1 1;1 1 1]</code> |
| **`imopen`** <code>imopen(A,se)</code> | Morphological opening. | <code>r = imopen([0 1 0;0 0 0],[1 1;1 1]);</code> | <code>[0 0 0;0 0 0]</code> |
| **`imclose`** <code>imclose(A,se)</code> | Morphological closing. | <code>r = imclose([0 1 0;0 0 0],[1 1;1 1]);</code> | <code>[0 0 1;0 0 1]</code> |
| **`imrotate`** <code>imrotate(A,angle)</code> | Rotate an image. | <code>r = size(imrotate(ones(2,2),90));</code> | <code>[2 2]</code> |
| **`imcrop`** <code>imcrop(A,rect)</code> | Crop an image. | <code>r = size(imcrop(ones(4,4),[1 1 2 2]));</code> | <code>[2 2]</code> |
| **`strel`** <code>strel(shape,params)</code> | Create a morphological structuring element. | <code>r = strel('square',2);</code> | <code>[1 1;1 1]</code> |

## function handles

<a id="function-handles"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`feval`** <code>feval(fun,varargin)</code> | Evaluate a function handle or name. | <code>r = feval(@(x) x+1,2);</code> | <code>[3]</code> |
| **`builtin`** <code>builtin(name,varargin)</code> | Call a builtin directly, bypassing overloads. | <code>r = builtin('abs',-3);</code> | <code>[3]</code> |
| **`func2str`** <code>func2str(fun)</code> | Convert a function handle to text. | <code>r = func2str(@sin);</code> | <code>@sin</code> |
| **`str2func`** <code>str2func(name)</code> | Create a function handle from a name. | <code>f = str2func('sin'); r = f(0);</code> | <code>[0]</code> |
| **`functions`** <code>functions(fun)</code> | Function-handle metadata. | <code>r = functions(@sin).function;</code> | <code>sin</code> |
| **`arrayfun`** <code>arrayfun(fun,A)</code> | Apply a function to every array element. | <code>r = arrayfun(@(x) x.^2,[1 2 3]);</code> | <code>[1 4 9]</code> |
| **`cellfun`** <code>cellfun(fun,C)</code> | Apply a function to every cell. | <code>r = cellfun(@(x) x+1,{1,2});</code> | <code>[2 3]</code> |
| **`inline`** <code>inline(expr)</code> | Create an inline function object. | <code>f = inline('x+1'); r = f(2);</code> | <code>[3]</code> |

## objects

<a id="objects"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`isobject`** <code>isobject(x)</code> | True for class objects. | <code>r = isobject(1);</code> | <code>[0]</code> |
| **`methods`** <code>methods(obj)</code> | Method names for classdef objects; struct-backed built-ins may return an empty list. | <code>r = methods(tf(1,[1 1]));</code> | <code>[]</code> |
| **`properties`** <code>properties(obj)</code> | Property names of a class or object. | <code>r = properties(tf(1,[1 1]));</code> | <code>'Num'               <br>   'Den'               <br>   'Variable'          <br>   'Ts'</code> |
| **`events`** <code>events(obj)</code> | Event names of a class. | <code>r = events(tf(1,[1 1]));</code> | <code>[]</code> |
| **`metaclass`** <code>metaclass(obj)</code> | Class metadata object. | <code>r = class(metaclass(tf(1,[1 1])));</code> | <code>struct</code> |
| **`ismethod`** <code>ismethod(obj,name)</code> | True when a class has a method. | <code>r = ismethod(tf(1,[1 1]),'tfdata');</code> | <code>[0]</code> |
| **`isprop`** <code>isprop(obj,name)</code> | True when an object has a property. | <code>r = isprop(tf(1,[1 1]),'Num');</code> | <code>[1]</code> |
| **`addlistener`** <code>addlistener(obj,event,fun)</code> | Attach an event listener. | <code>addlistener(obj, 'Changed', @(src,ev) disp('changed'))</code> | <code>addlistener attaches a callback to a handle-class event.</code> |
| **`notify`** <code>notify(obj,event)</code> | Trigger a handle-class event. | <code>notify(obj, 'Changed')</code> | <code>notify triggers listeners registered with addlistener.</code> |
| **`addprop`** <code>addprop(obj,name)</code> | Add a dynamic property. | <code>addprop(obj, 'NewProp')</code> | <code>addprop adds a property to a dynamicprops object.</code> |
| **`dynamicprops`** <code>dynamicprops(obj)</code> | List dynamic property names. | <code>dynamicprops(obj)</code> | <code>dynamicprops returns the dynamic property names of an object.</code> |
| **`removeprop`** <code>removeprop(obj,name)</code> | Remove a dynamic property. | <code>removeprop(obj, 'NewProp')</code> | <code>removeprop removes a dynamic property.</code> |

## debugging

<a id="debugging"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`dbstop`** <code>dbstop in file at line</code> | Set a debugger breakpoint. | <code>dbstop in myfun at 10</code> | <code>dbstop sets a breakpoint in a function or script.</code> |
| **`dbclear`** <code>dbclear ...</code> | Clear debugger breakpoints. | <code>dbclear all</code> | <code>dbclear removes breakpoints.</code> |
| **`dbcont`** <code>dbcont</code> | Resume execution after a breakpoint. | <code>dbcont</code> | <code>dbcont continues from the debugger prompt.</code> |
| **`dbstatus`** <code>dbstatus</code> | List active breakpoints. | <code>dbstatus</code> | <code>dbstatus lists breakpoints.</code> |
| **`dbstep`** <code>dbstep</code> | Execute one debugger step. | <code>dbstep</code> | <code>dbstep advances one statement.</code> |
| **`dbquit`** <code>dbquit</code> | Quit debug mode. | <code>dbquit</code> | <code>dbquit exits the debugger.</code> |
| **`run`** <code>run(file)</code> | Run a script in the current workspace. | <code>run('script.m')</code> | <code>run executes a .m script and shares its workspace.</code> |

## strings

<a id="strings"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`num2str`** <code>num2str(x[,format])</code> | Convert numbers to text. | <code>r = num2str(pi);</code> | <code>3.1416</code> |
| **`int2str`** <code>int2str(x)</code> | Convert an integer to text. | <code>r = int2str(5);</code> | <code>5</code> |
| **`mat2str`** <code>mat2str(A[,n])</code> | Convert a matrix to a string literal. | <code>r = mat2str([1 2;3 4]);</code> | <code>[1 2;3 4]</code> |
| **`str2double`** <code>str2double(str)</code> | Parse text as a double. | <code>r = str2double('3.5');</code> | <code>[3.5]</code> |
| **`str2num`** <code>str2num(str)</code> | Parse a numeric scalar string (array syntax is not evaluated in this build). | <code>r = str2num('3.5');</code> | <code>[3.5]</code> |
| **`str2mat`** <code>str2mat(s1,s2,...)</code> | Build a character matrix from strings. | <code>r = str2mat('ab','cd');</code> | <code>'ab'                <br>   'cd'</code> |
| **`strcmp`** <code>strcmp(a,b)</code> | Compare strings exactly. | <code>r = strcmp('abc','abc');</code> | <code>[1]</code> |
| **`strcmpi`** <code>strcmpi(a,b)</code> | Compare strings ignoring case. | <code>r = strcmpi('ABC','abc');</code> | <code>[1]</code> |
| **`strncmp`** <code>strncmp(a,b,n)</code> | Compare the first n characters. | <code>r = strncmp('abc','abd',2);</code> | <code>[1]</code> |
| **`strncmpi`** <code>strncmpi(a,b,n)</code> | Compare the first n characters ignoring case. | <code>r = strncmpi('ABC','abd',2);</code> | <code>[1]</code> |
| **`strmatch`** <code>strmatch(pattern,strings)</code> | Find strings with a prefix. | <code>r = strmatch('ab',{'ab','abc','x'});</code> | <code>[1;2]</code> |
| **`strcat`** <code>strcat(s1,s2,...)</code> | Concatenate strings. | <code>r = strcat('a','b');</code> | <code>ab</code> |
| **`upper`** <code>upper(str)</code> | Convert to uppercase. | <code>r = upper('abc');</code> | <code>ABC</code> |
| **`lower`** <code>lower(str)</code> | Convert to lowercase. | <code>r = lower('ABC');</code> | <code>abc</code> |
| **`strtrim`** <code>strtrim(str)</code> | Remove leading and trailing whitespace. | <code>r = strtrim('  a  ');</code> | <code>a</code> |
| **`strrep`** <code>strrep(str,old,new)</code> | Replace substrings. | <code>r = strrep('abc','b','X');</code> | <code>aXc</code> |
| **`strfind`** <code>strfind(str,pattern)</code> | Find substring positions. | <code>r = strfind('ababa','aba');</code> | <code>[1]</code> |
| **`strtok`** <code>strtok(str[,delim])</code> | First whitespace-delimited token. | <code>r = strtok('a b');</code> | <code>a</code> |
| **`blanks`** <code>blanks(n)</code> | String of n spaces. | <code>r = ['&lt;' blanks(3) '&gt;'];</code> | <code>&lt;   &gt;</code> |
| **`deblank`** <code>deblank(str)</code> | Remove trailing whitespace. | <code>r = deblank('a  ');</code> | <code>a</code> |
| **`double`** <code>double(x)</code> | Convert to double. | <code>r = double('A');</code> | <code>[65]</code> |
| **`logical`** <code>logical(x)</code> | Convert to logical. | <code>r = logical([1 0 2]);</code> | <code>[1 0 2]</code> |
| **`char`** <code>char(x)</code> | Convert to a character array. | <code>r = char(65);</code> | <code>A</code> |
| **`regexp`** <code>regexp(str,pat[,out])</code> | Regular-expression match. | <code>r = regexp('abc','b');</code> | <code>[2]</code> |
| **`regexpi`** <code>regexpi(str,pat[,out])</code> | Case-insensitive regular-expression match. | <code>r = regexpi('ABC','b');</code> | <code>[2]</code> |
| **`regexprep`** <code>regexprep(str,pat,rep)</code> | Regular-expression replacement. | <code>r = regexprep('abc','b','X');</code> | <code>aXc</code> |

## text scan

<a id="text-scan"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`fscanf`** <code>fscanf(fid,format[,size])</code> | Read formatted data from a file. | <code>r = sscanf('1 2 3','%f')';</code> | <code>[1 2 3]</code> |
| **`sscanf`** <code>sscanf(str,format[,size])</code> | Read formatted data from a string. | <code>r = sscanf('1 2 3','%f')';</code> | <code>[1 2 3]</code> |
| **`textscan`** <code>textscan(fid,format)</code> | Read columnar text data. | <code>textscan(fid, '%f %f')</code> | <code>textscan returns a cell array of columns from a file.</code> |
| **`strread`** <code>strread(str,format)</code> | Read formatted data from a string (legacy). | <code>x = strread('1 2','%f'); r = x{1};</code> | <code>[1;2]</code> |
| **`dlmread`** <code>dlmread(file[,delim])</code> | Read a delimited numeric file. | <code>dlmread('data.txt')</code> | <code>dlmread reads a numeric matrix from a delimited text file.</code> |
| **`dlmwrite`** <code>dlmwrite(file,A[,delim])</code> | Write a delimited numeric file. | <code>dlmwrite('out.txt', [1 2;3 4])</code> | <code>dlmwrite writes a matrix as delimited text.</code> |
| **`csvwrite`** <code>csvwrite(file,A)</code> | Write a CSV file. | <code>csvwrite('out.csv', [1 2;3 4])</code> | <code>csvwrite writes a matrix as comma-separated text.</code> |
| **`readmatrix`** <code>readmatrix(file)</code> | Read a numeric matrix from a file. | <code>readmatrix('data.csv')</code> | <code>readmatrix detects the format from the extension.</code> |
| **`writematrix`** <code>writematrix(A,file)</code> | Write a numeric matrix to a file. | <code>writematrix([1 2;3 4], 'out.csv')</code> | <code>writematrix writes a matrix using the file extension.</code> |

## file io

<a id="file-io"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`fopen`** <code>fopen(file[,mode])</code> | Open a file. | <code>fopen('notes.txt', 'r')</code> | <code>fopen returns a file identifier and message.</code> |
| **`fclose`** <code>fclose(fid)</code> | Close a file. | <code>fclose(fid)</code> | <code>fclose returns 0 on success.</code> |
| **`fread`** <code>fread(fid[,size,precision])</code> | Read binary data. | <code>fread(fid, 3)</code> | <code>fread returns the requested binary values.</code> |
| **`fwrite`** <code>fwrite(fid,A[,precision])</code> | Write binary data. | <code>fwrite(fid, [1 2 3])</code> | <code>fwrite returns the number of values written.</code> |
| **`fseek`** <code>fseek(fid,offset,origin)</code> | Set the file position. | <code>fseek(fid, 0, 'bof')</code> | <code>fseek returns 0 on success.</code> |
| **`ftell`** <code>ftell(fid)</code> | Current file position. | <code>ftell(fid)</code> | <code>ftell returns the byte offset.</code> |
| **`feof`** <code>feof(fid)</code> | End-of-file indicator. | <code>feof(fid)</code> | <code>feof returns 1 after the last byte.</code> |
| **`frewind`** <code>frewind(fid)</code> | Rewind a file. | <code>frewind(fid)</code> | <code>frewind returns 0 on success.</code> |
| **`ferror`** <code>ferror(fid)</code> | File error message. | <code>ferror(fid)</code> | <code>ferror returns the current error text or an empty string.</code> |
| **`fgetl`** <code>fgetl(fid)</code> | Read a line without the newline. | <code>fgetl(fid)</code> | <code>fgetl returns one text line.</code> |
| **`fgets`** <code>fgets(fid)</code> | Read a line including the newline. | <code>fgets(fid)</code> | <code>fgets returns one text line with its terminator.</code> |
| **`delete`** <code>delete(file)</code> | Delete a file. | <code>delete('old.txt')</code> | <code>delete removes a file or handle.</code> |
| **`copyfile`** <code>copyfile(src,dst)</code> | Copy a file or folder. | <code>copyfile('a.txt', 'b.txt')</code> | <code>copyfile returns status and message.</code> |
| **`movefile`** <code>movefile(src,dst)</code> | Move a file or folder. | <code>movefile('a.txt', 'b.txt')</code> | <code>movefile returns status and message.</code> |
| **`rmdir`** <code>rmdir(path)</code> | Remove a folder. | <code>rmdir('old-folder')</code> | <code>rmdir returns status and message.</code> |

## dates

<a id="dates"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`now`** <code>now</code> | Current date as a serial date number. | <code>now</code> | <code>now returns the current date/time as a serial number.</code> |
| **`clock`** <code>clock</code> | Current date and time vector. | <code>clock</code> | <code>clock returns [year month day hour minute second].</code> |
| **`datenum`** <code>datenum(y,m,d[,h,mi,s])</code> | Serial date number. | <code>r = datenum(2024,1,1);</code> | <code>[739252]</code> |
| **`datestr`** <code>datestr(d[,format])</code> | Format a serial date number. | <code>r = datestr(datenum(2024,1,1));</code> | <code>01-Jan-2024 00:00:00</code> |
| **`etime`** <code>etime(t2,t1)</code> | Elapsed seconds between date vectors. | <code>r = etime([2024 1 1 0 0 1],[2024 1 1 0 0 0]);</code> | <code>[1]</code> |

## system

<a id="system"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`version`** <code>version</code> | MatCraft version string. | <code>r = version;</code> | <code>MatCraft 0.1.0</code> |
| **`computer`** <code>computer</code> | Computer architecture string. | <code>r = computer;</code> | <code>PCWIN64</code> |
| **`ispc`** <code>ispc</code> | True on Windows. | <code>r = ispc;</code> | <code>[1]</code> |
| **`isunix`** <code>isunix</code> | True on Unix-like systems. | <code>r = isunix;</code> | <code>[0]</code> |
| **`ismac`** <code>ismac</code> | True on macOS. | <code>r = ismac;</code> | <code>[0]</code> |
| **`getenv`** <code>getenv(name)</code> | Read an environment variable. | <code>setenv('MATCRAFT_DEMO','1'); r = getenv('MATCRAFT_DEMO');</code> | <code>1</code> |
| **`setenv`** <code>setenv(name,value)</code> | Set an environment variable. | <code>setenv('MATCRAFT_DEMO','1'); r = getenv('MATCRAFT_DEMO');</code> | <code>1</code> |
| **`system`** <code>system(command)</code> | Run a shell command. | <code>system('echo hello')</code> | <code>system runs a command and returns status and output.</code> |
| **`dos`** <code>dos(command)</code> | Windows shell command alias. | <code>dos('dir')</code> | <code>dos runs a command through the Windows shell.</code> |
| **`unix`** <code>unix(command)</code> | Unix shell command alias. | <code>unix('ls')</code> | <code>unix runs a command through the Unix shell.</code> |
| **`ver`** <code>ver</code> | Toolbox/version information. | <code>r = ver;</code> | <code>struct with fields: Name, Release, Version</code> |
| **`license`** <code>license(feature)</code> | License availability. | <code>r = license('test','MATLAB');</code> | <code>[0]</code> |
| **`diary`** <code>diary [on&#124;off&#124;file]</code> | Record console output to a file. | <code>diary('session.log')</code> | <code>diary toggles or redirects console logging.</code> |
| **`beep`** <code>beep</code> | Play the system beep. | <code>beep</code> | <code>beep plays the system alert sound.</code> |

## graphics objects

<a id="graphics-objects"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`get`** <code>get(handle,property)</code> | Read a figure property. | <code>clf; plot(1:2); grid on; r = get(gcf,'grid');</code> | <code>on</code> |
| **`set`** <code>set(handle,property,value,...)</code> | Set figure properties. | <code>clf; set(gcf,'ylim',[0 9]); r = get(gcf,'ylim');</code> | <code>[0 9]</code> |
| **`drawnow`** <code>drawnow</code> | Flush pending graphics updates. | <code>clf; plot(1:2); drawnow; r = 'drawn';</code> | <code>drawn</code> |
| **`waitbar`** <code>waitbar(fraction[,message])</code> | Show a progress bar. | <code>waitbar(0.5, 'Working')</code> | <code>waitbar opens a progress dialog.</code> |
| **`gca`** <code>gca</code> | Current axes handle. | <code>clf; plot(1:2); r = gca;</code> | <code>[1]</code> |
| **`ishandle`** <code>ishandle(h)</code> | True for graphics handles. | <code>clf; plot(1:2); r = ishandle(gcf);</code> | <code>[1]</code> |
| **`findobj`** <code>findobj([handle],Name,Value,...)</code> | Find graphics objects. | <code>clf; plot(1:2); r = numel(findobj('Type','axes'));</code> | <code>[0]</code> |
| **`findall`** <code>findall(Name,Value,...)</code> | Find all graphics objects including hidden ones. | <code>clf; plot(1:2); r = numel(findall('Type','axes'));</code> | <code>[0]</code> |
| **`ishold`** <code>ishold</code> | True when hold is on. | <code>clf; plot(1:2); hold on; r = ishold;</code> | <code>[1]</code> |

## image io

<a id="image-io"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`imread`** <code>imread(file)</code> | Read an image file. | <code>imread('image.png')</code> | <code>imread returns a grayscale or RGB image array.</code> |
| **`imwrite`** <code>imwrite(A,file)</code> | Write an image file. | <code>imwrite(rand(8,8), 'image.png')</code> | <code>imwrite writes PNG or JPEG data.</code> |
| **`im2double`** <code>im2double(A)</code> | Scale integer image data to [0,1]. | <code>r = im2double(uint8(255));</code> | <code>[1]</code> |
| **`fspecial`** <code>fspecial(type,size)</code> | Create a standard image filter. | <code>r = fspecial('average',3);</code> | <code>[0.1111 0.1111 0.1111;0.1111 0.1111 0.1111;0.1111 0.1111 0.1111]</code> |

## workspace/env

<a id="workspace-env"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`eval`** <code>eval(expr)</code> | Evaluate an expression in the current workspace. | <code>eval('x = 1+2;'); r = x;</code> | <code>[3]</code> |
| **`evalc`** <code>evalc(expr)</code> | Evaluate and capture console output. | <code>r = evalc('disp(7)');</code> | <code>7</code> |
| **`evalin`** <code>evalin(ws,expr)</code> | Evaluate in a named workspace. | <code>r = evalin('base','1+2');</code> | <code>[3]</code> |
| **`assignin`** <code>assignin(ws,name,value)</code> | Assign a variable in a named workspace. | <code>assignin('base','zz',5); r = zz;</code> | <code>[5]</code> |
| **`error`** <code>error([id,]message)</code> | Raise an error. | <code>error('bad')</code> | <code>try, error('bad'); catch e, e.message returns 'bad'.</code> |
| **`warning`** <code>warning([id,]message)</code> | Raise a warning. | <code>warning('demo')</code> | <code>warning('demo') prints a warning.</code> |
| **`lasterror`** <code>lasterror</code> | Most recent error. | <code>try, error('bad'); catch, end; e = lasterror; r = e.message;</code> | <code>bad</code> |
| **`exist`** <code>exist(name[,kind])</code> | Test whether a name or path exists. | <code>r = exist('abs') &gt; 0;</code> | <code>[1]</code> |
| **`which`** <code>which(name)</code> | Locate a function or file. | <code>r = ischar(which('abs'));</code> | <code>[1]</code> |
| **`clc`** <code>clc</code> | Clear the command window. | <code>clc</code> | <code>clc clears the command window.</code> |
| **`home`** <code>home</code> | Move the cursor to the top of the command window. | <code>home</code> | <code>home clears the visible command window.</code> |
| **`tic`** <code>tic</code> | Start a stopwatch. | <code>tic</code> | <code>tic starts a stopwatch; toc reads it.</code> |
| **`toc`** <code>toc</code> | Read the stopwatch. | <code>toc</code> | <code>toc returns elapsed seconds since tic.</code> |
| **`pause`** <code>pause([seconds])</code> | Pause execution. | <code>pause(1)</code> | <code>pause waits for a key or a number of seconds.</code> |
| **`nargin`** <code>nargin</code> | Number of caller input arguments. | <code>nargin</code> | <code>nargin is the input count inside a function.</code> |
| **`nargout`** <code>nargout</code> | Number of caller output arguments. | <code>nargout</code> | <code>nargout is the output count inside a function.</code> |
| **`narginchk`** <code>narginchk(min,max)</code> | Validate the number of input arguments. | <code>narginchk(1, 2)</code> | <code>narginchk errors when nargin is outside [min,max].</code> |
| **`nargoutchk`** <code>nargoutchk(min,max)</code> | Validate the number of output arguments. | <code>nargoutchk(0, 1)</code> | <code>nargoutchk errors when nargout is outside [min,max].</code> |
| **`nargchk`** <code>nargchk(min,max,n)</code> | Legacy argument-count check. | <code>nargchk(1, 2, 1)</code> | <code>nargchk returns an error message or an empty value.</code> |
| **`disp`** <code>disp(x)</code> | Display a value without its variable name. | <code>disp(5)</code> | <code>disp(5) prints 5.</code> |
| **`display`** <code>display(x)</code> | Display a value with its variable name. | <code>display(5)</code> | <code>display(x) prints the name and value.</code> |
| **`format`** <code>format [short&#124;long&#124;...]</code> | Set numeric display format. | <code>format long</code> | <code>format changes how numbers are printed.</code> |
| **`fprintf`** <code>fprintf(format,...)</code> | Formatted console/file output. | <code>fprintf('%d\n', 5)</code> | <code>fprintf prints formatted text and returns the character count.</code> |
| **`sprintf`** <code>sprintf(format,...)</code> | Formatted text output. | <code>r = sprintf('%d-%d',1,2);</code> | <code>1-2</code> |
| **`input`** <code>input(prompt)</code> | Prompt for user input. | <code>input('Enter a value: ')</code> | <code>input reads a value typed at the prompt.</code> |
| **`who`** <code>who</code> | List variable names. | <code>who</code> | <code>who prints the names of workspace variables.</code> |
| **`whos`** <code>whos</code> | List variable names, sizes and classes. | <code>whos</code> | <code>whos prints workspace variable details.</code> |
| **`clear`** <code>clear [vars&#124;all]</code> | Remove variables from the workspace. | <code>x = 1; clear x; r = exist('x');</code> | <code>[0]</code> |
| **`assert`** <code>assert(cond[,message])</code> | Error when a condition is false. | <code>assert(true); r = 'passed';</code> | <code>passed</code> |
| **`rng`** <code>rng(seed&#124;'shuffle'&#124;'default')</code> | Seed the random-number generator. | <code>rng(1)</code> | <code>rng seeds the generator (or reads/sets its state).</code> |
| **`normrnd`** <code>normrnd(mu,sigma,sz)</code> | Normal random numbers. | <code>rng(1); r = normrnd(0,1,1,3);</code> | <code>[1.4243 -0.6189 -0.5908]</code> |
| **`mfilename`** <code>mfilename([kind])</code> | Name of the running file. | <code>mfilename</code> | <code>mfilename returns the current script/function name.</code> |
| **`help`** <code>help(name)</code> | Show builtin/function help. | <code>help('abs')</code> | <code>help('abs') prints a one-line description.</code> |
| **`doc`** <code>doc(name)</code> | Open documentation. | <code>doc('abs')</code> | <code>doc('abs') opens the documentation view.</code> |
| **`global`** <code>global x</code> | Declare a shared global variable. | <code>global gx; gx = 3; r = gx;</code> | <code>[3]</code> |
| **`persistent`** <code>persistent x</code> | Declare a function-persistent variable. | <code>persistent counter</code> | <code>persistent keeps a variable between calls to a function.</code> |
| **`import`** <code>import pkg.*</code> | Import package members. | <code>import pkg.*</code> | <code>import resolves names from a +package directory.</code> |
| **`argv`** <code>argv</code> | Command-line arguments. | <code>argv</code> | <code>argv is the CLI argument vector.</code> |

## files/path

<a id="files-path"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`save`** <code>save(file[,vars])</code> | Save workspace variables to a .mat file. | <code>save('data.mat', 'x')</code> | <code>save writes variables to a MAT-file.</code> |
| **`load`** <code>load(file)</code> | Load variables from a .mat file. | <code>load('data.mat')</code> | <code>load restores variables from a MAT-file.</code> |
| **`type`** <code>type(file)</code> | Display a file. | <code>type('script.m')</code> | <code>type prints a file's contents.</code> |
| **`addpath`** <code>addpath(dir[,'-begin'&#124;'-end'])</code> | Add a directory to the search path. | <code>p = tempdir; addpath(p); r = contains(path,p); rmpath(p);</code> | <code>[1]</code> |
| **`rmpath`** <code>rmpath(dir)</code> | Remove a directory from the search path. | <code>p = tempdir; addpath(p); rmpath(p); r = ~contains(path,p);</code> | <code>[1]</code> |
| **`savepath`** <code>savepath([file])</code> | Persist the current search path. | <code>savepath</code> | <code>savepath stores the path for future sessions.</code> |
| **`path`** <code>path([newpath])</code> | Get or set the search path. | <code>r = ischar(path);</code> | <code>[1]</code> |
| **`pwd`** <code>pwd</code> | Current folder. | <code>r = ischar(pwd);</code> | <code>[1]</code> |
| **`cd`** <code>cd(dir)</code> | Change the current folder. | <code>old = pwd; cd(tempdir); r = ischar(pwd); cd(old);</code> | <code>[1]</code> |
| **`dir`** <code>dir([pattern])</code> | List folder contents. | <code>r = isstruct(dir('.'));</code> | <code>[1]</code> |
| **`ls`** <code>ls([pattern])</code> | List folder contents to the console. | <code>ls</code> | <code>ls prints the current folder listing.</code> |
| **`genpath`** <code>genpath(dir)</code> | Search path including subfolders. | <code>r = contains(genpath(pwd),pwd);</code> | <code>[1]</code> |
| **`fullfile`** <code>fullfile(parts,...)</code> | Build a platform path. | <code>r = fullfile('a','b');</code> | <code>a\b</code> |
| **`fileparts`** <code>fileparts(path)</code> | Split a path into folder, name and extension. | <code>[d,n,e] = fileparts('a/b.txt'); r = [n e];</code> | <code>b.txt</code> |
| **`filesep`** <code>filesep</code> | Platform path separator. | <code>r = filesep;</code> | <code>\</code> |
| **`isdir`** <code>isdir(path)</code> | True when a path is a folder. | <code>r = isdir(tempdir);</code> | <code>[1]</code> |
| **`isfile`** <code>isfile(path)</code> | True when a path is a file. | <code>isfile('notes.txt')</code> | <code>isfile returns true for an existing regular file.</code> |
| **`tempdir`** <code>tempdir</code> | Temporary folder path. | <code>r = ischar(tempdir);</code> | <code>[1]</code> |
| **`tempname`** <code>tempname</code> | Unique temporary file name. | <code>r = ischar(tempname);</code> | <code>[1]</code> |
| **`csvread`** <code>csvread(file)</code> | Read a CSV numeric matrix. | <code>csvread('data.csv')</code> | <code>csvread reads comma-separated numeric data.</code> |
| **`mkdir`** <code>mkdir(path)</code> | Create a folder. | <code>p = [tempname]; r = mkdir(p); rmdir(p);</code> | <code>[1]</code> |

## plotting

<a id="plotting"></a>

| Function / usage | Description | Example | Output / effect |
| --- | --- | --- | --- |
| **`figure`** <code>figure([n])</code> | Create or select a figure. | <code>close all; r = figure;</code> | <code>[1]</code> |
| **`gcf`** <code>gcf</code> | Current figure handle. | <code>clf; r = gcf;</code> | <code>[1]</code> |
| **`clf`** <code>clf</code> | Clear the current figure. | <code>clf; r = 'cleared';</code> | <code>cleared</code> |
| **`close`** <code>close([handle&#124;all])</code> | Close figures. | <code>close all; r = 'closed';</code> | <code>closed</code> |
| **`subplot`** <code>subplot(m,n,p)</code> | Select an axes tile. | <code>clf; r = subplot(1,2,1);</code> | <code>[1]</code> |
| **`plot`** <code>plot(x,y[,spec])</code> | Line plot. | <code>clf; plot(1:3); r = gcf;</code> | <code>[1]</code> |
| **`scatter`** <code>scatter(x,y[,spec])</code> | Scatter plot. | <code>clf; scatter(1:3,[3 1 2]); r = gcf;</code> | <code>[1]</code> |
| **`stem`** <code>stem(x,y)</code> | Stem plot. | <code>clf; stem(1:3,[3 1 2]); r = gcf;</code> | <code>[1]</code> |
| **`bar`** <code>bar(y) &#124; bar(x,y)</code> | Vertical bar chart. | <code>clf; bar([3 1 2]); r = gcf;</code> | <code>[1]</code> |
| **`histogram`** <code>histogram(x[,n])</code> | Histogram plot. | <code>clf; histogram([1 2 2 3]); r = gcf;</code> | <code>[1]</code> |
| **`area`** <code>area(x,y)</code> | Filled area plot. | <code>clf; area(1:3,[1 3 2]); r = gcf;</code> | <code>[1]</code> |
| **`stairs`** <code>stairs(x,y)</code> | Stair-step plot. | <code>clf; stairs(1:3,[1 3 2]); r = gcf;</code> | <code>[1]</code> |
| **`errorbar`** <code>errorbar(x,y,e)</code> | Line plot with error bars. | <code>clf; errorbar(1:3,[1 2 3],[0.1 0.1 0.1]); r = gcf;</code> | <code>[1]</code> |
| **`contourf`** <code>contourf(Z[,n])</code> | Filled contour plot. | <code>clf; contourf(magic(3)); r = gcf;</code> | <code>[1]</code> |
| **`contour3`** <code>contour3(Z[,n])</code> | 3-D contour lines. | <code>clf; contour3(magic(3)); r = gcf;</code> | <code>[1]</code> |
| **`semilogx`** <code>semilogx(x,y)</code> | Plot with a logarithmic x-axis. | <code>clf; semilogx([1 10 100],[1 2 3]); r = gcf;</code> | <code>[1]</code> |
| **`semilogy`** <code>semilogy(x,y)</code> | Plot with a logarithmic y-axis. | <code>clf; semilogy(1:3,[1 10 100]); r = gcf;</code> | <code>[1]</code> |
| **`loglog`** <code>loglog(x,y)</code> | Plot with logarithmic axes. | <code>clf; loglog([1 10 100],[1 10 100]); r = gcf;</code> | <code>[1]</code> |
| **`hold`** <code>hold on&#124;off</code> | Retain the current plot. | <code>clf; plot(1:2); hold on; r = ishold;</code> | <code>[1]</code> |
| **`grid`** <code>grid on&#124;off</code> | Toggle the axes grid. | <code>clf; plot(1:2); grid on; r = 'grid on';</code> | <code>grid on</code> |
| **`title`** <code>title(text)</code> | Set the axes title. | <code>clf; plot(1:2); title('demo'); r = 'title set';</code> | <code>title set</code> |
| **`xlabel`** <code>xlabel(text)</code> | Set the x-axis label. | <code>clf; plot(1:2); xlabel('x'); r = 'xlabel set';</code> | <code>xlabel set</code> |
| **`ylabel`** <code>ylabel(text)</code> | Set the y-axis label. | <code>clf; plot(1:2); ylabel('y'); r = 'ylabel set';</code> | <code>ylabel set</code> |
| **`zlabel`** <code>zlabel(text)</code> | Set the z-axis label. | <code>clf; plot3(1:2,1:2,1:2); zlabel('z'); r = 'zlabel set';</code> | <code>zlabel set</code> |
| **`legend`** <code>legend(labels,...)</code> | Add a legend. | <code>clf; plot(1:3); legend('a'); r = 'legend set';</code> | <code>legend set</code> |
| **`xlim`** <code>xlim([lo hi])</code> | Get or set x limits. | <code>clf; plot(1:3); xlim([0 4]); r = xlim;</code> | <code>[0 4]</code> |
| **`ylim`** <code>ylim([lo hi])</code> | Get or set y limits. | <code>clf; plot(1:3); ylim([0 4]); r = ylim;</code> | <code>[0 4]</code> |
| **`zlim`** <code>zlim([lo hi])</code> | Get or set z limits. | <code>clf; plot3(1:3,1:3,1:3); zlim([0 4]); r = zlim;</code> | <code>[0 4]</code> |
| **`xscale`** <code>xscale('linear'&#124;'log')</code> | Set the x-axis scale. | <code>clf; plot(1:3); xscale('log'); r = 'xscale set';</code> | <code>xscale set</code> |
| **`yscale`** <code>yscale('linear'&#124;'log')</code> | Set the y-axis scale. | <code>clf; plot(1:3); yscale('log'); r = 'yscale set';</code> | <code>yscale set</code> |
| **`view`** <code>view(az,el) &#124; view(2&#124;3)</code> | Set the 3-D camera view. | <code>clf; plot3(1:3,1:3,1:3); view(2); r = 'view set';</code> | <code>view set</code> |
| **`colormap`** <code>colormap(map)</code> | Set or read the figure colormap. | <code>clf; imagesc(magic(3)); colormap(parula(4)); r = colormap;</code> | <code>parula</code> |
| **`colorbar`** <code>colorbar([on&#124;off])</code> | Show a colorbar. | <code>clf; imagesc(magic(3)); colorbar; r = 'colorbar shown';</code> | <code>colorbar shown</code> |
| **`lighting`** <code>lighting flat&#124;gouraud&#124;none</code> | Set surface lighting. | <code>clf; surf(magic(3)); lighting flat; r = 'lighting set';</code> | <code>lighting set</code> |
| **`light`** <code>light</code> | Add a light source. | <code>clf; surf(magic(3)); light; r = 'light added';</code> | <code>light added</code> |
| **`lines`** <code>lines([n])</code> | Lines colormap. | <code>r = size(lines(4));</code> | <code>[4 3]</code> |
| **`volcano`** <code>volcano(log2fc,p)</code> | Volcano plot. | <code>clf; r = volcano([-3 -1 1 3],[0.001 0.2 0.01 0.0001]);</code> | <code>[1]</code> |
| **`dendrogram`** <code>dendrogram(Z)</code> | Hierarchical-clustering dendrogram. | <code>clf; Z = linkage([1 2 3],'single'); r = dendrogram(Z);</code> | <code>[1]</code> |
| **`clustergram`** <code>clustergram(X)</code> | Clustered heatmap ordered by average-linkage trees. | <code>clf; clustergram([1 2 3;4 5 6;7 8 9;10 11 12]); r = gcf;</code> | <code>[1]</code> |
| **`venn`** <code>venn(A,B[,C,D])</code> | Venn diagram for 2-4 sets. | <code>clf; r = venn({'a','b'},{'b','c'});</code> | <code>[1]</code> |
| **`venncounts`** <code>venncounts(A,B,...)</code> | Region counts for 2-10 sets. | <code>r = venncounts({'a','b'},{'b','c'});</code> | <code>[0 0 0;1 0 1;0 1 1;1 1 1]</code> |
| **`quiver`** <code>quiver(X,Y,U,V)</code> | 2-D vector field. | <code>clf; r = quiver(1:2,1:2,[1 1],[0 0]);</code> | <code>[1]</code> |
| **`quiver3`** <code>quiver3(X,Y,Z,U,V,W)</code> | 3-D vector field. | <code>clf; r = quiver3(1:2,1:2,1:2,[1 1],[0 0],[1 1]);</code> | <code>[1]</code> |
| **`streamline`** <code>streamline(X,Y,U,V,SX,SY)</code> | Integrate streamlines through a 2-D vector field. | <code>clf; [X,Y] = meshgrid(1:3,1:3); streamline(X,Y,ones(3),zeros(3),1,1); r = gcf;</code> | <code>[1]</code> |
| **`streamslice`** <code>streamslice(X,Y,U,V)</code> | Streamlines on a slice of a 2-D vector field. | <code>clf; [X,Y] = meshgrid(1:3,1:3); streamslice(X,Y,ones(3),ones(3)); r = gcf;</code> | <code>[1]</code> |
| **`heatmap`** <code>heatmap(C)</code> | Heatmap with colorbar. | <code>clf; heatmap(magic(3)); r = gcf;</code> | <code>[1]</code> |
| **`validatecolor`** <code>validatecolor(c)</code> | Validate or normalize a color specification. | <code>r = validatecolor([1 0 0]);</code> | <code>[1 0 0]</code> |
| **`caxis`** <code>caxis([lo hi])</code> | Set the color limits. | <code>clf; imagesc(magic(3)); caxis([0 10]); r = caxis;</code> | <code>[0 10]</code> |
| **`clim`** <code>clim([lo hi])</code> | Set the color limits. | <code>clf; imagesc(magic(3)); clim([0 10]); r = clim;</code> | <code>[0 10]</code> |
| **`axis`** <code>axis([x0 x1 y0 y1])</code> | Set axis limits and modes. | <code>clf; plot(1:3); axis([0 4 0 4]); r = axis;</code> | <code>[0 4 0 4]</code> |
| **`box`** <code>box on&#124;off</code> | Toggle the axes box. | <code>clf; plot(1:2); box on; r = 'box on';</code> | <code>box on</code> |
| **`daspect`** <code>daspect([x y z])</code> | Set the data aspect ratio. | <code>clf; plot(1:2); daspect([1 1 1]); r = daspect;</code> | <code>[1 1 1]</code> |
| **`imagesc`** <code>imagesc(C) &#124; imagesc(x,y,C)</code> | Scaled image plot. | <code>clf; imagesc(magic(3)); r = gcf;</code> | <code>[1]</code> |
| **`image`** <code>image(C)</code> | Image plot. | <code>clf; image(magic(3)); r = gcf;</code> | <code>[1]</code> |
| **`imshow`** <code>imshow(A)</code> | Display an image. | <code>clf; imshow(ones(2,2)); r = gcf;</code> | <code>[1]</code> |
| **`text`** <code>text(x,y,str)</code> | Add a text label. | <code>clf; plot(1:2); text(1,1,'a'); r = 'text added';</code> | <code>text added</code> |
| **`rectangle`** <code>rectangle('Position',[x y w h])</code> | Draw a rectangle. | <code>clf; rectangle('Position',[0 0 1 1]); r = 'rectangle added';</code> | <code>rectangle added</code> |
| **`patch`** <code>patch(x,y,c)</code> | Draw a filled polygon. | <code>clf; patch([0 1 1],[0 0 1],'b'); r = 'patch added';</code> | <code>patch added</code> |
| **`axes`** <code>axes([left bottom width height])</code> | Create or select axes. | <code>clf; r = axes;</code> | <code>[1]</code> |
| **`boxplot`** <code>boxplot(x[,groups])</code> | Box-and-whisker plot. | <code>clf; boxplot([1 2 3 4]); r = gcf;</code> | <code>[1]</code> |
| **`xticks`** <code>xticks(values)</code> | Set x ticks. | <code>clf; plot(1:3); xticks([1 2 3]); r = xticks;</code> | <code>[1 2 3]</code> |
| **`yticks`** <code>yticks(values)</code> | Set y ticks. | <code>clf; plot(1:3); yticks([1 2 3]); r = yticks;</code> | <code>[1 2 3]</code> |
| **`xticklabels`** <code>xticklabels(labels)</code> | Set x tick labels. | <code>clf; plot(1:3); xticklabels({'a','b','c'}); r = 'labels set';</code> | <code>labels set</code> |
| **`yticklabels`** <code>yticklabels(labels)</code> | Set y tick labels. | <code>clf; plot(1:3); yticklabels({'a','b','c'}); r = 'labels set';</code> | <code>labels set</code> |
| **`xtickangle`** <code>xtickangle(deg)</code> | Rotate x tick labels. | <code>clf; plot(1:3); xtickangle(45); r = 'angle set';</code> | <code>angle set</code> |
| **`ytickangle`** <code>ytickangle(deg)</code> | Rotate y tick labels. | <code>clf; plot(1:3); ytickangle(45); r = 'angle set';</code> | <code>angle set</code> |
| **`sgtitle`** <code>sgtitle(text)</code> | Figure-wide title. | <code>clf; plot(1:2); sgtitle('demo'); r = 'sgtitle set';</code> | <code>sgtitle set</code> |
| **`tiledlayout`** <code>tiledlayout(m,n)</code> | Create a tiled layout. | <code>clf; tiledlayout(1,2); r = 'layout set';</code> | <code>layout set</code> |
| **`nexttile`** <code>nexttile</code> | Select the next tile. | <code>clf; tiledlayout(1,2); nexttile; plot(1:2); r = gcf;</code> | <code>[1]</code> |
| **`yline`** <code>yline(y[,spec])</code> | Horizontal reference line. | <code>clf; plot(1:3); yline(2); r = 'yline added';</code> | <code>yline added</code> |
| **`barh`** <code>barh(y)</code> | Horizontal bar chart. | <code>clf; barh([3 1 2]); r = gcf;</code> | <code>[1]</code> |
| **`annotation`** <code>annotation(type,...)</code> | Add a figure annotation. | <code>clf; annotation('textarrow',[0.2 0.4],[0.2 0.4]); r = 'annotation added';</code> | <code>annotation added</code> |
| **`gobjects`** <code>gobjects(n)</code> | Preallocate graphics objects. | <code>r = numel(gobjects(2));</code> | <code>[2]</code> |
| **`isgraphics`** <code>isgraphics(h)</code> | True for graphics handles. | <code>clf; plot(1:2); r = isgraphics(gcf);</code> | <code>[1]</code> |
| **`allchild`** <code>allchild(h)</code> | All children of a graphics object. | <code>clf; plot(1:2); r = numel(allchild(gcf));</code> | <code>[0]</code> |
| **`ancestor`** <code>ancestor(h,type)</code> | Find an ancestor of a graphics object. | <code>clf; plot(1:2); r = ancestor(gca,'figure');</code> | <code>[1]</code> |
| **`getappdata`** <code>getappdata(h,name)</code> | Read application data. | <code>clf; setappdata(gcf,'k',5); r = getappdata(gcf,'k');</code> | <code>[5]</code> |
| **`setappdata`** <code>setappdata(h,name,value)</code> | Store application data. | <code>clf; setappdata(gcf,'k',5); r = getappdata(gcf,'k');</code> | <code>[5]</code> |
| **`gcp`** <code>gcp</code> | Current point in the axes. | <code>clf; plot(1:2); r = gcp;</code> | <code>[0 0 0]</code> |
| **`cla`** <code>cla</code> | Clear the current axes. | <code>clf; plot(1:2); cla; r = 'axes cleared';</code> | <code>axes cleared</code> |
| **`colororder`** <code>colororder(colors)</code> | Set the axes color order. | <code>clf; colororder([1 0 0;0 1 0]); r = 'color order set';</code> | <code>color order set</code> |
| **`getpixelposition`** <code>getpixelposition(h)</code> | Axes position in pixels. | <code>clf; plot(1:2); r = numel(getpixelposition(gca));</code> | <code>[4]</code> |
| **`savefig`** <code>savefig(file)</code> | Save a figure to a FIG file. | <code>savefig('figure.fig')</code> | <code>savefig writes a figure file.</code> |
| **`hsv`** <code>hsv([n])</code> | HSV colormap. | <code>r = size(hsv(4));</code> | <code>[4 3]</code> |
| **`parula`** <code>parula([n])</code> | Parula colormap. | <code>r = size(parula(4));</code> | <code>[4 3]</code> |
| **`cool`** <code>cool([n])</code> | Cool colormap. | <code>r = size(cool(4));</code> | <code>[4 3]</code> |
| **`pie`** <code>pie(x[,labels])</code> | Pie chart. | <code>clf; pie([1 2 3]); r = gcf;</code> | <code>[1]</code> |
| **`contour`** <code>contour(Z[,n])</code> | Contour plot. | <code>clf; contour(magic(3)); r = gcf;</code> | <code>[1]</code> |
| **`plot3`** <code>plot3(x,y,z[,spec])</code> | 3-D line plot. | <code>clf; plot3(1:3,1:3,1:3); r = gcf;</code> | <code>[1]</code> |
| **`surf`** <code>surf(Z) &#124; surf(X,Y,Z)</code> | 3-D shaded surface. | <code>clf; surf(magic(3)); r = gcf;</code> | <code>[1]</code> |
| **`surfl`** <code>surfl(Z)</code> | Lit 3-D surface. | <code>clf; surfl(magic(3)); r = gcf;</code> | <code>[1]</code> |
| **`surfc`** <code>surfc(Z)</code> | Surface with base contours. | <code>clf; surfc(magic(3)); r = gcf;</code> | <code>[1]</code> |
| **`mesh`** <code>mesh(Z)</code> | 3-D wireframe. | <code>clf; mesh(magic(3)); r = gcf;</code> | <code>[1]</code> |
| **`vline`** <code>vline(x[,spec])</code> | Vertical reference line. | <code>clf; plot(1:3); vline(2); r = 'vline added';</code> | <code>vline added</code> |
| **`xline`** <code>xline(x[,spec])</code> | Vertical reference line. | <code>clf; plot(1:3); xline(2); r = 'xline added';</code> | <code>xline added</code> |
| **`saveas`** <code>saveas(h,file)</code> | Save a figure to a file. | <code>saveas(gcf, 'figure.svg')</code> | <code>saveas writes SVG, PNG or PDF by extension.</code> |
| **`exportgraphics`** <code>exportgraphics(h,file)</code> | Export a figure. | <code>exportgraphics(gcf, 'figure.png')</code> | <code>exportgraphics writes an image or vector file.</code> |
| **`print`** <code>print([-dformat],file)</code> | Print/export a figure. | <code>print('-dsvg', 'figure.svg')</code> | <code>print exports using -dsvg, -dpng or -dpdf.</code> |

## Notes

- Outputs are shown as the interpreter prints them; numeric matrices
  larger than 16 elements are summarised as `rows×cols class`.
- `STATIC:` entries in the source data describe dialogs, network
  access, host-dependent paths or file effects rather than a value.
- Compatibility stubs are labelled in their description (for example
  `isdatetime`, `isduration`, `istimetable` and the categorical
  editing helpers, which currently pass their input through).
