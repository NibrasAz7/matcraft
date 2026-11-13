% Image-processing tour: colour conversion, filtering, edges, components.

% Synthetic 6x6 grayscale image with a bright square and an isolated dot.
I = zeros(6, 6);
I(2:4, 2:4) = 1;
I(5, 5) = 1;

fprintf('--- rgb2gray ---\n');
rgb = zeros(1, 1, 3);
rgb(1, 1, 1) = 1;
fprintf('pure red -> %g\n\n', rgb2gray(rgb));

fprintf('--- resize / filter ---\n');
R = imresize(I, [12 12], 'nearest');
szr0 = size(R);
fprintf('resized size: %s\n', mat2str(szr0));
H = fspecial('average', 3);
S = imfilter(I, H, 'replicate');
fprintf('smoothed centre: %g\n\n', S(3, 3));

fprintf('--- edges and components ---\n');
E = edge(I, 'sobel');
[L, n] = bwlabel(E, 8);
fprintf('edge pixels: %d, components: %d\n', sum(E(:)), n);
Sprops = regionprops(L, {'Area', 'Centroid'});
if n > 0
    fprintf('first region area: %g, centroid: [%g %g]\n\n', Sprops(1).Area, Sprops(1).Centroid(1), Sprops(1).Centroid(2));
end

fprintf('--- threshold ---\n');
t = graythresh(I);
B = imbinarize(I);
fprintf('Otsu threshold: %.4f, foreground pixels: %d\n\n', t, sum(B(:)));

fprintf('--- morphology ---\n');
sq = strel('square', 3);
D = imdilate(I, sq);
E = imerode(I, sq);
fprintf('dilated pixels: %d, eroded pixels: %d\n', sum(D(:)), sum(E(:)));
open = imopen(D, sq);
close = imclose(E, sq);
fprintf('opened: %d, closed: %d\n\n', sum(open(:)), sum(close(:)));

fprintf('--- rotate and crop ---\n');
R = imrotate(I, 30, 'nearest', 'loose');
C = imcrop(I, [2 2 3 3]);
szr = size(R);
szc = size(C);
fprintf('rotated size: %s, crop size: %s\n\n', mat2str(szr), mat2str(szc));

fprintf('--- bicubic and Canny ---\n');
Bc = imresize(I, 2, 'bicubic');
szb = size(Bc);
Ec = edge(I, 'canny', [0.1 0.3], 1);
fprintf('bicubic size: %s, centre: %.4f\n', mat2str(szb), Bc(3,3));
fprintf('Canny edge pixels: %d\n', sum(Ec(:)));
