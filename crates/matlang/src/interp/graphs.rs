//! MATLAB-style `graph`/`digraph` objects and small graph algorithms.
//!
//! A graph stores a dense adjacency matrix plus node names and a directed
//! flag. The algorithms are deliberately simple (BFS/DFS and Dijkstra), which
//! keeps them deterministic and safe for the sizes used by scripts.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

use super::*;

fn vector_arg(v: Option<&Value>, what: &str) -> Result<Vec<f64>, LangError> {
    let m = v.ok_or_else(|| LangError::new(format!("{what} is required"), 0))?.to_numeric()?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err(LangError::new(format!("{what} must be a vector"), 0));
    }
    Ok(m.data().to_vec())
}

fn node_names(v: Option<&Value>, n: usize) -> Result<Vec<String>, LangError> {
    let mut out = match v {
        Some(Value::Cell(c)) => {
            let mut names = Vec::with_capacity(c.numel());
            for i in 0..c.numel() {
                match c.get_linear(i) {
                    Some(Value::Str(s)) => names.push(s.clone()),
                    _ => return Err(LangError::new("NodeNames must be strings", 0)),
                }
            }
            names
        }
        Some(Value::Str(s)) => vec![s.clone()],
        _ => Vec::new(),
    };
    if out.is_empty() {
        out = (1..=n).map(|i| i.to_string()).collect();
    }
    if out.len() != n {
        return Err(LangError::new("NodeNames must match the number of nodes", 0));
    }
    Ok(out)
}

fn make_graph(a: Matrix, names: Vec<String>, directed: bool) -> Result<Value, LangError> {
    let mut s = StructArray::new();
    s.set_field(0, "Adjacency", Value::Mat(a))?;
    s.set_field(0, "NodeNames", Value::Cell(CellArray::row(names.into_iter().map(Value::Str).collect())))?;
    s.set_field(0, "Directed", Value::Num(directed as u8 as f64))?;
    Ok(Value::Object(MObject::owned(if directed { "digraph".into() } else { "graph".into() }, s)))
}

fn graph_parts(v: &Value) -> Result<(Matrix, Vec<String>, bool), LangError> {
    let Value::Object(o) = v else {
        return Err(LangError::new("expected a graph or digraph object", 0));
    };
    if o.class_name != "graph" && o.class_name != "digraph" {
        return Err(LangError::new("expected a graph or digraph object", 0));
    }
    let a = o.prop("Adjacency").ok_or_else(|| LangError::new("graph has no Adjacency", 0))?.to_matrix()?;
    if a.rows() != a.cols() {
        return Err(LangError::new("graph adjacency must be square", 0));
    }
    let names = match o.prop("NodeNames") {
        Some(Value::Cell(c)) => {
            let mut v = Vec::with_capacity(c.numel());
            for i in 0..c.numel() {
                v.push(match c.get_linear(i) {
                    Some(Value::Str(s)) => s.clone(),
                    _ => (i + 1).to_string(),
                });
            }
            v
        }
        _ => (1..=a.rows()).map(|i| i.to_string()).collect(),
    };
    let directed = o.prop("Directed").and_then(|x| x.scalar().ok()).unwrap_or(0.0) != 0.0;
    Ok((a, names, directed))
}

fn adjacency_from_edges(s: &[f64], t: &[f64], w: Option<&[f64]>) -> Result<(Matrix, usize), LangError> {
    if s.len() != t.len() {
        return Err(LangError::new("graph: s and t must have the same length", 0));
    }
    let mut n = 0usize;
    for v in s.iter().chain(t) {
        if !v.is_finite() || *v < 1.0 || v.fract() != 0.0 {
            return Err(LangError::new("graph: edge endpoints must be positive integer indices", 0));
        }
        n = n.max(*v as usize);
    }
    let mut a = Matrix::zeros(n, n);
    for i in 0..s.len() {
        let si = s.get(i).copied().unwrap_or(1.0) as usize - 1;
        let ti = t.get(i).copied().unwrap_or(1.0) as usize - 1;
        let weight = w.and_then(|w| w.get(i)).copied().unwrap_or(1.0);
        let _ = a.set(si, ti, weight);
    }
    Ok((a, n))
}

fn weak_components(a: &Matrix) -> Vec<usize> {
    let n = a.rows();
    let mut labels = vec![0usize; n];
    let mut next = 1usize;
    let mut queue = VecDeque::new();
    for start in 0..n {
        if labels.get(start).copied().unwrap_or(0) != 0 {
            continue;
        }
        labels[start] = next;
        queue.push_back(start);
        while let Some(u) = queue.pop_front() {
            for v in 0..n {
                let edge = a.get(u, v).unwrap_or(0.0) != 0.0 || a.get(v, u).unwrap_or(0.0) != 0.0;
                if edge && labels.get(v).copied().unwrap_or(0) == 0 {
                    labels[v] = next;
                    queue.push_back(v);
                }
            }
        }
        next += 1;
    }
    labels
}

fn degree(a: &Matrix, directed: bool) -> Vec<f64> {
    let n = a.rows();
    (0..n)
        .map(|i| {
            let mut d = 0usize;
            for j in 0..n {
                let out = a.get(i, j).unwrap_or(0.0) != 0.0;
                let inc = a.get(j, i).unwrap_or(0.0) != 0.0;
                if i == j {
                    // A self-loop contributes two to the degree.
                    if out || inc {
                        d += 2;
                    }
                } else if directed {
                    d += usize::from(out) + usize::from(inc);
                } else if out || inc {
                    d += 1;
                }
            }
            d as f64
        })
        .collect()
}

#[derive(Clone, Copy)]
struct HeapItem {
    dist: f64,
    node: usize,
}
impl PartialEq for HeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.dist == other.dist && self.node == other.node
    }
}
impl Eq for HeapItem {}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other.dist.total_cmp(&self.dist).then_with(|| other.node.cmp(&self.node))
    }
}

fn dijkstra(a: &Matrix, start: usize, directed: bool) -> (Vec<f64>, Vec<usize>) {
    let n = a.rows();
    let mut dist = vec![f64::INFINITY; n];
    let mut prev = vec![usize::MAX; n];
    if start >= n {
        return (dist, prev);
    }
    dist[start] = 0.0;
    let mut heap = BinaryHeap::new();
    heap.push(HeapItem { dist: 0.0, node: start });
    while let Some(HeapItem { dist: d, node: u }) = heap.pop() {
        if d > dist.get(u).copied().unwrap_or(f64::INFINITY) {
            continue;
        }
        for v in 0..n {
            let w = a.get(u, v).unwrap_or(0.0);
            let linked = w != 0.0 || (!directed && a.get(v, u).unwrap_or(0.0) != 0.0);
            if !linked {
                continue;
            }
            let weight = if w != 0.0 { w.abs() } else { a.get(v, u).unwrap_or(1.0).abs() };
            let nd = d + weight;
            if nd < dist.get(v).copied().unwrap_or(f64::INFINITY) {
                dist[v] = nd;
                prev[v] = u;
                heap.push(HeapItem { dist: nd, node: v });
            }
        }
    }
    (dist, prev)
}

impl Session {
    pub(super) fn plot_graph_object(&mut self, o: &MObject) -> Result<Option<Value>, LangError> {
        let (a, names, directed) = graph_parts(&Value::Object(o.clone()))?;
        let n = a.rows();
        let mut x = Vec::with_capacity(n);
        let mut y = Vec::with_capacity(n);
        for i in 0..n {
            let angle = std::f64::consts::TAU * i as f64 / n.max(1) as f64;
            x.push(angle.cos());
            y.push(angle.sin());
        }
        let palette = [
            matplot::Rgb(0x00, 0x72, 0xBD),
            matplot::Rgb(0xD9, 0x53, 0x19),
            matplot::Rgb(0xED, 0xB1, 0x20),
            matplot::Rgb(0x7E, 0x2F, 0x8E),
            matplot::Rgb(0x77, 0xAC, 0x30),
            matplot::Rgb(0x4D, 0xBE, 0xEE),
            matplot::Rgb(0xA2, 0x14, 0x2F),
        ];
        let colors = (0..n).map(|i| palette[i % palette.len()]).collect();
        if !self.figure_mut().hold {
            self.figure_mut().series.clear();
        }
        self.figure_mut().push_graph(x, y, a.data().to_vec(), names, directed, colors);
        Ok(Some(Value::Num(self.cur_fig as f64)))
    }

    pub(super) fn graph_builtin(&mut self, name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "graph" | "digraph" => {
                let directed = name == "digraph";
                let selfloops = argv.iter().any(|v| matches!(v, Value::Str(s) if s.eq_ignore_ascii_case("omitselfloops")));
                let first_matrix = argv.first().and_then(|v| v.to_numeric().ok());
                let names_after_adjacency = matches!(argv.get(1), Some(Value::Str(_)) | Some(Value::Cell(_)))
                    && !matches!(argv.get(1), Some(Value::Str(s)) if s.eq_ignore_ascii_case("omitselfloops"));
                let option_after_adjacency = matches!(argv.get(1), Some(Value::Str(s)) if s.eq_ignore_ascii_case("omitselfloops"));
                let adjacency_mode = first_matrix.as_ref().is_some_and(|m| m.rows() == m.cols())
                    && (argv.len() == 1 || names_after_adjacency || option_after_adjacency);
                let (mut a, names_at) = if adjacency_mode {
                    let m = first_matrix.ok_or_else(|| LangError::new(format!("{name}: adjacency matrix must be square"), 0))?;
                    (m, if names_after_adjacency { 1 } else { usize::MAX })
                } else {
                    let s = vector_arg(argv.first(), "s")?;
                    let t = vector_arg(argv.get(1), "t")?;
                    let weights = argv.get(2).and_then(|v| v.to_numeric().ok()).map(|m| m.data().to_vec());
                    let (mut a, _) = adjacency_from_edges(&s, &t, weights.as_deref())?;
                    if !directed {
                        let n = a.rows();
                        for i in 0..n {
                            for j in 0..n {
                                if a.get(i, j).unwrap_or(0.0) != 0.0 {
                                    let _ = a.set(j, i, a.get(i, j).unwrap_or(0.0));
                                }
                            }
                        }
                    }
                    let is_option = |v: Option<&Value>| matches!(v, Some(Value::Str(s)) if s.eq_ignore_ascii_case("omitselfloops"));
                    let name_index = if weights.is_some() {
                        if matches!(argv.get(3), Some(Value::Str(_)) | Some(Value::Cell(_))) && !is_option(argv.get(3)) {
                            3
                        } else {
                            usize::MAX
                        }
                    } else if matches!(argv.get(2), Some(Value::Str(_)) | Some(Value::Cell(_))) && !is_option(argv.get(2)) {
                        2
                    } else {
                        usize::MAX
                    };
                    (a, name_index)
                };
                if selfloops {
                    for i in 0..a.rows() {
                        let _ = a.set(i, i, 0.0);
                    }
                }
                if !directed {
                    let n = a.rows();
                    for i in 0..n {
                        for j in 0..n {
                            if a.get(i, j).unwrap_or(0.0) != 0.0 {
                                let _ = a.set(j, i, a.get(i, j).unwrap_or(0.0));
                            }
                        }
                    }
                }
                let names = node_names(argv.get(names_at), a.rows())?;
                Ok(vec![make_graph(a, names, directed)?])
            }
            "numnodes" | "numedges" | "degree" | "conncomp" | "neighbors" | "shortestpath" | "distances" | "bfsearch" | "dfsearch" | "adjacency" => {
                let (a, names, directed) = graph_parts(argv.first().ok_or_else(|| LangError::new(format!("{name} needs a graph"), 0))?)?;
                let n = a.rows();
                match name {
                    "numnodes" => Ok(vec![Value::Num(n as f64)]),
                    "numedges" => {
                        let mut count = 0usize;
                        for i in 0..n {
                            for j in 0..n {
                                if a.get(i, j).unwrap_or(0.0) != 0.0 && (directed || j >= i) {
                                    count += 1;
                                }
                            }
                        }
                        Ok(vec![Value::Num(count as f64)])
                    }
                    "degree" => Ok(vec![Value::Mat(Matrix::col_vector(degree(&a, directed)))]),
                    "adjacency" => Ok(vec![Value::Mat(a)]),
                    "conncomp" => {
                        let labels = weak_components(&a);
                        let k = labels.iter().copied().max().unwrap_or(0);
                        Ok(vec![Value::Mat(Matrix::col_vector(labels.into_iter().map(|v| v as f64).collect())), Value::Num(k as f64)])
                    }
                    "neighbors" => {
                        let node = argv.get(1).ok_or_else(|| LangError::new("neighbors needs a node", 0))?.scalar()?.round();
                        if node < 1.0 || node > n as f64 {
                            return Err(LangError::new("neighbors: node index out of range", 0));
                        }
                        let u = node as usize - 1;
                        let mut out = Vec::new();
                        for v in 0..n {
                            if a.get(u, v).unwrap_or(0.0) != 0.0 || (!directed && a.get(v, u).unwrap_or(0.0) != 0.0) {
                                out.push(Value::Str(names.get(v).cloned().unwrap_or_else(|| (v + 1).to_string())));
                            }
                        }
                        Ok(vec![Value::Cell(CellArray::row(out))])
                    }
                    "shortestpath" => {
                        let s = argv.get(1).ok_or_else(|| LangError::new("shortestpath needs source", 0))?.scalar()?.round();
                        let t = argv.get(2).ok_or_else(|| LangError::new("shortestpath needs target", 0))?.scalar()?.round();
                        if s < 1.0 || s > n as f64 || t < 1.0 || t > n as f64 {
                            return Err(LangError::new("shortestpath: node index out of range", 0));
                        }
                        let (dist, prev) = dijkstra(&a, s as usize - 1, directed);
                        let target = t as usize - 1;
                        let d = dist.get(target).copied().unwrap_or(f64::INFINITY);
                        if !d.is_finite() {
                            return Ok(vec![Value::Cell(CellArray::row(Vec::new())), Value::Num(f64::INFINITY)]);
                        }
                        let mut path = Vec::new();
                        let mut cur = target;
                        while cur != usize::MAX {
                            path.push(cur);
                            if cur == s as usize - 1 {
                                break;
                            }
                            cur = prev.get(cur).copied().unwrap_or(usize::MAX);
                        }
                        path.reverse();
                        let path: Vec<Value> =
                            path.into_iter().map(|i| Value::Str(names.get(i).cloned().unwrap_or_else(|| (i + 1).to_string()))).collect();
                        Ok(vec![Value::Cell(CellArray::row(path)), Value::Num(d)])
                    }
                    "distances" => {
                        let s = argv.get(1).and_then(|v| v.scalar().ok()).map(|v| v.round() as usize).unwrap_or(0);
                        if s == 0 {
                            let mut out = Matrix::zeros(n, n);
                            for i in 0..n {
                                let (d, _) = dijkstra(&a, i, directed);
                                for j in 0..n {
                                    let _ = out.set(i, j, d.get(j).copied().unwrap_or(f64::INFINITY));
                                }
                            }
                            Ok(vec![Value::Mat(out)])
                        } else {
                            if s > n {
                                return Err(LangError::new("distances: source out of range", 0));
                            }
                            let (d, _) = dijkstra(&a, s - 1, directed);
                            Ok(vec![Value::Mat(Matrix::row_vector(d))])
                        }
                    }
                    "bfsearch" | "dfsearch" => {
                        let s = argv.get(1).and_then(|v| v.scalar().ok()).map(|v| v.round() as usize).unwrap_or(1);
                        if s < 1 || s > n {
                            return Err(LangError::new("search: start node out of range", 0));
                        }
                        let mut seen = vec![false; n];
                        let mut order = Vec::new();
                        if name == "bfsearch" {
                            let mut q = VecDeque::new();
                            q.push_back(s - 1);
                            seen[s - 1] = true;
                            while let Some(u) = q.pop_front() {
                                order.push((u + 1) as f64);
                                for (v, seen_v) in seen.iter_mut().enumerate() {
                                    if !*seen_v && (a.get(u, v).unwrap_or(0.0) != 0.0 || (!directed && a.get(v, u).unwrap_or(0.0) != 0.0)) {
                                        *seen_v = true;
                                        q.push_back(v);
                                    }
                                }
                            }
                        } else {
                            fn dfs(u: usize, a: &Matrix, directed: bool, seen: &mut [bool], order: &mut Vec<f64>) {
                                seen[u] = true;
                                order.push((u + 1) as f64);
                                for v in 0..a.rows() {
                                    if !seen[v] && (a.get(u, v).unwrap_or(0.0) != 0.0 || (!directed && a.get(v, u).unwrap_or(0.0) != 0.0)) {
                                        dfs(v, a, directed, seen, order);
                                    }
                                }
                            }
                            dfs(s - 1, &a, directed, &mut seen, &mut order);
                        }
                        let _ = nout;
                        Ok(vec![Value::Mat(Matrix::row_vector(order))])
                    }
                    _ => Err(LangError::new(format!("{name}: unsupported graph operation"), 0)),
                }
            }
            "gplot" => {
                let a = argv.first().ok_or_else(|| LangError::new("gplot needs an adjacency matrix", 0))?.to_numeric()?;
                let xy = argv.get(1).ok_or_else(|| LangError::new("gplot needs node coordinates", 0))?.to_numeric()?;
                if xy.rows() < a.rows() || xy.cols() < 2 {
                    return Err(LangError::new("gplot: XY must be an n-by-2 matrix", 0));
                }
                let x: Vec<f64> = (0..a.rows()).map(|i| xy.get(i, 0).unwrap_or(0.0)).collect();
                let y: Vec<f64> = (0..a.rows()).map(|i| xy.get(i, 1).unwrap_or(0.0)).collect();
                let names: Vec<String> = (1..=a.rows()).map(|i| i.to_string()).collect();
                if !self.figure_mut().hold {
                    self.figure_mut().series.clear();
                }
                self.figure_mut().push_graph(x, y, a.data().to_vec(), names, false, Vec::new());
                Ok(vec![Value::Num(self.cur_fig as f64)])
            }
            "isgraph" => {
                let ok = matches!(argv.first(), Some(Value::Object(o)) if o.class_name == "graph" || o.class_name == "digraph");
                Ok(vec![Value::Num(ok as u8 as f64)])
            }
            _ => Err(LangError::new(format!("{name}: not a graph builtin"), 0)),
        }
    }
}
