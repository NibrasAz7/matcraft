//! matplot — MatCraft's plotting crate.
//!
//! Phase 3 scope: a `Figure` data model (what `plot`/`scatter`/`stem`/`bar`,
//! `text` and `imshow` build) and a headless SVG renderer. No GPU, no
//! windowing — the interactive wgpu viewer lands later and consumes the
//! same model.

/// One plotted data set.
#[derive(Debug, Clone)]
pub struct Series {
    pub kind: SeriesKind,
    /// Point x-coords — or column coordinates for grid kinds (len `zshape.1`).
    pub x: Vec<f64>,
    /// Point y-coords — or row coordinates for grid kinds (len `zshape.0`).
    /// `pie` stores slice values here.
    pub y: Vec<f64>,
    /// `plot3` z-coords, or column-major grid data (`z[c*rows + r]`) for
    /// `imagesc`/`contour`/`surf`/`mesh`.
    pub z: Vec<f64>,
    /// Grid `(rows, cols)` for `imagesc`/`contour`/`surf`/`mesh`; `(0,0)` otherwise.
    pub zshape: (usize, usize),
    /// Explicit `contour` levels; empty = auto levels.
    pub levels: Vec<f64>,
    /// `pie` slice labels.
    pub labels: Vec<String>,
    /// `errorbar` half-amplitudes, one per point.
    pub err: Option<Vec<f64>>,
    /// `ImageRgb` pixel payload — row-major RGBA8, `zshape` = (rows, cols).
    /// `None` for every other kind.
    pub rgba: Option<Vec<u8>>,
    pub spec: LineSpec,
}

impl Series {
    /// True for kinds drawn by the 3-D projected path.
    fn is_3d(&self) -> bool {
        matches!(self.kind, SeriesKind::Line3 | SeriesKind::Mesh | SeriesKind::Surf)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeriesKind {
    Line,
    Scatter,
    Stem,
    Bar,
    /// Filled polygon from the curve down to y = 0.
    Area,
    /// Piecewise-constant step line (hold each y until the next x).
    Stairs,
    /// Points with vertical error bars (`err` carries half-lengths).
    ErrorBar,
    /// `imagesc` — grid cells coloured by `z` through the figure colormap.
    Image,
    /// `pie` — wedges sized by `y` (fractions of the sum).
    Pie,
    /// `contour` — marching-squares iso-lines over the `z` grid.
    Contour,
    /// `plot3` — a 3-D polyline (x/y/z point lists).
    Line3,
    /// `surf` — a 3-D surface: filled projected quads coloured by z.
    Surf,
    /// `mesh` — a 3-D surface wireframe coloured by z.
    Mesh,
    /// `text` — a string anchored at a data point: `x[0]`/`y[0]` carry
    /// the anchor, `labels[0]` the text, `spec.color` the colour.
    Text,
    /// `imshow`-class true-color image: `zshape` = (rows, cols), `rgba`
    /// carries row-major RGBA8 pixels.
    ImageRgb,
}

/// Dash pattern from a line spec like `'r--o'`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DashStyle {
    Solid,
    Dashed,
    Dotted,
    DashDot,
    /// Specified a marker with no line (`'o'`), or scatter series.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Marker {
    Circle,
    Plus,
    Star,
    Point,
    Cross,
    Square,
    Diamond,
    TriUp,
    TriDown,
    TriLeft,
    TriRight,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// The default axis colour order (MATLAB-style seven-colour cycle).
const CYCLE: [Rgb; 7] = [
    Rgb(0x00, 0x72, 0xBD),
    Rgb(0xD9, 0x53, 0x19),
    Rgb(0xED, 0xB1, 0x20),
    Rgb(0x7E, 0x2F, 0x8E),
    Rgb(0x77, 0xAC, 0x30),
    Rgb(0x4D, 0xBE, 0xEE),
    Rgb(0xA2, 0x14, 0x2F),
];

/// Line/marker/colour spec parsed from strings like `"r--o"`, `"k:"`, `"b"`.
#[derive(Debug, Clone)]
pub struct LineSpec {
    pub color: Option<Rgb>,
    pub dash: DashStyle,
    pub marker: Option<Marker>,
    /// `'FontSize'` value in px — consumed by `Text` series today;
    /// reserved as the `'MarkerSize'` carrier for point kinds.
    pub size: Option<f64>,
}

impl Default for LineSpec {
    fn default() -> Self {
        Self::new()
    }
}

impl LineSpec {
    pub fn new() -> Self {
        LineSpec { color: None, dash: DashStyle::Solid, marker: None, size: None }
    }

    /// Parse a MATLAB-style spec string. Unknown characters are ignored.
    pub fn parse(s: &str) -> Self {
        let mut out = LineSpec { color: None, dash: DashStyle::Solid, marker: None, size: None };
        let mut saw_dash = false;
        let cs: Vec<char> = s.chars().collect();
        let mut i = 0;
        while let Some(&c) = cs.get(i) {
            match c {
                'b' => out.color = Some(CYCLE[0]),
                'r' => out.color = Some(Rgb(0xA2, 0x14, 0x2F)),
                'g' => out.color = Some(Rgb(0x00, 0x80, 0x00)),
                'c' => out.color = Some(CYCLE[5]),
                'm' => out.color = Some(CYCLE[3]),
                'y' => out.color = Some(CYCLE[2]),
                'k' => out.color = Some(Rgb(0x00, 0x00, 0x00)),
                'w' => out.color = Some(Rgb(0xFF, 0xFF, 0xFF)),
                '-' => {
                    saw_dash = true;
                    out.dash = match cs.get(i + 1) {
                        Some('-') => {
                            i += 1;
                            DashStyle::Dashed
                        }
                        Some('.') => {
                            i += 1;
                            DashStyle::DashDot
                        }
                        _ => DashStyle::Solid,
                    };
                }
                ':' => {
                    saw_dash = true;
                    out.dash = DashStyle::Dotted;
                }
                'o' => out.marker = Some(Marker::Circle),
                '+' => out.marker = Some(Marker::Plus),
                '*' | 'p' | 'h' => out.marker = Some(Marker::Star),
                '.' => out.marker = Some(Marker::Point),
                'x' => out.marker = Some(Marker::Cross),
                's' => out.marker = Some(Marker::Square),
                'd' => out.marker = Some(Marker::Diamond),
                '^' => out.marker = Some(Marker::TriUp),
                'v' => out.marker = Some(Marker::TriDown),
                '<' => out.marker = Some(Marker::TriLeft),
                '>' => out.marker = Some(Marker::TriRight),
                _ => {}
            }
            i += 1;
        }
        // Marker given without a line style ⇒ markers only ('o', 'x').
        if out.marker.is_some() && !saw_dash {
            out.dash = DashStyle::None;
        }
        out
    }
}

/// A snapshot of one axes' drawing state — the per-axes half of `Figure`,
/// used to stash subplot tiles that are not currently selected.
#[derive(Debug, Clone)]
pub struct Axes {
    pub series: Vec<Series>,
    pub title: Option<String>,
    pub xlabel: Option<String>,
    pub ylabel: Option<String>,
    pub legend: Vec<String>,
    pub grid: bool,
    pub xlim: Option<(f64, f64)>,
    pub ylim: Option<(f64, f64)>,
    pub zlim: Option<(f64, f64)>,
    pub zlabel: Option<String>,
    pub hold: bool,
    pub xlog: bool,
    pub ylog: bool,
    /// `view(az, el)` camera for 3-D series, in degrees.
    pub view: (f64, f64),
    /// Colour scale for Image/Contour/Surf/Mesh series.
    pub cmap: Colormap,
    /// `clim`/`caxis` colour limits; `None` = auto from data.
    pub clim: Option<(f64, f64)>,
    /// `colorbar` — draw the colour scale beside the axes.
    pub colorbar: bool,
    /// `axis ij`/`imagesc` — y axis increases downward (row 1 at top).
    pub yrev: bool,
    /// `box on` (default) — draw the axes frame; `box off` hides it.
    pub boxed: bool,
    pub drawn: usize,
}

impl Axes {
    /// Rehydrate into a standalone `Figure` so the renderer can treat stashed
    /// tiles uniformly.
    fn to_figure(&self) -> Figure {
        let mut f = Figure::new();
        f.series = self.series.clone();
        f.title = self.title.clone();
        f.xlabel = self.xlabel.clone();
        f.ylabel = self.ylabel.clone();
        f.legend = self.legend.clone();
        f.grid = self.grid;
        f.xlim = self.xlim;
        f.ylim = self.ylim;
        f.zlim = self.zlim;
        f.zlabel = self.zlabel.clone();
        f.hold = self.hold;
        f.xlog = self.xlog;
        f.ylog = self.ylog;
        f.view = self.view;
        f.cmap = self.cmap;
        f.clim = self.clim;
        f.colorbar = self.colorbar;
        f.yrev = self.yrev;
        f.boxed = self.boxed;
        f.drawn = self.drawn;
        f
    }
}

impl Default for Axes {
    fn default() -> Self {
        // `boxed` defaults to true (`box on`); everything else zero/none.
        Axes {
            series: Vec::new(),
            title: None,
            xlabel: None,
            ylabel: None,
            legend: Vec::new(),
            grid: false,
            xlim: None,
            ylim: None,
            zlim: None,
            zlabel: None,
            hold: false,
            xlog: false,
            ylog: false,
            view: (-37.5, 30.0),
            cmap: Colormap::Parula,
            clim: None,
            colorbar: false,
            yrev: false,
            boxed: true,
            drawn: 0,
        }
    }
}

/// One occupied subplot tile — a 0-based inclusive cell span.
#[derive(Debug, Clone)]
pub struct Tile {
    pub r0: usize,
    pub c0: usize,
    pub r1: usize,
    pub c1: usize,
    pub axes: Axes,
}

/// `subplot(m,n,p)` state: an m×n grid of tiles; the currently selected tile
/// lives in the figure's flat fields (`cur`/`cur_rect` say where it goes).
#[derive(Debug, Clone)]
pub struct Subplot {
    pub rows: usize,
    pub cols: usize,
    /// Cell span of the current axes: (r0, c0, r1, c1).
    pub cur_rect: (usize, usize, usize, usize),
    /// Stashed non-current tiles.
    pub tiles: Vec<Tile>,
}

/// A figure: axes plus the series to draw.
#[derive(Debug, Clone)]
pub struct Figure {
    pub series: Vec<Series>,
    pub title: Option<String>,
    pub xlabel: Option<String>,
    pub ylabel: Option<String>,
    pub legend: Vec<String>,
    pub grid: bool,
    pub xlim: Option<(f64, f64)>,
    pub ylim: Option<(f64, f64)>,
    pub zlim: Option<(f64, f64)>,
    pub zlabel: Option<String>,
    /// `hold on` — new `plot` calls append instead of replacing.
    pub hold: bool,
    /// Log-scaled axes (`semilogx`/`semilogy`/`loglog`, `xscale`/`yscale`).
    pub xlog: bool,
    pub ylog: bool,
    /// `view(az, el)` camera for 3-D series, in degrees.
    pub view: (f64, f64),
    /// Colour scale for Image/Contour/Surf/Mesh series.
    pub cmap: Colormap,
    /// `clim`/`caxis` colour limits; `None` = auto from data.
    pub clim: Option<(f64, f64)>,
    /// `colorbar` — draw the colour scale beside the axes.
    pub colorbar: bool,
    /// `imagesc`/`axis ij` — y axis increases downward.
    pub yrev: bool,
    /// `box on` (default) — draw the axes frame; `box off` hides it.
    pub boxed: bool,
    /// `subplot` tiling; when set, `to_svg` renders one axes per tile.
    pub sub: Option<Subplot>,
    /// Series drawn so far; drives the default colour cycle.
    drawn: usize,
}

/// Pseudocolour map for Image/Contour/Surf/Mesh data.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Colormap {
    /// Blue→teal→yellow ramp (parula-like default).
    #[default]
    Parula,
    /// Blue→cyan→yellow→red ramp.
    Jet,
    /// White→red→black ramp.
    Hot,
    /// Black→white.
    Gray,
}

impl Colormap {
    /// Map `t ∈ [0,1]` to a colour.
    pub fn map(self, t: f64) -> Rgb {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        let (r, g, b) = match self {
            // Anchor rows are coarse approximations of the classic ramps.
            Colormap::Parula => {
                lerp_stops(t, &[(0.208, 0.166, 0.529), (0.0, 0.47, 0.68), (0.06, 0.72, 0.66), (0.66, 0.79, 0.25), (0.98, 0.96, 0.13)])
            }
            Colormap::Jet => lerp_stops(t, &[(0.0, 0.0, 0.5), (0.0, 0.0, 1.0), (0.0, 1.0, 1.0), (1.0, 1.0, 0.0), (1.0, 0.0, 0.0), (0.5, 0.0, 0.0)]),
            Colormap::Hot => lerp_stops(t, &[(0.0, 0.0, 0.0), (0.9, 0.0, 0.0), (1.0, 0.85, 0.0), (1.0, 1.0, 1.0)]),
            Colormap::Gray => lerp_stops(t, &[(0.0, 0.0, 0.0), (1.0, 1.0, 1.0)]),
        };
        Rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
    }

    /// Builtin name lookup for `colormap(name)`.
    pub fn named(name: &str) -> Option<Colormap> {
        match name {
            "parula" => Some(Colormap::Parula),
            "jet" => Some(Colormap::Jet),
            "hot" => Some(Colormap::Hot),
            "gray" | "grey" => Some(Colormap::Gray),
            _ => None,
        }
    }

    /// Name used by `colormap` when read back.
    pub fn name(self) -> &'static str {
        match self {
            Colormap::Parula => "parula",
            Colormap::Jet => "jet",
            Colormap::Hot => "hot",
            Colormap::Gray => "gray",
        }
    }
}

/// Piecewise-linear RGB ramp through `stops` (positions evenly spaced).
fn lerp_stops(t: f64, stops: &[(f64, f64, f64)]) -> (f64, f64, f64) {
    let n = stops.len();
    if n < 2 {
        return stops.first().copied().unwrap_or((0.0, 0.0, 0.0));
    }
    let f = t * (n - 1) as f64;
    let i = (f.floor() as usize).min(n - 2);
    let f = f - i as f64;
    let (a, b) = (stops[i], stops[i + 1]);
    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, a.2 + (b.2 - a.2) * f)
}

impl Figure {
    pub fn new() -> Self {
        Figure {
            series: Vec::new(),
            title: None,
            xlabel: None,
            ylabel: None,
            legend: Vec::new(),
            grid: false,
            xlim: None,
            ylim: None,
            zlim: None,
            zlabel: None,
            hold: false,
            xlog: false,
            ylog: false,
            view: (-37.5, 30.0),
            cmap: Colormap::Parula,
            clim: None,
            colorbar: false,
            yrev: false,
            boxed: true,
            sub: None,
            drawn: 0,
        }
    }

    /// `clf` — empty the figure, keep the window/number.
    pub fn clear(&mut self) {
        *self = Figure::new();
    }

    /// Snapshot the flat axes-level fields into an `Axes`.
    fn axes_snapshot(&self) -> Axes {
        Axes {
            series: self.series.clone(),
            title: self.title.clone(),
            xlabel: self.xlabel.clone(),
            ylabel: self.ylabel.clone(),
            legend: self.legend.clone(),
            grid: self.grid,
            xlim: self.xlim,
            ylim: self.ylim,
            zlim: self.zlim,
            zlabel: self.zlabel.clone(),
            hold: self.hold,
            xlog: self.xlog,
            ylog: self.ylog,
            view: self.view,
            cmap: self.cmap,
            clim: self.clim,
            colorbar: self.colorbar,
            yrev: self.yrev,
            boxed: self.boxed,
            drawn: self.drawn,
        }
    }

    /// Restore `a` into the flat fields.
    fn axes_restore(&mut self, a: &Axes) {
        self.series = a.series.clone();
        self.title = a.title.clone();
        self.xlabel = a.xlabel.clone();
        self.ylabel = a.ylabel.clone();
        self.legend = a.legend.clone();
        self.grid = a.grid;
        self.xlim = a.xlim;
        self.ylim = a.ylim;
        self.zlim = a.zlim;
        self.zlabel = a.zlabel.clone();
        self.hold = a.hold;
        self.xlog = a.xlog;
        self.ylog = a.ylog;
        self.view = a.view;
        self.cmap = a.cmap;
        self.clim = a.clim;
        self.colorbar = a.colorbar;
        self.yrev = a.yrev;
        self.boxed = a.boxed;
        self.drawn = a.drawn;
    }

    /// `subplot(rows, cols, pos)` — `pos` is a list of 1-based cell numbers
    /// (row-major); a multi-cell pos makes one axes spanning the bounding box.
    /// `subplot(311)`-style packing is decoded by the caller.
    pub fn subplot(&mut self, rows: usize, cols: usize, pos: &[usize]) {
        if rows == 0 || cols == 0 || pos.is_empty() {
            return;
        }
        let cells = rows * cols;
        // Row-major positions → cell coords; bounding box of the selection.
        let mut r0 = usize::MAX;
        let mut c0 = usize::MAX;
        let mut r1 = 0;
        let mut c1 = 0;
        let mut any = false;
        for &p in pos {
            if p == 0 || p > cells {
                continue;
            }
            let (r, c) = ((p - 1) / cols, (p - 1) % cols);
            r0 = r0.min(r);
            c0 = c0.min(c);
            r1 = r1.max(r);
            c1 = c1.max(c);
            any = true;
        }
        if !any {
            return;
        }
        let rect = (r0, c0, r1, c1);
        // New grid (or first use): the outgoing flat axes is treated as cell 1.
        if self.sub.as_ref().map(|s| (s.rows, s.cols)) != Some((rows, cols)) {
            self.sub = Some(Subplot { rows, cols, cur_rect: (0, 0, 0, 0), tiles: Vec::new() });
        }
        // Snapshot before borrowing `sub` mutably.
        let had_content = self.axes_has_content();
        let outgoing_snap = self.axes_snapshot();
        let mut incoming: Option<Axes> = None;
        {
            let Some(sub) = self.sub.as_mut() else { return };
            let outgoing = sub.cur_rect;
            // Stash the outgoing axes if it carries anything worth keeping.
            if had_content {
                sub.tiles.retain(|t| (t.r0, t.c0, t.r1, t.c1) != outgoing);
                sub.tiles.push(Tile { r0: outgoing.0, c0: outgoing.1, r1: outgoing.2, c1: outgoing.3, axes: outgoing_snap });
            }
            // Reuse a stashed tile that covers the requested cells (selecting
            // cell 3 reactivates an axes created with `p = [3 4]`); otherwise
            // the position gets a fresh axes.
            let idx = sub.tiles.iter().position(|t| t.r0 <= rect.0 && t.c0 <= rect.1 && t.r1 >= rect.2 && t.c1 >= rect.3);
            match idx {
                Some(i) => {
                    let t = sub.tiles.remove(i);
                    sub.cur_rect = (t.r0, t.c0, t.r1, t.c1);
                    incoming = Some(t.axes);
                }
                None => sub.cur_rect = rect,
            }
        }
        self.axes_restore(&incoming.unwrap_or_default());
    }

    /// True if the flat axes holds anything beyond defaults.
    fn axes_has_content(&self) -> bool {
        !self.series.is_empty()
            || self.title.is_some()
            || self.xlabel.is_some()
            || self.ylabel.is_some()
            || !self.legend.is_empty()
            || self.xlim.is_some()
            || self.ylim.is_some()
            || self.grid
            || self.xlog
            || self.ylog
    }

    /// Colour for the next series when no explicit spec colour was given.
    fn next_color(&mut self) -> Rgb {
        let c = CYCLE[self.drawn % CYCLE.len()];
        self.drawn += 1;
        c
    }

    /// Add a series; assigns a cycle colour when the spec has none.
    pub fn push(&mut self, kind: SeriesKind, x: Vec<f64>, y: Vec<f64>, spec: LineSpec) {
        self.push_err(kind, x, y, None, spec);
    }

    /// Add a series with error-bar data (`errorbar`).
    pub fn push_err(&mut self, kind: SeriesKind, x: Vec<f64>, y: Vec<f64>, err: Option<Vec<f64>>, mut spec: LineSpec) {
        if spec.color.is_none() {
            spec.color = Some(self.next_color());
        }
        // Non-line kinds default to no connecting dash; stairs and area do
        // draw their outline so they keep the solid default.
        if matches!(kind, SeriesKind::Scatter | SeriesKind::Stem | SeriesKind::Bar | SeriesKind::ErrorBar) && spec.dash == DashStyle::Solid {
            spec.dash = DashStyle::None;
        }
        self.series.push(Series { kind, x, y, z: Vec::new(), zshape: (0, 0), levels: Vec::new(), labels: Vec::new(), err, rgba: None, spec });
    }

    /// `plot3` — a 3-D point-list series.
    pub fn push_xyz(&mut self, x: Vec<f64>, y: Vec<f64>, z: Vec<f64>, mut spec: LineSpec) {
        if spec.color.is_none() {
            spec.color = Some(self.next_color());
        }
        self.series.push(Series {
            kind: SeriesKind::Line3,
            x,
            y,
            z,
            zshape: (0, 0),
            levels: Vec::new(),
            labels: Vec::new(),
            err: None,
            rgba: None,
            spec,
        });
    }

    /// `imagesc`/`contour`/`surf`/`mesh` — grid data with column-major `z`,
    /// `x` = column coords (len `cols`), `y` = row coords (len `rows`).
    /// (`surf`/`mesh` store per-vertex x/y flattened to `rows*cols`.)
    #[allow(clippy::too_many_arguments)]
    pub fn push_grid(
        &mut self,
        kind: SeriesKind,
        x: Vec<f64>,
        y: Vec<f64>,
        z: Vec<f64>,
        rows: usize,
        cols: usize,
        levels: Vec<f64>,
        mut spec: LineSpec,
    ) {
        if spec.color.is_none() {
            spec.color = Some(self.next_color());
        }
        self.series.push(Series { kind, x, y, z, zshape: (rows, cols), levels, labels: Vec::new(), err: None, rgba: None, spec });
    }

    /// `pie` — slice values and labels.
    pub fn push_pie(&mut self, values: Vec<f64>, labels: Vec<String>) {
        let mut spec = LineSpec::new();
        spec.color = Some(self.next_color());
        self.series.push(Series {
            kind: SeriesKind::Pie,
            x: Vec::new(),
            y: values,
            z: Vec::new(),
            zshape: (0, 0),
            levels: Vec::new(),
            labels,
            err: None,
            rgba: None,
            spec,
        });
    }

    /// `text` — a string anchored at the data point (`x`, `y`). The text
    /// colour defaults to black rather than consuming the line cycle;
    /// `spec.size` is the font size in px when set.
    pub fn push_text(&mut self, x: f64, y: f64, label: String, mut spec: LineSpec) {
        if spec.color.is_none() {
            spec.color = Some(Rgb(0x00, 0x00, 0x00));
        }
        self.series.push(Series {
            kind: SeriesKind::Text,
            x: vec![x],
            y: vec![y],
            z: Vec::new(),
            zshape: (0, 0),
            levels: Vec::new(),
            labels: vec![label],
            err: None,
            rgba: None,
            spec,
        });
    }

    /// `imshow` — a true-colour raster: `rgba` is row-major RGBA8 over a
    /// `rows`×`cols` pixel grid; `x`/`y` hold the column/row cell-centre
    /// coordinates (same convention as `Image`).
    pub fn push_rgba(&mut self, x: Vec<f64>, y: Vec<f64>, rows: usize, cols: usize, rgba: Vec<u8>) {
        self.series.push(Series {
            kind: SeriesKind::ImageRgb,
            x,
            y,
            z: Vec::new(),
            zshape: (rows, cols),
            levels: Vec::new(),
            labels: Vec::new(),
            err: None,
            rgba: Some(rgba),
            spec: LineSpec::new(),
        });
    }
}

impl Default for Figure {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SVG rendering
// ---------------------------------------------------------------------------

const W: f64 = 680.0;
const H: f64 = 460.0;
const ML: f64 = 74.0;
const MR: f64 = 26.0;
const MT: f64 = 46.0;
const MB: f64 = 66.0;
const TICKS: usize = 6;

fn axes() -> (f64, f64, f64, f64) {
    (ML, MT, W - ML - MR, H - MT - MB)
}

/// The fixed SVG canvas size (width, height) `to_svg` emits, in px.
pub fn svg_size() -> (f64, f64) {
    (W, H)
}

/// The axes rectangle (x, y, w, h) in SVG pixels for the single-axes
/// layout — interactive viewers map screen positions through it.
pub fn axes_rect() -> (f64, f64, f64, f64) {
    axes()
}

/// Whether the figure is a single 2-D axes that supports x/y view
/// interaction — no subplot tiling, no pie-only or 3-D series.
pub fn is_xy_view(fig: &Figure) -> bool {
    fig.sub.is_none() && !fig.series.iter().any(Series::is_3d) && (fig.series.is_empty() || !fig.series.iter().all(|s| s.kind == SeriesKind::Pie))
}

/// Resolved 2-D view limits `(x0, x1, y0, y1)` in *drawn* (axis) space —
/// log axes are already log10-transformed; explicit `xlim`/`ylim` win over
/// the padded data extent. This is exactly what `draw_axes` maps through.
pub fn view_limits(fig: &Figure) -> (f64, f64, f64, f64) {
    view2d(fig)
}

/// The limit computation shared by `draw_axes` and [`view_limits`]: data
/// extent (image cells padded a half-step, error bars, zero-baseline
/// anchors) resolved through [`limits`], in transformed space.
fn view2d(fig: &Figure) -> (f64, f64, f64, f64) {
    let tx = |v: f64| if fig.xlog { v.log10() } else { v };
    let ty = |v: f64| if fig.ylog { v.log10() } else { v };

    let mut xmin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    for s in &fig.series {
        // Image grids: coordinate vectors hold cell centres — extend the
        // range by half a cell so the image edge lands on the axis limit.
        let (xpad, ypad) = if matches!(s.kind, SeriesKind::Image | SeriesKind::ImageRgb) { (half_step(&s.x), half_step(&s.y)) } else { (0.0, 0.0) };
        for &v in &s.x {
            let t = tx(v);
            if t.is_finite() {
                xmin = xmin.min(t - xpad);
                xmax = xmax.max(t + xpad);
            }
        }
        let n = s.y.len();
        for i in 0..n {
            if let Some(&v) = s.y.get(i) {
                let t = ty(v);
                if t.is_finite() {
                    ymin = ymin.min(t - ypad);
                    ymax = ymax.max(t + ypad);
                }
                if let Some(err) = &s.err {
                    if let Some(&e) = err.get(i) {
                        for cand in [ty(v - e), ty(v + e)] {
                            if cand.is_finite() {
                                ymin = ymin.min(cand);
                                ymax = ymax.max(cand);
                            }
                        }
                    }
                }
            }
        }
        // Stems, bars and areas anchor to the zero baseline (linear axes
        // only — zero is unreachable on a log axis).
        if !fig.ylog && matches!(s.kind, SeriesKind::Stem | SeriesKind::Bar | SeriesKind::Area) {
            ymin = ymin.min(0.0);
            ymax = ymax.max(0.0);
        }
    }
    let (x0, x1) = limits(xmin, xmax, fig.xlim.map(|(a, b)| (tx(a), tx(b))));
    let (y0, y1) = limits(ymin, ymax, fig.ylim.map(|(a, b)| (ty(a), ty(b))));
    (x0, x1, y0, y1)
}

/// Escape text for XML.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Standard base64 (no line breaks) for SVG `data:` URIs.
fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for ch in data.chunks(3) {
        let (b0, b1, b2) = (ch.first().copied().unwrap_or(0) as u32, ch.get(1).copied().unwrap_or(0) as u32, ch.get(2).copied().unwrap_or(0) as u32);
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if ch.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if ch.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Row-major RGBA8 → PNG bytes for an `<image>` data URI. A `None`
/// return skips the series rather than failing the whole render.
fn encode_rgba_png(w: usize, h: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = (u32::try_from(w).ok()?, u32::try_from(h).ok()?);
    let need = w.checked_mul(h)?.checked_mul(4)? as usize;
    if w == 0 || h == 0 || rgba.len() < need {
        return None;
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(rgba.get(..need)?).ok()?;
    }
    Some(out)
}

/// Compact tick label: `3`, `0.5`, `1e-06`.
fn tick_label(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    if v.abs() >= 1e-4 && v.abs() < 1e6 {
        let s = format!("{v:.4}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        return if s.is_empty() { "0".to_string() } else { s.to_string() };
    }
    format!("{v:e}")
}

/// A "nice" tick step ≥ `raw` from the 1/2/2.5/5/10 family.
fn nice_step(raw: f64) -> f64 {
    if raw <= 0.0 || !raw.is_finite() {
        return 1.0;
    }
    let k = raw.log10().floor();
    let base = 10f64.powf(k);
    for m in [1.0, 2.0, 2.5, 5.0, 10.0] {
        let s = m * base;
        if s >= raw * (1.0 - 1e-12) {
            return s;
        }
    }
    10.0 * base
}

fn ticks(lo: f64, hi: f64, count: usize) -> Vec<f64> {
    if !lo.is_finite() || !hi.is_finite() || lo >= hi {
        return vec![lo];
    }
    let step = nice_step((hi - lo) / count.max(1) as f64);
    let mut out = Vec::new();
    let mut v = (lo / step).ceil() * step;
    let mut guard = 0;
    while v <= hi + step * 1e-9 && guard < 100 {
        out.push(v);
        v += step;
        guard += 1;
    }
    out
}

/// Axis limits: explicit override, else data min/max padded by 4%.
fn limits(data_min: f64, data_max: f64, explicit: Option<(f64, f64)>) -> (f64, f64) {
    if let Some(l) = explicit {
        return l;
    }
    if !data_min.is_finite() || !data_max.is_finite() {
        return (0.0, 1.0);
    }
    if (data_max - data_min).abs() < 1e-15 {
        let d = if data_min == 0.0 { 1.0 } else { data_min.abs() * 0.5 };
        return (data_min - d, data_max + d);
    }
    let pad = (data_max - data_min) * 0.04;
    (data_min - pad, data_max + pad)
}

/// Log ticks for an axis spanning transformed `[lo, hi]` (which is already
/// log10 of the data): one tick per integer decade, labelled `1e{k}`.
fn log_ticks(lo: f64, hi: f64) -> Vec<(f64, String)> {
    let mut out = Vec::new();
    let mut k = lo.ceil();
    let mut guard = 0;
    while k <= hi + 1e-9 && guard < 40 {
        out.push((k, format!("1e{k:.0}")));
        k += 1.0;
        guard += 1;
    }
    if out.is_empty() {
        out.push((lo, format!("1e{lo:.0}")));
    }
    out
}

/// Render the figure to an SVG document.
pub fn to_svg(fig: &Figure) -> String {
    let mut o = String::with_capacity(4096);
    o.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{W:.0}\" height=\"{H:.0}\" \
viewBox=\"0 0 {W:.0} {H:.0}\" font-family=\"Helvetica, Arial, sans-serif\">\n"
    ));
    o.push_str(&format!("<rect width=\"{W:.0}\" height=\"{H:.0}\" fill=\"#ffffff\"/>\n"));

    match &fig.sub {
        None => {
            let (ax, ay, aw, ah) = axes();
            draw_axes(&mut o, fig, ax, ay, aw, ah, "ax");
        }
        Some(sub) => {
            let cw = W / sub.cols as f64;
            let ch = H / sub.rows as f64;
            for (i, t) in sub.tiles.iter().enumerate() {
                let (ax, ay, aw, ah) = tile_rect(cw, ch, (t.r0, t.c0, t.r1, t.c1));
                let fv = t.axes.to_figure();
                draw_axes(&mut o, &fv, ax, ay, aw, ah, &format!("ax{i}"));
            }
            // The live axes is held in the flat fields at `cur_rect`.
            let live = fig.axes_snapshot();
            let (ax, ay, aw, ah) = tile_rect(cw, ch, sub.cur_rect);
            let fv = live.to_figure();
            draw_axes(&mut o, &fv, ax, ay, aw, ah, &format!("ax{}", sub.tiles.len()));
        }
    }
    o.push_str("</svg>\n");
    o
}

/// Axes rect inside a subplot cell span — proportional margins leave room for
/// ticks, labels and the title.
fn tile_rect(cw: f64, ch: f64, rect: (usize, usize, usize, usize)) -> (f64, f64, f64, f64) {
    let tx = rect.1 as f64 * cw;
    let ty = rect.0 as f64 * ch;
    let tw = (rect.3 - rect.1 + 1) as f64 * cw;
    let th = (rect.2 - rect.0 + 1) as f64 * ch;
    (tx + tw * 0.14, ty + th * 0.16, tw * 0.82, th * 0.65)
}

/// Draw one axes (ticks, frame, series, labels, legend) into `o` at the given
/// pixel rect. `clip_id` must be unique per axes in the document.
fn draw_axes(o: &mut String, fig: &Figure, ax: f64, ay: f64, aw: f64, ah: f64, clip_id: &str) {
    // Specialised paths: all-pie axes draw a centred chart; any 3-D series
    // switches to the projected-axes renderer.
    if !fig.series.is_empty() && fig.series.iter().all(|s| s.kind == SeriesKind::Pie) {
        draw_pie_axes(o, fig, ax, ay, aw, ah);
        return;
    }
    if fig.series.iter().any(Series::is_3d) {
        draw_axes3d(o, fig, ax, ay, aw, ah, clip_id);
        return;
    }
    // Axis transforms: identity or log10. Points that are non-positive under
    // a log axis are dropped when drawing.
    let tx = |v: f64| if fig.xlog { v.log10() } else { v };
    let ty = |v: f64| if fig.ylog { v.log10() } else { v };

    let (x0, x1, y0, y1) = view2d(fig);
    let xspan = (x1 - x0).max(f64::EPSILON);
    let yspan = (y1 - y0).max(f64::EPSILON);
    let sx = |v: f64| ax + (tx(v) - x0) / xspan * aw;
    // `yrev` (imagesc / `axis ij`) puts the lowest y at the top.
    let sy = |v: f64| if fig.yrev { ay + (ty(v) - y0) / yspan * ah } else { ay + ah - (ty(v) - y0) / yspan * ah };

    // Grid + ticks. Positions are in transformed space; log axes get one
    // decade per tick with a `1e{k}` label.
    let tick_col = "#1a1a1a";
    let xticks: Vec<(f64, String)> =
        if fig.xlog { log_ticks(x0, x1) } else { ticks(x0, x1, TICKS).into_iter().map(|t| (t, tick_label(t))).collect() };
    let yticks: Vec<(f64, String)> =
        if fig.ylog { log_ticks(y0, y1) } else { ticks(y0, y1, TICKS).into_iter().map(|t| (t, tick_label(t))).collect() };
    for (t, lab) in &xticks {
        let p = ax + (t - x0) / xspan * aw;
        if fig.grid {
            o.push_str(&format!("<line x1=\"{p:.2}\" y1=\"{ay:.2}\" x2=\"{p:.2}\" y2=\"{:.2}\" stroke=\"#e0e0e0\" stroke-width=\"1\"/>\n", ay + ah));
        }
        o.push_str(&format!(
            "<line x1=\"{p:.2}\" y1=\"{:.2}\" x2=\"{p:.2}\" y2=\"{:.2}\" stroke=\"{tick_col}\" stroke-width=\"1\"/>\n",
            ay + ah,
            ay + ah + 5.0
        ));
        o.push_str(&format!(
            "<text x=\"{p:.2}\" y=\"{:.2}\" font-size=\"11\" fill=\"{tick_col}\" text-anchor=\"middle\">{}</text>\n",
            ay + ah + 18.0,
            esc(lab)
        ));
    }
    for (t, lab) in &yticks {
        let p = ay + ah - (t - y0) / yspan * ah;
        if fig.grid {
            o.push_str(&format!("<line x1=\"{ax:.2}\" y1=\"{p:.2}\" x2=\"{:.2}\" y2=\"{p:.2}\" stroke=\"#e0e0e0\" stroke-width=\"1\"/>\n", ax + aw));
        }
        o.push_str(&format!("<line x1=\"{:.2}\" y1=\"{p:.2}\" x2=\"{ax:.2}\" y2=\"{p:.2}\" stroke=\"{tick_col}\" stroke-width=\"1\"/>\n", ax - 5.0));
        o.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"11\" fill=\"{tick_col}\" text-anchor=\"end\">{}</text>\n",
            ax - 8.0,
            p + 4.0,
            esc(lab)
        ));
    }

    // Axes box (`box off` hides the frame).
    if fig.boxed {
        o.push_str(&format!(
            "<rect x=\"{ax:.2}\" y=\"{ay:.2}\" width=\"{aw:.2}\" height=\"{ah:.2}\" \
fill=\"none\" stroke=\"{tick_col}\" stroke-width=\"1.2\"/>\n"
        ));
    }
    // Clip subsequent drawing to the axes box.
    o.push_str(&format!("<clipPath id=\"{clip_id}\"><rect x=\"{ax:.2}\" y=\"{ay:.2}\" width=\"{aw:.2}\" height=\"{ah:.2}\"/></clipPath>\n"));
    o.push_str(&format!("<g clip-path=\"url(#{clip_id})\">\n"));

    for s in &fig.series {
        let col = s.spec.color.unwrap_or(CYCLE[0]).hex();
        let n = s.x.len().min(s.y.len());
        // Transformed pixel positions; `None` = dropped (non-finite, e.g. a
        // non-positive coordinate on a log axis).
        let px: Vec<f64> = s.x.iter().take(n).map(|&v| sx(v)).collect();
        let py: Vec<f64> = s.y.iter().take(n).map(|&v| sy(v)).collect();
        let pt = |i: usize| -> Option<(f64, f64)> {
            match (px.get(i), py.get(i)) {
                (Some(&x), Some(&y)) if x.is_finite() && y.is_finite() => Some((x, y)),
                _ => None,
            }
        };
        match s.kind {
            SeriesKind::Bar => {
                // Bar width: 80% of the smallest x gap (in data space).
                let mut gap = f64::INFINITY;
                for i in 1..n {
                    gap = gap.min((s.x[i] - s.x[i - 1]).abs());
                }
                let bw = if gap.is_finite() { gap * 0.8 } else { (x1 - x0) * 0.05 };
                let bw_px = bw / xspan * aw;
                let base = sy(0.0).clamp(ay, ay + ah);
                for i in 0..n {
                    if let Some((cx, top)) = pt(i) {
                        let (yt, hh) = if top <= base { (top, base - top) } else { (base, top - base) };
                        o.push_str(&format!(
                            "<rect x=\"{:.2}\" y=\"{yt:.2}\" width=\"{bw_px:.2}\" height=\"{hh:.2}\" \
fill=\"{col}\" fill-opacity=\"0.75\" stroke=\"{col}\" stroke-width=\"1\"/>\n",
                            cx - bw_px / 2.0
                        ));
                    }
                }
            }
            SeriesKind::Stem => {
                let base = sy(0.0).clamp(ay, ay + ah);
                for i in 0..n {
                    if let Some((px_, py_)) = pt(i) {
                        o.push_str(&format!(
                            "<line x1=\"{px_:.2}\" y1=\"{base:.2}\" x2=\"{px_:.2}\" y2=\"{py_:.2}\" stroke=\"{col}\" stroke-width=\"1.2\"/>\n"
                        ));
                    }
                }
            }
            SeriesKind::Area => {
                // Filled polygon: along the curve, then back to baseline.
                if n >= 1 {
                    let base = sy(0.0).clamp(ay, ay + ah);
                    let mut d = String::new();
                    let mut first: Option<(f64, f64)> = None;
                    let mut last: Option<(f64, f64)> = None;
                    for i in 0..n {
                        if let Some((x, y)) = pt(i) {
                            if first.is_none() {
                                d.push_str(&format!("M{x:.2},{base:.2}"));
                            }
                            d.push_str(&format!("L{x:.2},{y:.2}"));
                            if first.is_none() {
                                first = Some((x, y));
                            }
                            last = Some((x, y));
                        }
                    }
                    if let Some((lx, _)) = last {
                        d.push_str(&format!("L{lx:.2},{base:.2}Z"));
                        o.push_str(&format!("<path d=\"{d}\" fill=\"{col}\" fill-opacity=\"0.35\" stroke=\"{col}\" stroke-width=\"1.4\"/>\n"));
                    }
                }
            }
            SeriesKind::ErrorBar => {
                // Vertical bar y±e with caps, plus a centre marker.
                let cap = 4.0;
                for i in 0..n {
                    if let Some((x, y)) = pt(i) {
                        if let Some(err) = &s.err {
                            if let Some(&e) = err.get(i) {
                                let (lo, hi) = (sy(s.y[i] - e), sy(s.y[i] + e));
                                if lo.is_finite() && hi.is_finite() {
                                    o.push_str(&format!(
                                        "<line x1=\"{x:.2}\" y1=\"{lo:.2}\" x2=\"{x:.2}\" y2=\"{hi:.2}\" stroke=\"{col}\" stroke-width=\"1.2\"/>\n\
                                         <line x1=\"{:.2}\" y1=\"{lo:.2}\" x2=\"{:.2}\" y2=\"{lo:.2}\" stroke=\"{col}\" stroke-width=\"1.2\"/>\n\
                                         <line x1=\"{:.2}\" y1=\"{hi:.2}\" x2=\"{:.2}\" y2=\"{hi:.2}\" stroke=\"{col}\" stroke-width=\"1.2\"/>\n",
                                        x - cap,
                                        x + cap,
                                        x - cap,
                                        x + cap
                                    ));
                                }
                            }
                        }
                        draw_marker(o, s.spec.marker.unwrap_or(Marker::Circle), x, y, &col);
                    }
                }
            }
            SeriesKind::Image => {
                let (rows, cols) = s.zshape;
                let (cl, ch) = caxis(fig);
                let cspan = (ch - cl).abs().max(f64::EPSILON);
                // Subsample stride so the SVG stays a reasonable size.
                let stride = ((rows.saturating_mul(cols)) as f64 / 40_000.0).sqrt().ceil().max(1.0) as usize;
                let (dx, dy) = (half_step(&s.x), half_step(&s.y));
                for r in (0..rows).step_by(stride) {
                    for c in (0..cols).step_by(stride) {
                        let Some(&zv) = s.z.get(c * rows + r) else { continue };
                        if !zv.is_finite() {
                            continue;
                        }
                        let (Some(&xc), Some(&yc)) = (s.x.get(c), s.y.get(r)) else { continue };
                        let fill = fig.cmap.map((zv - cl) / cspan).hex();
                        // Cell spans `stride` cells on each side (subsampling merges them).
                        let (xa, xb) = (xc - dx * stride as f64, xc + dx * stride as f64);
                        let (ya, yb) = (yc - dy * stride as f64, yc + dy * stride as f64);
                        let (px0, px1) = (sx(xa), sx(xb));
                        let (py0, py1) = (sy(ya), sy(yb));
                        let (rx, rw) = (px0.min(px1), (px1 - px0).abs());
                        let (ry, rh) = (py0.min(py1), (py1 - py0).abs());
                        o.push_str(&format!("<rect x=\"{rx:.2}\" y=\"{ry:.2}\" width=\"{rw:.2}\" height=\"{rh:.2}\" fill=\"{fill}\"/>\n"));
                    }
                }
            }
            SeriesKind::Text => {
                // `<text>` anchored at the data point; y offset lifts the
                // baseline so the glyph box sits centred on the anchor.
                if let Some((x, y)) = pt(0) {
                    let size = s.spec.size.unwrap_or(12.0).clamp(1.0, 400.0);
                    let label = s.labels.first().map(String::as_str).unwrap_or("");
                    o.push_str(&format!(
                        "<text x=\"{x:.2}\" y=\"{:.2}\" font-size=\"{size:.1}\" fill=\"{col}\">{}</text>\n",
                        y + size * 0.35,
                        esc(label)
                    ));
                }
            }
            SeriesKind::ImageRgb => {
                // True-colour raster: encode the RGBA payload to PNG and
                // embed it as a data-URI `<image>` spanning the cell edges.
                let (rows, cols) = s.zshape;
                if let (Some(rgba), Some(&xa), Some(&xb), Some(&ya), Some(&yb)) = (s.rgba.as_ref(), s.x.first(), s.x.last(), s.y.first(), s.y.last())
                {
                    if let Some(png) = encode_rgba_png(cols, rows, rgba) {
                        let (dx, dy) = (half_step(&s.x), half_step(&s.y));
                        let (px0, px1) = (sx(xa - dx), sx(xb + dx));
                        let (py0, py1) = (sy(ya - dy), sy(yb + dy));
                        let (rx, rw) = (px0.min(px1), (px1 - px0).abs());
                        let (ry, rh) = (py0.min(py1), (py1 - py0).abs());
                        if rx.is_finite() && ry.is_finite() && rw > 0.0 && rh > 0.0 {
                            o.push_str(&format!(
                                "<image x=\"{rx:.2}\" y=\"{ry:.2}\" width=\"{rw:.2}\" height=\"{rh:.2}\" \
preserveAspectRatio=\"none\" href=\"data:image/png;base64,{}\"/>\n",
                                b64(&png)
                            ));
                        }
                    }
                }
            }
            SeriesKind::Contour => {
                let (cl, ch) = caxis(fig);
                let cspan = (ch - cl).abs().max(f64::EPSILON);
                for lv in contour_levels(s, cl, ch) {
                    let col = fig.cmap.map((lv - cl) / cspan).hex();
                    let mut d = String::new();
                    for (a, b) in contour_segments(s, lv) {
                        let (pa, pb) = ((sx(a.0), sy(a.1)), (sx(b.0), sy(b.1)));
                        if pa.0.is_finite() && pa.1.is_finite() && pb.0.is_finite() && pb.1.is_finite() {
                            d.push_str(&format!("M{:.2},{:.2}L{:.2},{:.2}", pa.0, pa.1, pb.0, pb.1));
                        }
                    }
                    o.push_str(&format!("<path d=\"{d}\" fill=\"none\" stroke=\"{col}\" stroke-width=\"1.2\"/>\n"));
                }
            }
            _ => {}
        }
        // The line itself — Line and Stairs both draw a polyline (stairs
        // inserts the horizontal-then-vertical elbow at each step).
        let draw_line = matches!(s.kind, SeriesKind::Line | SeriesKind::Stairs) && s.spec.dash != DashStyle::None;
        if draw_line && n > 1 {
            let dash = match s.spec.dash {
                DashStyle::Dashed => " stroke-dasharray=\"8,4\"",
                DashStyle::Dotted => " stroke-dasharray=\"2,3\"",
                DashStyle::DashDot => " stroke-dasharray=\"8,4,2,4\"",
                _ => "",
            };
            let mut d = String::new();
            let mut started = false;
            let mut prev: Option<(f64, f64)> = None;
            for i in 0..n {
                let (cx, cy) = match pt(i) {
                    Some(p) => p,
                    None => continue,
                };
                if !started {
                    d.push_str(&format!("M{cx:.2},{cy:.2}"));
                    started = true;
                } else if s.kind == SeriesKind::Stairs {
                    if let Some((_, pyp)) = prev {
                        d.push_str(&format!("L{cx:.2},{pyp:.2}L{cx:.2},{cy:.2}"));
                    } else {
                        d.push_str(&format!("L{cx:.2},{cy:.2}"));
                    }
                } else {
                    d.push_str(&format!("L{cx:.2},{cy:.2}"));
                }
                prev = Some((cx, cy));
            }
            o.push_str(&format!("<path d=\"{d}\" fill=\"none\" stroke=\"{col}\" stroke-width=\"1.6\"{dash}/>\n"));
        }
        // Markers.
        let marker = match s.kind {
            SeriesKind::Scatter | SeriesKind::Stem => Some(s.spec.marker.unwrap_or(Marker::Circle)),
            SeriesKind::ErrorBar => None, // drawn with the bars above
            _ => s.spec.marker,
        };
        if let Some(mk) = marker {
            for i in 0..n {
                if let Some((x, y)) = pt(i) {
                    draw_marker(o, mk, x, y, &col);
                }
            }
        }
    }
    o.push_str("</g>\n");

    draw_labels(o, fig, ax, ay, aw, ah);
    draw_colorbar(o, fig, ax, ay, aw, ah);
}

/// Title, axis labels and the legend box — shared by the 2-D, 3-D and pie
/// renderers.
fn draw_labels(o: &mut String, fig: &Figure, ax: f64, ay: f64, aw: f64, ah: f64) {
    if let Some(t) = &fig.title {
        o.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"15\" font-weight=\"bold\" text-anchor=\"middle\">{}</text>\n",
            ax + aw / 2.0,
            ay - 16.0,
            esc(t)
        ));
    }
    if let Some(t) = &fig.xlabel {
        o.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"13\" text-anchor=\"middle\">{}</text>\n",
            ax + aw / 2.0,
            ay + ah + 40.0,
            esc(t)
        ));
    }
    if let Some(t) = &fig.ylabel {
        o.push_str(&format!(
            "<text transform=\"rotate(-90 {:.2} {:.2})\" x=\"{:.2}\" y=\"{:.2}\" font-size=\"13\" text-anchor=\"middle\">{}</text>\n",
            ax - 52.0,
            ay + ah / 2.0,
            ax - 52.0,
            ay + ah / 2.0,
            esc(t)
        ));
    }

    // Legend box, top-right inside the axes.
    if !fig.legend.is_empty() {
        let rows = fig.legend.len().min(fig.series.len().max(1));
        let lw = fig.legend.iter().take(rows).map(|s| s.chars().count()).max().unwrap_or(0) as f64 * 7.5 + 46.0;
        let lh = rows as f64 * 18.0 + 10.0;
        let lx = ax + aw - lw - 8.0;
        let ly = ay + 8.0;
        o.push_str(&format!(
            "<rect x=\"{lx:.2}\" y=\"{ly:.2}\" width=\"{lw:.2}\" height=\"{lh:.2}\" \
fill=\"#ffffff\" fill-opacity=\"0.9\" stroke=\"#cccccc\"/>\n"
        ));
        for (i, name) in fig.legend.iter().take(rows).enumerate() {
            let y = ly + 14.0 + i as f64 * 18.0;
            let col = fig.series.get(i).and_then(|s| s.spec.color).unwrap_or(CYCLE[i % CYCLE.len()]).hex();
            o.push_str(&format!(
                "<line x1=\"{:.2}\" y1=\"{y:.2}\" x2=\"{:.2}\" y2=\"{y:.2}\" stroke=\"{col}\" stroke-width=\"2\"/>\n",
                lx + 8.0,
                lx + 34.0
            ));
            o.push_str(&format!("<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"11\">{}</text>\n", lx + 40.0, y + 4.0, esc(name)));
        }
    }
}

/// Half the median spacing of a coordinate vector — the cell half-width used
/// to turn `imagesc` centre coordinates into cell edges.
fn half_step(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.5;
    }
    let first = match v.first() {
        Some(&f) => f,
        None => return 0.5,
    };
    let last = match v.last() {
        Some(&l) => l,
        None => return 0.5,
    };
    ((last - first) / (v.len() - 1) as f64).abs() / 2.0
}

/// Colour limits: explicit `clim`, else the min/max of all grid data.
fn caxis(fig: &Figure) -> (f64, f64) {
    if let Some(l) = fig.clim {
        return l;
    }
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for s in &fig.series {
        if s.zshape != (0, 0) {
            for &v in &s.z {
                if v.is_finite() {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return (0.0, 1.0);
    }
    if (hi - lo).abs() < f64::EPSILON {
        return (lo, lo + 1.0);
    }
    (lo, hi)
}

/// Contour levels: explicit `series.levels`, else ~7 nice values spanning the
/// colour range.
fn contour_levels(s: &Series, lo: f64, hi: f64) -> Vec<f64> {
    if !s.levels.is_empty() {
        return s.levels.clone();
    }
    let span = hi - lo;
    if span <= 0.0 || !span.is_finite() {
        return Vec::new();
    }
    let step = nice_step(span / 7.0);
    let mut out = Vec::new();
    let mut v = (lo / step).ceil() * step;
    let mut guard = 0;
    while v <= hi + step * 1e-9 && guard < 200 {
        if v > lo + step * 1e-9 && v < hi - step * 1e-9 {
            out.push(v);
        }
        v += step;
        guard += 1;
    }
    out
}

/// Marching squares over the series grid at `lv` — returns line segments in
/// data coordinates. `z` is column-major `rows × cols`; `x`/`y` hold the
/// column/row coordinates.
fn contour_segments(s: &Series, lv: f64) -> Vec<((f64, f64), (f64, f64))> {
    let (rows, cols) = s.zshape;
    let mut out = Vec::new();
    let z = |r: usize, c: usize| s.z.get(c * rows + r).copied().unwrap_or(f64::NAN);
    for r in 0..rows.saturating_sub(1) {
        for c in 0..cols.saturating_sub(1) {
            let (bl, br) = (z(r, c), z(r, c + 1)); // bottom-left, bottom-right
            let (tl, tr) = (z(r + 1, c), z(r + 1, c + 1)); // top-left, top-right
            if ![bl, br, tl, tr].iter().all(|v| v.is_finite()) {
                continue;
            }
            let (Some(&x0), Some(&x1), Some(&y0), Some(&y1)) = (s.x.get(c), s.x.get(c + 1), s.y.get(r), s.y.get(r + 1)) else { continue };
            // Edge crossing points (data coords): 0 = top, 1 = right, 2 = bottom, 3 = left.
            let lerp = |va: f64, vb: f64| if (vb - va).abs() < f64::EPSILON { 0.5 } else { (lv - va) / (vb - va) };
            let top = (x0 + lerp(tl, tr) * (x1 - x0), y1);
            let right = (x1, y0 + lerp(br, tr) * (y1 - y0));
            let bottom = (x0 + lerp(bl, br) * (x1 - x0), y0);
            let left = (x0, y0 + lerp(bl, tl) * (y1 - y0));
            // Corner bits: bit0 = tl, bit1 = tr, bit2 = br, bit3 = bl.
            let idx = (usize::from(tl > lv)) | (usize::from(tr > lv) << 1) | (usize::from(br > lv) << 2) | (usize::from(bl > lv) << 3);
            match idx {
                1 | 14 => out.push((top, left)),
                2 | 13 => out.push((top, right)),
                3 | 12 => out.push((left, right)),
                4 | 11 => out.push((right, bottom)),
                5 => {
                    // Saddle: the cell centre decides which pair joins.
                    let mid = (bl + br + tl + tr) / 4.0;
                    if mid > lv {
                        out.push((top, right));
                        out.push((left, bottom));
                    } else {
                        out.push((top, left));
                        out.push((right, bottom));
                    }
                }
                6 | 9 => out.push((top, bottom)),
                7 | 8 => out.push((left, bottom)),
                10 => {
                    let mid = (bl + br + tl + tr) / 4.0;
                    if mid > lv {
                        out.push((top, left));
                        out.push((right, bottom));
                    } else {
                        out.push((top, right));
                        out.push((left, bottom));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// The `colorbar` strip — a vertical colormap gradient with min/max labels,
/// drawn right of the axes when `fig.colorbar` and a mapped series exist.
fn draw_colorbar(o: &mut String, fig: &Figure, ax: f64, ay: f64, aw: f64, ah: f64) {
    // True-colour rasters have a zshape but no mapped scalar data.
    let has_mapped = fig.series.iter().any(|s| s.zshape != (0, 0) && s.kind != SeriesKind::ImageRgb);
    if !fig.colorbar || !has_mapped {
        return;
    }
    let (cl, ch) = caxis(fig);
    let bx = ax + aw + 8.0;
    let bw = 14.0;
    // Gradient: top of the bar is the high end of the scale.
    let mut defs = String::from("<linearGradient id=\"cb\" x1=\"0\" y1=\"1\" x2=\"0\" y2=\"0\">");
    for i in 0..=10 {
        let t = i as f64 / 10.0;
        defs.push_str(&format!("<stop offset=\"{t:.1}\" stop-color=\"{}\"/>", fig.cmap.map(t).hex()));
    }
    defs.push_str("</linearGradient>");
    o.push_str(&format!("<defs>{defs}</defs>\n"));
    o.push_str(&format!(
        "<rect x=\"{bx:.2}\" y=\"{ay:.2}\" width=\"{bw:.2}\" height=\"{ah:.2}\" fill=\"url(#cb)\" stroke=\"#1a1a1a\" stroke-width=\"0.6\"/>\n"
    ));
    o.push_str(&format!("<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"10\">{}</text>\n", bx + bw + 4.0, ay + 8.0, tick_label(ch)));
    o.push_str(&format!("<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"10\">{}</text>\n", bx + bw + 4.0, ay + ah, tick_label(cl)));
}

/// Pie chart: wedges proportional to `series.y`, CYCLE colours, labels from
/// `series.labels` (or the legend).
fn draw_pie_axes(o: &mut String, fig: &Figure, ax: f64, ay: f64, aw: f64, ah: f64) {
    let cx = ax + aw / 2.0;
    let cy = ay + ah / 2.0;
    let r = aw.min(ah) * 0.38;
    for s in &fig.series {
        let total: f64 = s.y.iter().map(|v| v.abs()).sum();
        if total <= 0.0 {
            continue;
        }
        let mut a0 = std::f64::consts::FRAC_PI_2; // MATLAB starts at 12 o'clock.
        for (i, &v) in s.y.iter().enumerate() {
            let frac = v.abs() / total;
            let a1 = a0 - frac * std::f64::consts::TAU;
            let (x0, y0) = (cx + r * a0.cos(), cy - r * a0.sin());
            let (x1, y1) = (cx + r * a1.cos(), cy - r * a1.sin());
            let large = usize::from(frac > 0.5);
            let col = CYCLE[i % CYCLE.len()].hex();
            if frac >= 1.0 {
                // A single slice fills the circle — an arc can't express 360°.
                o.push_str(&format!("<circle cx=\"{cx:.2}\" cy=\"{cy:.2}\" r=\"{r:.2}\" fill=\"{col}\" stroke=\"#ffffff\" stroke-width=\"1\"/>\n"));
            } else {
                o.push_str(&format!(
                    "<path d=\"M{cx:.2},{cy:.2}L{x0:.2},{y0:.2}A{r:.2},{r:.2} 0 {large} 1 {x1:.2},{y1:.2}Z\" \
fill=\"{col}\" stroke=\"#ffffff\" stroke-width=\"1\"/>\n"
                ));
            }
            // Slice label at the wedge's mid-angle, just outside the circle.
            let mid = (a0 + a1) / 2.0;
            if let Some(lab) = s.labels.get(i) {
                let (lx, ly) = (cx + r * 1.18 * mid.cos(), cy - r * 1.18 * mid.sin());
                let anchor = if mid.cos() > 0.25 {
                    "start"
                } else if mid.cos() < -0.25 {
                    "end"
                } else {
                    "middle"
                };
                o.push_str(&format!("<text x=\"{lx:.2}\" y=\"{ly:.2}\" font-size=\"11\" text-anchor=\"{anchor}\">{}</text>\n", esc(lab)));
            }
            a0 = a1;
        }
    }
    draw_labels(o, fig, ax, ay, aw, ah);
}

/// The 8 corners of the data box, indexed `x*4 + y*2 + z` (lo=0/hi=1 each).
fn box_corners(x0: f64, x1: f64, y0: f64, y1: f64, z0: f64, z1: f64) -> Vec<(f64, f64, f64)> {
    let mut out = Vec::with_capacity(8);
    for &x in &[x0, x1] {
        for &y in &[y0, y1] {
            for &z in &[z0, z1] {
                out.push((x, y, z));
            }
        }
    }
    out
}

/// Projected 3-D axes: data box wireframe, ticks on the three edges nearest
/// the camera, and `plot3`/`mesh`/`surf` content projected orthographically
/// by `fig.view` (azimuth/elevation degrees).
fn draw_axes3d(o: &mut String, fig: &Figure, ax: f64, ay: f64, aw: f64, ah: f64, clip_id: &str) {
    // Data bounds across all 3-D series (grid kinds store per-vertex x/y/z
    // flattened — see push_grid_from_mesh in the caller).
    let mut b = [f64::INFINITY; 3];
    let mut t = [f64::NEG_INFINITY; 3];
    for s in &fig.series {
        if !s.is_3d() {
            continue;
        }
        for (i, src) in [&s.x, &s.y, &s.z].iter().enumerate() {
            for &v in src.iter() {
                if v.is_finite() {
                    b[i] = b[i].min(v);
                    t[i] = t[i].max(v);
                }
            }
        }
    }
    let (x0, x1) = limits(b[0], t[0], fig.xlim);
    let (y0, y1) = limits(b[1], t[1], fig.ylim);
    let (z0, z1) = limits(b[2], t[2], fig.zlim);

    // Orthographic view: camera dir from (az, el), screen basis r/s.
    let (az, el) = (fig.view.0.to_radians(), fig.view.1.to_radians());
    let (cx, cy, cz) = (el.cos() * az.sin(), -el.cos() * az.cos(), el.sin());
    let hl = (cx * cx + cy * cy).sqrt();
    let (rx, ry) = if hl > 1e-9 { (-cy / hl, cx / hl) } else { (1.0, 0.0) };
    let (sx_, sy_, sz) = (-cz * ry, cz * rx, cx * ry - cy * rx);
    let norm = |v: f64, lo: f64, hi: f64| {
        let span = (hi - lo).abs().max(f64::EPSILON);
        (v - (lo + hi) / 2.0) / span * 2.0
    };
    // Project data point → (u, v, depth).
    let prj = |x: f64, y: f64, z: f64| {
        let (xn, yn, zn) = (norm(x, x0, x1), norm(y, y0, y1), norm(z, z0, z1));
        (rx * xn + ry * yn, sx_ * xn + sy_ * yn + sz * zn, cx * xn + cy * yn + cz * zn)
    };
    // Fit: project the 8 box corners to find the u/v extent.
    let corners: Vec<(f64, f64, f64)> = box_corners(x0, x1, y0, y1, z0, z1).into_iter().map(|(x, y, z)| prj(x, y, z)).collect();
    let umin = corners.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
    let umax = corners.iter().map(|c| c.0).fold(f64::NEG_INFINITY, f64::max);
    let vmin = corners.iter().map(|c| c.1).fold(f64::INFINITY, f64::min);
    let vmax = corners.iter().map(|c| c.1).fold(f64::NEG_INFINITY, f64::max);
    let scale = (aw / (umax - umin).max(f64::EPSILON)).min(ah / (vmax - vmin).max(f64::EPSILON)) * 0.9;
    let (umid, vmid) = ((umin + umax) / 2.0, (vmin + vmax) / 2.0);
    let px = |p: (f64, f64, f64)| (ax + aw / 2.0 + (p.0 - umid) * scale, ay + ah / 2.0 - (p.1 - vmid) * scale);

    // Box wireframe (12 edges).
    let boxc = box_corners(x0, x1, y0, y1, z0, z1);
    let edge = |o: &mut String, a: usize, bix: usize, col: &str, wdt: f64| {
        let (pa, pb) = (px(prj(boxc[a].0, boxc[a].1, boxc[a].2)), px(prj(boxc[bix].0, boxc[bix].1, boxc[bix].2)));
        o.push_str(&format!(
            "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{col}\" stroke-width=\"{wdt}\"/>\n",
            pa.0, pa.1, pb.0, pb.1
        ));
    };
    // Corner index: x*4 + y*2 + z.
    for a in 0..8 {
        for &bix in &[a ^ 4, a ^ 2, a ^ 1] {
            if a < bix {
                edge(o, a, bix, "#cccccc", 1.0);
            }
        }
    }
    // Ticks on the three edges nearest the camera: x-edge (y=y0,z=z0),
    // y-edge (x=x0,z=z0), z-edge (x=x0,y=y0) — flipped to the far side when
    // the view quadrant puts them behind.
    let (xn_ear, yn_ear) = (if cx > 0.0 { x1 } else { x0 }, if cy > 0.0 { y1 } else { y0 });
    let tick_col = "#1a1a1a";
    for tv in ticks(x0, x1, 4) {
        let p = px(prj(tv, yn_ear, z0));
        o.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"10\" fill=\"{tick_col}\" text-anchor=\"middle\">{}</text>\n",
            p.0,
            p.1 + 12.0,
            tick_label(tv)
        ));
    }
    for tv in ticks(y0, y1, 4) {
        let p = px(prj(xn_ear, tv, z0));
        o.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"10\" fill=\"{tick_col}\" text-anchor=\"middle\">{}</text>\n",
            p.0,
            p.1 + 12.0,
            tick_label(tv)
        ));
    }
    for tv in ticks(z0, z1, 5) {
        let p = px(prj(xn_ear, yn_ear, tv));
        o.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"10\" fill=\"{tick_col}\" text-anchor=\"end\">{}</text>\n",
            p.0 - 4.0,
            p.1 + 3.0,
            tick_label(tv)
        ));
    }

    o.push_str(&format!("<clipPath id=\"{clip_id}\"><rect x=\"{ax:.2}\" y=\"{ay:.2}\" width=\"{aw:.2}\" height=\"{ah:.2}\"/></clipPath>\n"));
    o.push_str(&format!("<g clip-path=\"url(#{clip_id})\">\n"));
    let (cl, ch) = caxis(fig);
    let cspan = (ch - cl).abs().max(f64::EPSILON);
    for s in &fig.series {
        if !s.is_3d() {
            continue;
        }
        match s.kind {
            SeriesKind::Line3 => {
                let col = s.spec.color.unwrap_or(CYCLE[0]).hex();
                let mut d = String::new();
                let mut started = false;
                for i in 0..s.x.len().min(s.y.len()).min(s.z.len()) {
                    let (Some(&x), Some(&y), Some(&z)) = (s.x.get(i), s.y.get(i), s.z.get(i)) else { continue };
                    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
                        continue;
                    }
                    let p = px(prj(x, y, z));
                    d.push_str(&format!("{}{:.2},{:.2}", if started { "L" } else { "M" }, p.0, p.1));
                    started = true;
                }
                o.push_str(&format!("<path d=\"{d}\" fill=\"none\" stroke=\"{col}\" stroke-width=\"1.6\"/>\n"));
                if let Some(mk) = s.spec.marker {
                    for i in 0..s.x.len().min(s.y.len()).min(s.z.len()) {
                        if let (Some(&x), Some(&y), Some(&z)) = (s.x.get(i), s.y.get(i), s.z.get(i)) {
                            if x.is_finite() && y.is_finite() && z.is_finite() {
                                let p = px(prj(x, y, z));
                                draw_marker(o, mk, p.0, p.1, &col);
                            }
                        }
                    }
                }
            }
            SeriesKind::Mesh | SeriesKind::Surf => {
                let (rows, cols) = s.zshape;
                let vtx = |r: usize, c: usize| -> Option<(f64, f64, f64)> {
                    let i = c * rows + r;
                    match (s.x.get(i), s.y.get(i), s.z.get(i)) {
                        (Some(&x), Some(&y), Some(&z)) if x.is_finite() && y.is_finite() && z.is_finite() => Some((x, y, z)),
                        _ => None,
                    }
                };
                if s.kind == SeriesKind::Mesh {
                    // Wireframe: polylines along both grid directions, each
                    // segment tinted by its mean z through the colormap.
                    for r in 0..rows {
                        for c in 0..cols.saturating_sub(1) {
                            if let (Some(a), Some(bq)) = (vtx(r, c), vtx(r, c + 1)) {
                                let col = fig.cmap.map(((a.2 + bq.2) / 2.0 - cl) / cspan).hex();
                                let (pa, pb) = (px(prj(a.0, a.1, a.2)), px(prj(bq.0, bq.1, bq.2)));
                                o.push_str(&format!(
                                    "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{col}\" stroke-width=\"1\"/>\n",
                                    pa.0, pa.1, pb.0, pb.1
                                ));
                            }
                        }
                    }
                    for c in 0..cols {
                        for r in 0..rows.saturating_sub(1) {
                            if let (Some(a), Some(bq)) = (vtx(r, c), vtx(r + 1, c)) {
                                let col = fig.cmap.map(((a.2 + bq.2) / 2.0 - cl) / cspan).hex();
                                let (pa, pb) = (px(prj(a.0, a.1, a.2)), px(prj(bq.0, bq.1, bq.2)));
                                o.push_str(&format!(
                                    "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{col}\" stroke-width=\"1\"/>\n",
                                    pa.0, pa.1, pb.0, pb.1
                                ));
                            }
                        }
                    }
                } else {
                    // Surf: quads painted far→near (painter's algorithm).
                    let mut quads: Vec<(f64, usize, usize)> = Vec::new();
                    for r in 0..rows.saturating_sub(1) {
                        for c in 0..cols.saturating_sub(1) {
                            let corners4 = [vtx(r, c), vtx(r + 1, c), vtx(r + 1, c + 1), vtx(r, c + 1)];
                            if corners4.iter().all(Option::is_some) {
                                let depth: f64 = corners4.iter().flatten().map(|v| prj(v.0, v.1, v.2).2).sum::<f64>() / 4.0;
                                quads.push((depth, r, c));
                            }
                        }
                    }
                    quads.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    for (_, r, c) in quads {
                        let vs = [vtx(r, c), vtx(r + 1, c), vtx(r + 1, c + 1), vtx(r, c + 1)];
                        let mut d = String::new();
                        let mut mz = 0.0;
                        for (k, v) in vs.iter().enumerate() {
                            if let Some(v) = v {
                                let p = px(prj(v.0, v.1, v.2));
                                d.push_str(&format!("{}{:.2},{:.2}", if k == 0 { "M" } else { "L" }, p.0, p.1));
                                mz += v.2;
                            }
                        }
                        d.push('Z');
                        let fill = fig.cmap.map((mz / 4.0 - cl) / cspan).hex();
                        o.push_str(&format!("<path d=\"{d}\" fill=\"{fill}\" stroke=\"#333333\" stroke-width=\"0.4\" stroke-opacity=\"0.5\"/>\n"));
                    }
                }
            }
            _ => {}
        }
    }
    o.push_str("</g>\n");
    draw_labels(o, fig, ax, ay, aw, ah);
    draw_colorbar(o, fig, ax, ay, aw, ah);
}

/// Render the figure to PNG bytes (2× device pixels for crispness).
#[cfg(not(target_arch = "wasm32"))]
pub fn to_png(fig: &Figure, scale: f32) -> Result<Vec<u8>, String> {
    use resvg::{tiny_skia, usvg};
    let mut db = usvg::fontdb::Database::new();
    db.load_system_fonts();
    let opt = usvg::Options { fontdb: std::sync::Arc::new(db), ..Default::default() };
    let tree = usvg::Tree::from_str(&to_svg(fig), &opt).map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let w = (size.width() * scale).ceil().max(1.0) as u32;
    let h = (size.height() * scale).ceil().max(1.0) as u32;
    if w.saturating_mul(h) > 16_000_000 {
        return Err("figure too large to rasterize".to_string());
    }
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| "cannot allocate pixmap".to_string())?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(pixmap.data()).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Render the figure to a single-page PDF (the rasterised PNG-resolution
/// image embedded as a FlateDecode RGB XObject — keeps the exporter tiny and
/// dependency-free beyond `flate2`, which the workspace already carries).
#[cfg(not(target_arch = "wasm32"))]
pub fn to_pdf(fig: &Figure) -> Result<Vec<u8>, String> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use resvg::{tiny_skia, usvg};
    use std::io::Write;
    let mut db = usvg::fontdb::Database::new();
    db.load_system_fonts();
    let opt = usvg::Options { fontdb: std::sync::Arc::new(db), ..Default::default() };
    let tree = usvg::Tree::from_str(&to_svg(fig), &opt).map_err(|e| e.to_string())?;
    let scale = 2.0_f32;
    let size = tree.size();
    let (w, h) = ((size.width() * scale).ceil().max(1.0) as u32, (size.height() * scale).ceil().max(1.0) as u32);
    if w.saturating_mul(h) > 16_000_000 {
        return Err("figure too large to rasterize".to_string());
    }
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| "cannot allocate pixmap".to_string())?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // Premultiplied RGBA → opaque RGB.
    let mut rgb = Vec::with_capacity(pixmap.data().len() / 4 * 3);
    for px_ in pixmap.data().as_chunks::<4>().0 {
        rgb.extend_from_slice(&px_[..3]);
    }
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&rgb).map_err(|e| e.to_string())?;
    let img = enc.finish().map_err(|e| e.to_string())?;
    // Page size in points = CSS pixels of the SVG.
    let (pw, ph) = (size.width() as f64, size.height() as f64);
    let content = format!("q {pw:.0} 0 0 {ph:.0} 0 0 cm /Im0 Do Q\n");
    let mut objs: Vec<String> = Vec::new();
    objs.push("<< /Type /Catalog /Pages 2 0 R >>".to_string());
    objs.push("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string());
    objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.0} {ph:.0}] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>"));
    objs.push(format!(
        "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>",
        img.len()
    ));
    objs.push(format!("<< /Length {} >>", content.len()));
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, head) in objs.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{head}\n", i + 1));
        match i + 1 {
            4 => pdf.push_str("stream\n"),
            5 => pdf.push_str(&format!("stream\n{content}endstream\n")),
            _ => {}
        }
        if i + 1 != 4 {
            pdf.push_str("endobj\n");
        }
    }
    // Object 4's stream is binary — append raw.
    let mut bytes = pdf.into_bytes();
    bytes.extend_from_slice(&img);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    let xref = bytes.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1);
    for off in &offsets {
        tail.push_str(&format!("{off:010} 00000 n \n"));
    }
    tail.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%%%EOF\n", objs.len() + 1));
    bytes.extend_from_slice(tail.as_bytes());
    Ok(bytes)
}

fn draw_marker(o: &mut String, mk: Marker, x: f64, y: f64, col: &str) {
    let r = match mk {
        Marker::Point => 1.6,
        _ => 3.6,
    };
    match mk {
        Marker::Circle | Marker::Point => {
            o.push_str(&format!("<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"{r:.2}\" fill=\"{col}\"/>\n"));
        }
        Marker::Square => {
            o.push_str(&format!("<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"{col}\"/>\n", x - r, y - r, r * 2.0, r * 2.0));
        }
        Marker::Plus | Marker::Cross => {
            let (d1, d2) = if mk == Marker::Plus {
                (format!("M{x:.2},{:.2}L{x:.2},{:.2}M{:.2},{y:.2}L{:.2},{y:.2}", y - r, y + r, x - r, x + r), String::new())
            } else {
                (format!("M{:.2},{:.2}L{:.2},{:.2}", x - r, y - r, x + r, y + r), format!("M{:.2},{:.2}L{:.2},{:.2}", x - r, y + r, x + r, y - r))
            };
            o.push_str(&format!("<path d=\"{d1}{d2}\" stroke=\"{col}\" stroke-width=\"1.6\" fill=\"none\"/>\n"));
        }
        Marker::Star => {
            let mut d = String::new();
            for k in 0..3 {
                let a = k as f64 * std::f64::consts::PI / 3.0 - std::f64::consts::FRAC_PI_2;
                let (dx, dy) = (a.cos() * r * 1.4, a.sin() * r * 1.4);
                d.push_str(&format!("M{:.2},{:.2}L{:.2},{:.2}", x - dx, y - dy, x + dx, y + dy));
            }
            o.push_str(&format!("<path d=\"{d}\" stroke=\"{col}\" stroke-width=\"1.4\" fill=\"none\"/>\n"));
        }
        Marker::Diamond | Marker::TriUp | Marker::TriDown | Marker::TriLeft | Marker::TriRight => {
            let pts: Vec<(f64, f64)> = match mk {
                Marker::Diamond => vec![(0.0, -r), (r, 0.0), (0.0, r), (-r, 0.0)],
                Marker::TriUp => vec![(0.0, -r), (r, r), (-r, r)],
                Marker::TriDown => vec![(0.0, r), (r, -r), (-r, -r)],
                Marker::TriLeft => vec![(-r, 0.0), (r, -r), (r, r)],
                _ => vec![(r, 0.0), (-r, -r), (-r, r)],
            };
            let d: String = pts
                .iter()
                .enumerate()
                .map(|(i, (dx, dy))| format!("{}{:.2},{:.2}", if i == 0 { "M" } else { "L" }, x + dx, y + dy))
                .collect::<Vec<_>>()
                .join("");
            o.push_str(&format!("<path d=\"{d}Z\" fill=\"{col}\"/>\n"));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn linespec_parses() {
        let s = LineSpec::parse("r--o");
        assert_eq!(s.dash, DashStyle::Dashed);
        assert_eq!(s.marker, Some(Marker::Circle));
        assert_eq!(s.color, Some(Rgb(0xA2, 0x14, 0x2F)));
        let s2 = LineSpec::parse(":");
        assert_eq!(s2.dash, DashStyle::Dotted);
        let s3 = LineSpec::parse("x");
        assert_eq!(s3.dash, DashStyle::None);
        assert_eq!(s3.marker, Some(Marker::Cross));
    }

    #[test]
    fn svg_renders() {
        let mut f = Figure::new();
        f.push(SeriesKind::Line, vec![0.0, 1.0, 2.0], vec![0.0, 1.0, 0.0], LineSpec::new());
        f.title = Some("demo".to_string());
        let svg = to_svg(&f);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("demo"));
        assert!(svg.contains("<path"));
    }

    #[test]
    fn text_series_renders() {
        let mut f = Figure::new();
        f.push(SeriesKind::Line, vec![0.0, 1.0], vec![0.0, 1.0], LineSpec::new());
        let mut spec = LineSpec::new();
        spec.size = Some(14.0);
        f.push_text(0.5, 0.5, "hello <there>".to_string(), spec);
        let svg = to_svg(&f);
        assert!(svg.contains("font-size=\"14.0\""));
        assert!(svg.contains("hello &lt;there&gt;"));
    }

    #[test]
    fn rgba_series_renders() {
        let mut f = Figure::new();
        f.yrev = true;
        // 2x2: red, green / blue, white — row-major RGBA8.
        let px = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
        f.push_rgba(vec![1.0, 2.0], vec![1.0, 2.0], 2, 2, px);
        let svg = to_svg(&f);
        assert!(svg.contains("<image"));
        assert!(svg.contains("data:image/png;base64,"));
        // The raster expands the view by a half cell on each side.
        let (x0, x1, _, _) = view_limits(&f);
        assert!((x0 - 0.5).abs() < 0.1 && (x1 - 2.5).abs() < 0.1);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn rgba_rasterizes() {
        // The embedded PNG data URI must survive the resvg path.
        let mut f = Figure::new();
        f.push_rgba(vec![1.0], vec![1.0], 1, 1, vec![255, 0, 0, 255]);
        let png = to_png(&f, 1.0).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let pdf = to_pdf(&f).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
    }

    #[test]
    fn nice_ticks() {
        let t = ticks(0.0, 1.0, 6);
        // step 0.2 → {0, 0.2, ..., 1.0} (float-accumulated: compare loosely)
        assert!(t.iter().any(|&v| (v - 0.4).abs() < 1e-9));
        assert!(t.iter().any(|&v| (v - 1.0).abs() < 1e-9));
        assert!(!t.contains(&0.5));
        assert!((nice_step(0.17) - 0.2).abs() < 1e-12);
    }
}
