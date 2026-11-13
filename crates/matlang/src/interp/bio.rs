//! Chemistry, reaction, GPR and sequence-file builtins.
//!
//! These are original implementations of generic, documented formats:
//! elemental formulas, reaction equations, gene-protein-reaction Boolean
//! rules, FASTA and GenBank flat files. They deliberately avoid any
//! metabolic-model object API.

use std::collections::{BTreeMap, BTreeSet};

use super::*;

fn as_string(v: &Value, what: &str) -> Result<String, LangError> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        _ => Err(LangError::new(format!("{what} must be a string"), 0)),
    }
}

pub(super) fn as_string_list(v: &Value, what: &str) -> Result<Vec<String>, LangError> {
    match v {
        Value::Str(s) => Ok(vec![s.clone()]),
        Value::Cell(c) => {
            let mut out = Vec::with_capacity(c.numel());
            for i in 0..c.numel() {
                match c.get_linear(i) {
                    Some(Value::Str(s)) => out.push(s.clone()),
                    _ => return Err(LangError::new(format!("{what} must contain strings"), 0)),
                }
            }
            Ok(out)
        }
        _ => Err(LangError::new(format!("{what} must be a string or cellstr"), 0)),
    }
}

fn numeric_vector(v: &Value, what: &str) -> Result<Vec<f64>, LangError> {
    let m = v.to_numeric()?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err(LangError::new(format!("{what} must be a vector"), 0));
    }
    Ok(m.data().to_vec())
}

#[derive(Clone, Debug, Default)]
struct Reaction {
    reactants: Vec<(f64, String)>,
    products: Vec<(f64, String)>,
    reversible: bool,
}

impl Reaction {
    fn species(&self) -> Vec<String> {
        self.reactants.iter().chain(self.products.iter()).map(|(_, s)| s.clone()).collect()
    }

    fn signed(&self) -> Vec<(f64, String)> {
        let mut out: Vec<(f64, String)> = self.reactants.iter().map(|(c, s)| (-*c, s.clone())).collect();
        out.extend(self.products.iter().map(|(c, s)| (*c, s.clone())));
        out
    }

    fn canonical(&self) -> Reaction {
        let mut r = self.reactants.clone();
        let mut p = self.products.clone();
        r.sort_by(|a, b| a.1.cmp(&b.1));
        p.sort_by(|a, b| a.1.cmp(&b.1));
        // Normalize orientation: the alphabetically first participant is a
        // reactant. This makes A -> B and B -> A compare equal when reverse
        // matching is allowed.
        let first_r = r.first().map(|(_, s)| s.clone());
        let first_p = p.first().map(|(_, s)| s.clone());
        let flip = match (first_r, first_p) {
            (Some(a), Some(b)) => b < a,
            (None, Some(_)) => true,
            _ => false,
        };
        if flip {
            Reaction { reactants: p, products: r, reversible: self.reversible }
        } else {
            Reaction { reactants: r, products: p, reversible: self.reversible }
        }
    }

    fn text(&self) -> String {
        let side = |v: &[(f64, String)]| -> String {
            if v.is_empty() {
                return String::new();
            }
            v.iter()
                .map(|(c, s)| if (*c - 1.0).abs() < 1e-12 { s.clone() } else { format!("{} {}", fmt_num(*c, false), s) })
                .collect::<Vec<_>>()
                .join(" + ")
        };
        let arrow = if self.reversible { " <=> " } else { " -> " };
        format!("{}{}{}", side(&self.reactants), arrow, side(&self.products))
    }
}

fn split_side(side: &str) -> Result<Vec<(f64, String)>, LangError> {
    let chars: Vec<char> = side.chars().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0usize;
    for (i, c) in chars.iter().enumerate() {
        if *c != '+' {
            continue;
        }
        let prev = if i > 0 { chars.get(i - 1).copied().unwrap_or(' ') } else { ' ' };
        let next = chars.get(i + 1).copied().unwrap_or(' ');
        // A trailing `+` is an ionic charge (`Na+`, `H+`), not a separator.
        if !prev.is_whitespace() && next.is_whitespace() {
            continue;
        }
        parts.push(chars.get(start..i).unwrap_or(&[]).iter().collect());
        start = i + 1;
    }
    parts.push(chars.get(start..).unwrap_or(&[]).iter().collect());
    let mut out = Vec::new();
    for raw in parts {
        let term = raw.trim();
        if term.is_empty() {
            continue;
        }
        let bytes = term.as_bytes();
        let mut end = 0usize;
        let mut seen_dot = false;
        while end < bytes.len() {
            let c = bytes.get(end).copied().unwrap_or(b' ');
            if c.is_ascii_digit() {
                end += 1;
            } else if c == b'.' && !seen_dot {
                seen_dot = true;
                end += 1;
            } else {
                break;
            }
        }
        let (coeff, name) = if end > 0 {
            let number = term.get(..end).ok_or_else(|| LangError::new("bad reaction coefficient", 0))?;
            let value = number.parse::<f64>().map_err(|_| LangError::new(format!("bad reaction coefficient '{number}'"), 0))?;
            let rest = term.get(end..).unwrap_or("").trim_start_matches('*').trim();
            (value, rest.to_string())
        } else {
            (1.0, term.to_string())
        };
        if !coeff.is_finite() || coeff < 0.0 || name.is_empty() {
            return Err(LangError::new(format!("bad reaction term '{term}'"), 0));
        }
        out.push((coeff, name));
    }
    if out.is_empty() {
        return Err(LangError::new("reaction side is empty", 0));
    }
    Ok(out)
}

fn parse_reaction(text: &str) -> Result<Reaction, LangError> {
    let text = text.trim();
    let (left, right, reversible) = if let Some((a, b)) = text.split_once("<=>") {
        (a, b, true)
    } else if let Some((a, b)) = text.split_once("<->") {
        (a, b, true)
    } else if let Some((a, b)) = text.split_once("->") {
        (a, b, false)
    } else if let Some((a, b)) = text.split_once("=>") {
        (a, b, false)
    } else {
        return Err(LangError::new("reaction needs an arrow (->, <=> or <->)", 0));
    };
    Ok(Reaction { reactants: split_side(left)?, products: split_side(right)?, reversible })
}

fn proton_name(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.starts_with("h_")
        || lower.starts_with("h[")
        || lower.starts_with("cpd00067")
        || lower.starts_with("proton")
        || lower.starts_with("c00080")
        || lower.starts_with("mnxm1")
        || lower == "h+"
        || lower == "h"
}

fn reactions_equal(a: &Reaction, b: &Reaction, allow_reverse: bool, proton_mismatch: bool) -> bool {
    let norm = |r: &Reaction, reverse: bool| -> BTreeMap<String, f64> {
        let mut m = BTreeMap::new();
        for (c, s) in r.signed() {
            if proton_mismatch && proton_name(&s) {
                continue;
            }
            let c = if reverse { -c } else { c };
            *m.entry(s).or_insert(0.0) += c;
        }
        m.retain(|_, v| v.abs() > 1e-12);
        m
    };
    let base = norm(a, false);
    if base == norm(b, false) {
        return true;
    }
    allow_reverse && base == norm(b, true)
}

fn reaction_value(r: &Reaction) -> Result<Value, LangError> {
    let species = r.species();
    let coeffs = r.signed();
    let mut s = StructArray::new();
    s.set_field(0, "Reactants", Value::Cell(CellArray::row(r.reactants.iter().map(|(_, x)| Value::Str(x.clone())).collect())))?;
    s.set_field(0, "Products", Value::Cell(CellArray::row(r.products.iter().map(|(_, x)| Value::Str(x.clone())).collect())))?;
    s.set_field(0, "Species", Value::Cell(CellArray::row(species.into_iter().map(Value::Str).collect())))?;
    s.set_field(0, "Coefficients", Value::Mat(Matrix::row_vector(coeffs.into_iter().map(|(c, _)| c).collect())))?;
    s.set_field(0, "Reversible", Value::Num(r.reversible as u8 as f64))?;
    s.set_field(0, "Text", Value::Str(r.text()))?;
    Ok(Value::Struct(s))
}

fn reaction_from_value(v: &Value) -> Result<Reaction, LangError> {
    match v {
        Value::Str(s) => parse_reaction(s),
        Value::Struct(st) => {
            let reversible = st.field_at(0, "Reversible").and_then(|x| x.scalar().ok()).map(|x| x != 0.0).unwrap_or(false);
            if let (Some(sp), Some(co)) = (
                st.field_at(0, "Species").and_then(|x| as_string_list(x, "Species").ok()),
                st.field_at(0, "Coefficients").and_then(|x| x.to_numeric().ok()),
            ) {
                if sp.len() == co.numel() {
                    let mut reactants = Vec::new();
                    let mut products = Vec::new();
                    for (i, name) in sp.into_iter().enumerate() {
                        let c = co.get_linear(i).unwrap_or(0.0);
                        if c < 0.0 {
                            reactants.push((-c, name));
                        } else if c > 0.0 {
                            products.push((c, name));
                        }
                    }
                    return Ok(Reaction { reactants, products, reversible });
                }
            }
            let reactants = st.field_at(0, "Reactants").and_then(|x| as_string_list(x, "Reactants").ok()).unwrap_or_default();
            let products = st.field_at(0, "Products").and_then(|x| as_string_list(x, "Products").ok()).unwrap_or_default();
            if reactants.is_empty() && products.is_empty() {
                return parse_reaction(&st.field_at(0, "Text").and_then(|x| as_string(x, "Text").ok()).unwrap_or_default());
            }
            Ok(Reaction {
                reactants: reactants.into_iter().map(|s| (1.0, s)).collect(),
                products: products.into_iter().map(|s| (1.0, s)).collect(),
                reversible,
            })
        }
        _ => Err(LangError::new("reaction must be a string or parsereaction struct", 0)),
    }
}

fn parse_formula(text: &str) -> Result<Vec<(String, f64)>, String> {
    let mut out: Vec<(String, f64)> = Vec::new();
    for group in text.split('.') {
        let group = group.trim().trim_end_matches(['+', '-']);
        if group.is_empty() {
            continue;
        }
        let chars: Vec<char> = group.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            let c = chars.get(i).copied().ok_or("truncated formula")?;
            if !c.is_ascii_uppercase() {
                return Err(format!("formula element must start with an uppercase letter near '{group}'"));
            }
            let mut name = String::new();
            name.push(c);
            i += 1;
            while i < chars.len() {
                let c = chars.get(i).copied().ok_or("truncated formula")?;
                if c.is_ascii_lowercase() {
                    name.push(c);
                    i += 1;
                } else {
                    break;
                }
            }
            let start = i;
            let mut seen_dot = false;
            while i < chars.len() {
                let c = chars.get(i).copied().ok_or("truncated formula")?;
                if c.is_ascii_digit() {
                    i += 1;
                } else if c == '.' && !seen_dot {
                    seen_dot = true;
                    i += 1;
                } else {
                    break;
                }
            }
            let count = if i > start {
                let num: String = chars.get(start..i).ok_or("bad formula count")?.iter().collect();
                num.parse::<f64>().map_err(|_| format!("bad formula count '{num}'"))?
            } else {
                1.0
            };
            if !count.is_finite() || count < 0.0 {
                return Err("formula counts must be nonnegative".into());
            }
            if let Some(slot) = out.iter_mut().find(|(n, _)| n == &name) {
                slot.1 += count;
            } else {
                out.push((name, count));
            }
        }
        if i < chars.len() {
            return Err(format!("unexpected character '{}' in formula", chars.get(i).copied().unwrap_or(' ')));
        }
    }
    if out.is_empty() {
        return Err("formula is empty".into());
    }
    Ok(out)
}

fn formula_text(counts: &[(String, f64)]) -> String {
    let mut out = String::new();
    for (name, count) in counts {
        if count.abs() < 1e-12 {
            continue;
        }
        out.push_str(name);
        if (*count - 1.0).abs() > 1e-12 {
            if count.fract().abs() < 1e-9 {
                out.push_str(&format!("{}", *count as i64));
            } else {
                out.push_str(&fmt_num(*count, false));
            }
        }
    }
    out
}

fn atomic_weight(name: &str) -> Option<f64> {
    let w = match name {
        "H" => 1.00794,
        "He" => 4.002602,
        "Li" => 6.941,
        "B" => 10.811,
        "C" => 12.0107,
        "N" => 14.0067,
        "O" => 15.9994,
        "F" => 18.9984032,
        "Na" => 22.989769,
        "Mg" => 24.3050,
        "Al" => 26.981538,
        "Si" => 28.0855,
        "P" => 30.973762,
        "S" => 32.065,
        "Cl" => 35.453,
        "K" => 39.0983,
        "Ca" => 40.078,
        "Mn" => 54.938045,
        "Fe" => 55.845,
        "Co" => 58.933195,
        "Ni" => 58.6934,
        "Cu" => 63.546,
        "Zn" => 65.38,
        "As" => 74.92160,
        "Se" => 78.96,
        "Br" => 79.904,
        "Mo" => 95.96,
        "Ag" => 107.8682,
        "Cd" => 112.411,
        "I" => 126.90447,
        "W" => 183.84,
        "Hg" => 200.59,
        "R" => 0.0,
        "X" => 0.0,
        "FULLR" => 0.0,
        _ => return None,
    };
    Some(w)
}

fn formula_struct(counts: &[(String, f64)]) -> Result<Value, LangError> {
    let mut s = StructArray::new();
    for (name, count) in counts {
        if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        s.set_field(0, name, Value::Num(*count))?;
    }
    Ok(Value::Struct(s))
}

#[derive(Clone, Debug, PartialEq)]
enum Gpr {
    Gene(String),
    And(Box<Gpr>, Box<Gpr>),
    Or(Box<Gpr>, Box<Gpr>),
}

fn gpr_tokenize(s: &str) -> Result<Vec<String>, LangError> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars.get(i).copied().ok_or_else(|| LangError::new("truncated GPR", 0))?;
        if c.is_whitespace() {
            i += 1;
        } else if matches!(c, '(' | ')' | '&' | '|') {
            out.push(c.to_string());
            i += 1;
        } else if c.is_ascii_alphabetic()
            && s.get(i..).is_some_and(|r| r.to_ascii_lowercase().starts_with("and") || r.to_ascii_lowercase().starts_with("or"))
        {
            let remaining = s.get(i..).unwrap_or("");
            if remaining.to_ascii_lowercase().starts_with("and") {
                out.push("&".into());
                i += 3;
            } else {
                out.push("|".into());
                i += 2;
            }
        } else {
            let start = i;
            while i < chars.len() {
                let c = chars.get(i).copied().ok_or_else(|| LangError::new("truncated GPR", 0))?;
                if c.is_whitespace() || matches!(c, '(' | ')' | '&' | '|') {
                    break;
                }
                if c.is_ascii_alphabetic()
                    && s.get(i..).is_some_and(|r| r.to_ascii_lowercase().starts_with("and") || r.to_ascii_lowercase().starts_with("or"))
                {
                    let before_ok =
                        i == start || chars.get(i.wrapping_sub(1)).is_some_and(|x| x.is_whitespace() || matches!(*x, '(' | ')' | '&' | '|'));
                    if before_ok {
                        break;
                    }
                }
                i += 1;
            }
            if i > start {
                let token: String = chars.get(start..i).ok_or_else(|| LangError::new("truncated GPR", 0))?.iter().collect();
                out.push(token);
            } else {
                i += 1;
            }
        }
    }
    Ok(out)
}

struct GprParser {
    tokens: Vec<String>,
    pos: usize,
}

impl GprParser {
    fn parse_or(&mut self) -> Result<Gpr, LangError> {
        if self.pos > 128 {
            return Err(LangError::new("GPR is too deeply nested", 0));
        }
        let mut left = self.parse_and()?;
        while self.tokens.get(self.pos).map(String::as_str) == Some("|") {
            self.pos += 1;
            let right = self.parse_and()?;
            left = Gpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Gpr, LangError> {
        let mut left = self.parse_atom()?;
        while self.tokens.get(self.pos).map(String::as_str) == Some("&") {
            self.pos += 1;
            let right = self.parse_atom()?;
            left = Gpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_atom(&mut self) -> Result<Gpr, LangError> {
        match self.tokens.get(self.pos).map(String::as_str) {
            Some("(") => {
                self.pos += 1;
                let inner = self.parse_or()?;
                if self.tokens.get(self.pos).map(String::as_str) != Some(")") {
                    return Err(LangError::new("GPR: missing closing parenthesis", 0));
                }
                self.pos += 1;
                Ok(inner)
            }
            Some(")") | None => Err(LangError::new("GPR: expected a gene or '('", 0)),
            Some(op) if op == "&" || op == "|" => Err(LangError::new("GPR: expected a gene or '('", 0)),
            Some(gene) => {
                self.pos += 1;
                Ok(Gpr::Gene(gene.to_string()))
            }
        }
    }
}

fn parse_gpr(s: &str) -> Result<Gpr, LangError> {
    let tokens = gpr_tokenize(s)?;
    if tokens.is_empty() {
        return Err(LangError::new("GPR is empty", 0));
    }
    let mut p = GprParser { tokens, pos: 0 };
    let root = p.parse_or()?;
    if p.pos != p.tokens.len() {
        return Err(LangError::new(format!("GPR: unexpected token '{}'", p.tokens.get(p.pos).map(String::as_str).unwrap_or("")), 0));
    }
    Ok(root)
}

fn gpr_value(g: &Gpr) -> Result<Value, LangError> {
    let mut s = StructArray::new();
    match g {
        Gpr::Gene(name) => {
            s.set_field(0, "Type", Value::Str("gene".into()))?;
            s.set_field(0, "Gene", Value::Str(name.clone()))?;
        }
        Gpr::And(a, b) | Gpr::Or(a, b) => {
            let kind = if matches!(g, Gpr::And(_, _)) { "and" } else { "or" };
            s.set_field(0, "Type", Value::Str(kind.into()))?;
            s.set_field(0, "Left", gpr_value(a)?)?;
            s.set_field(0, "Right", gpr_value(b)?)?;
        }
    }
    Ok(Value::Struct(s))
}

fn gpr_from_value(v: &Value) -> Result<Gpr, LangError> {
    match v {
        Value::Str(s) => parse_gpr(s),
        Value::Struct(st) => {
            let kind = st.field_at(0, "Type").and_then(|x| as_string(x, "Type").ok()).unwrap_or_default();
            match kind.as_str() {
                "gene" => Ok(Gpr::Gene(st.field_at(0, "Gene").and_then(|x| as_string(x, "Gene").ok()).unwrap_or_default())),
                "and" | "or" => {
                    let a = gpr_from_value(st.field_at(0, "Left").ok_or_else(|| LangError::new("GPR node has no Left", 0))?)?;
                    let b = gpr_from_value(st.field_at(0, "Right").ok_or_else(|| LangError::new("GPR node has no Right", 0))?)?;
                    if kind == "and" {
                        Ok(Gpr::And(Box::new(a), Box::new(b)))
                    } else {
                        Ok(Gpr::Or(Box::new(a), Box::new(b)))
                    }
                }
                _ => Err(LangError::new("GPR struct has an unknown Type", 0)),
            }
        }
        _ => Err(LangError::new("GPR must be a string or gprparse struct", 0)),
    }
}

fn gpr_eval(g: &Gpr, genes: &BTreeSet<String>) -> bool {
    match g {
        Gpr::Gene(name) => genes.contains(name),
        Gpr::And(a, b) => gpr_eval(a, genes) && gpr_eval(b, genes),
        Gpr::Or(a, b) => gpr_eval(a, genes) || gpr_eval(b, genes),
    }
}

fn gpr_collect(g: &Gpr, out: &mut BTreeSet<String>) {
    match g {
        Gpr::Gene(name) => {
            out.insert(name.clone());
        }
        Gpr::And(a, b) | Gpr::Or(a, b) => {
            gpr_collect(a, out);
            gpr_collect(b, out);
        }
    }
}

fn gpr_text(g: &Gpr) -> String {
    match g {
        Gpr::Gene(name) => name.clone(),
        Gpr::And(a, b) => format!("({} & {})", gpr_text(a), gpr_text(b)),
        Gpr::Or(a, b) => format!("({} | {})", gpr_text(a), gpr_text(b)),
    }
}

fn list_value(items: Vec<String>) -> Value {
    Value::Cell(CellArray::row(items.into_iter().map(Value::Str).collect()))
}

fn read_text_file(path: &str) -> Result<String, LangError> {
    std::fs::read_to_string(path).map_err(|e| LangError::new(format!("cannot read {path}: {e}"), 0))
}

fn fasta_records(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut header = String::new();
    let mut seq = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(h) = line.strip_prefix('>') {
            if !header.is_empty() || !seq.is_empty() {
                out.push((header.clone(), seq.clone()));
            }
            header = h.trim().to_string();
            seq.clear();
        } else {
            seq.push_str(line);
        }
    }
    if !header.is_empty() || !seq.is_empty() {
        out.push((header, seq));
    }
    out
}

#[derive(Clone, Debug, Default)]
struct GbFeature {
    key: String,
    location: String,
    locus_tag: String,
    old_locus_tag: String,
    gene: String,
    protein_id: String,
    product: String,
    ec: String,
    translation: String,
    pending: Option<String>,
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').trim().to_string()
}

fn genbank_record_values(locus: &str, definition: &str, sequence: &str, features: &[GbFeature]) -> Vec<Value> {
    let coding: Vec<&GbFeature> = features.iter().filter(|f| f.key == "CDS" || f.key == "mRNA" || f.key == "tRNA" || f.key == "rRNA").collect();
    let chosen: Vec<&GbFeature> = if coding.is_empty() { features.iter().filter(|f| f.key != "source").collect() } else { coding };
    if chosen.is_empty() {
        return vec![genbank_value(locus, definition, sequence, &GbFeature::default())];
    }
    chosen.into_iter().map(|f| genbank_value(locus, definition, sequence, f)).collect()
}

fn genbank_value(locus: &str, definition: &str, sequence: &str, f: &GbFeature) -> Value {
    let mut s = StructArray::new();
    let _ = s.set_field(0, "Locus", Value::Str(locus.to_string()));
    let _ = s.set_field(0, "Definition", Value::Str(definition.to_string()));
    let _ = s.set_field(0, "Feature", Value::Str(f.key.clone()));
    let _ = s.set_field(0, "Location", Value::Str(f.location.clone()));
    let _ = s.set_field(0, "LocusTag", Value::Str(f.locus_tag.clone()));
    let _ = s.set_field(0, "OldLocusTag", Value::Str(f.old_locus_tag.clone()));
    let _ = s.set_field(0, "Gene", Value::Str(f.gene.clone()));
    let _ = s.set_field(0, "ProteinID", Value::Str(f.protein_id.clone()));
    let _ = s.set_field(0, "Product", Value::Str(f.product.clone()));
    let _ = s.set_field(0, "ECNumber", Value::Str(f.ec.clone()));
    let _ = s.set_field(0, "Translation", Value::Str(f.translation.clone()));
    let _ = s.set_field(0, "Sequence", Value::Str(sequence.to_string()));
    Value::Struct(s)
}

fn genbank_records(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut locus = String::new();
    let mut definition = String::new();
    let mut sequence = String::new();
    let mut features: Vec<GbFeature> = Vec::new();
    let mut in_features = false;
    let mut in_origin = false;
    let mut in_definition = false;
    for line in text.lines() {
        if line.starts_with("LOCUS") {
            if !locus.is_empty() {
                out.extend(genbank_record_values(&locus, &definition, &sequence, &features));
            }
            locus = line.split_whitespace().nth(1).unwrap_or("").to_string();
            definition.clear();
            sequence.clear();
            features.clear();
            in_features = false;
            in_origin = false;
            in_definition = false;
        } else if line.starts_with("DEFINITION") {
            definition = line.get(12..).unwrap_or("").trim().to_string();
            in_definition = true;
            in_features = false;
        } else if line.starts_with("FEATURES") {
            in_features = true;
            in_origin = false;
            in_definition = false;
        } else if line.starts_with("ORIGIN") {
            in_origin = true;
            in_features = false;
            in_definition = false;
        } else if line.starts_with("//") {
            if !locus.is_empty() {
                out.extend(genbank_record_values(&locus, &definition, &sequence, &features));
            }
            locus.clear();
            definition.clear();
            sequence.clear();
            features.clear();
            in_features = false;
            in_origin = false;
            in_definition = false;
        } else if in_origin {
            sequence.extend(line.chars().filter(|c| c.is_ascii_alphabetic()));
        } else if in_features {
            let trimmed = line.trim_start();
            let indent = line.len().saturating_sub(trimmed.len());
            if indent == 5 {
                let mut it = trimmed.split_whitespace();
                let key = it.next().unwrap_or("");
                let location = it.next().unwrap_or("").to_string();
                if !key.is_empty() {
                    features.push(GbFeature { key: key.to_string(), location, ..GbFeature::default() });
                }
            } else if let Some(rest) = trimmed.strip_prefix('/') {
                let (name, value) = match rest.split_once('=') {
                    Some((n, v)) => (n, v),
                    None => (rest, ""),
                };
                if let Some(f) = features.last_mut() {
                    let value = unquote(value);
                    match name {
                        "locus_tag" => f.locus_tag = value.clone(),
                        "old_locus_tag" => f.old_locus_tag = value.clone(),
                        "gene" => f.gene = value.clone(),
                        "protein_id" => f.protein_id = value.clone(),
                        "product" => f.product = value.clone(),
                        "EC_number" => f.ec = value.clone(),
                        "translation" => f.translation = value.clone(),
                        _ => {}
                    }
                    f.pending = if value.is_empty() { None } else { Some(name.to_string()) };
                }
            } else if let Some(f) = features.last_mut() {
                if let Some(name) = f.pending.clone() {
                    let piece = if name == "translation" { trimmed.to_string() } else { format!(" {trimmed}") };
                    match name.as_str() {
                        "locus_tag" => f.locus_tag.push_str(&piece),
                        "old_locus_tag" => f.old_locus_tag.push_str(&piece),
                        "gene" => f.gene.push_str(&piece),
                        "protein_id" => f.protein_id.push_str(&piece),
                        "product" => f.product.push_str(&piece),
                        "EC_number" => f.ec.push_str(&piece),
                        "translation" => f.translation.push_str(&piece),
                        _ => {}
                    }
                }
            }
        } else if in_definition && line.starts_with("            ") {
            definition.push(' ');
            definition.push_str(line.trim());
        }
    }
    if !locus.is_empty() {
        out.extend(genbank_record_values(&locus, &definition, &sequence, &features));
    }
    out
}

impl Session {
    pub(super) fn bio_builtin(&mut self, name: &str, argv: Vec<Value>, nout: usize) -> Result<Vec<Value>, LangError> {
        match name {
            "formulaatoms" => {
                let text = as_string(argv.first().ok_or_else(|| LangError::new("formulaatoms needs a formula", 0))?, "formula")?;
                let counts = parse_formula(&text).map_err(|e| LangError::new(format!("formulaatoms: {e}"), 0))?;
                Ok(vec![formula_struct(&counts)?])
            }
            "formulaweight" => {
                let text = as_string(argv.first().ok_or_else(|| LangError::new("formulaweight needs a formula", 0))?, "formula")?;
                let counts = parse_formula(&text).map_err(|e| LangError::new(format!("formulaweight: {e}"), 0))?;
                let mut total = 0.0;
                for (el, n) in &counts {
                    let w = atomic_weight(el).ok_or_else(|| LangError::new(format!("formulaweight: unknown element '{el}'"), 0))?;
                    total += w * n;
                }
                Ok(vec![Value::Num(total)])
            }
            "formulacheck" => {
                let text = as_string(argv.first().ok_or_else(|| LangError::new("formulacheck needs a formula", 0))?, "formula")?;
                Ok(vec![Value::Num(parse_formula(&text).is_ok() as u8 as f64)])
            }
            "inferformula" => {
                let coeffs = numeric_vector(argv.first().ok_or_else(|| LangError::new("inferformula needs coefficients", 0))?, "coefficients")?;
                let formulas = as_string_list(argv.get(1).ok_or_else(|| LangError::new("inferformula needs formulas", 0))?, "formulas")?;
                let target = argv.get(2).ok_or_else(|| LangError::new("inferformula needs a target index", 0))?.scalar()?.round();
                if formulas.len() != coeffs.len() || target < 1.0 || target > coeffs.len() as f64 {
                    return Err(LangError::new("inferformula: coefficients/formulas lengths or target index do not match", 0));
                }
                let ti = target as usize - 1;
                let ct = coeffs.get(ti).copied().unwrap_or(0.0);
                if ct.abs() < 1e-12 {
                    return Err(LangError::new("inferformula: target coefficient is zero", 0));
                }
                let parsed: Vec<Option<Vec<(String, f64)>>> = formulas
                    .iter()
                    .enumerate()
                    .map(|(i, f)| if i == ti || f.trim().is_empty() { Ok(None) } else { parse_formula(f).map(Some) })
                    .collect::<Result<_, _>>()
                    .map_err(|e| LangError::new(format!("inferformula: {e}"), 0))?;
                let mut elements: BTreeMap<String, f64> = BTreeMap::new();
                for (i, formula) in parsed.iter().enumerate() {
                    if i == ti {
                        continue;
                    }
                    let Some(formula) = formula else { continue };
                    let c = coeffs.get(i).copied().unwrap_or(0.0);
                    for (el, n) in formula {
                        *elements.entry(el.clone()).or_insert(0.0) -= c * n;
                    }
                }
                let mut counts: Vec<(String, f64)> = elements.into_iter().map(|(el, n)| (el, n / ct)).filter(|(_, n)| n.abs() > 1e-9).collect();
                counts.sort_by(|a, b| a.0.cmp(&b.0));
                let text = formula_text(&counts);
                let mut out = StructArray::new();
                out.set_field(0, "Formula", Value::Str(text))?;
                out.set_field(0, "Atoms", formula_struct(&counts)?)?;
                Ok(vec![Value::Struct(out)])
            }
            "parsereaction" => {
                let text = as_string(argv.first().ok_or_else(|| LangError::new("parsereaction needs an equation", 0))?, "equation")?;
                Ok(vec![reaction_value(&parse_reaction(&text)?)?])
            }
            "formatreaction" => {
                let r = reaction_from_value(argv.first().ok_or_else(|| LangError::new("formatreaction needs a reaction", 0))?)?;
                Ok(vec![Value::Str(r.text())])
            }
            "canonicalreaction" => {
                let r = reaction_from_value(argv.first().ok_or_else(|| LangError::new("canonicalreaction needs a reaction", 0))?)?.canonical();
                Ok(vec![reaction_value(&r)?])
            }
            "rxnequal" => {
                let a = reaction_from_value(argv.first().ok_or_else(|| LangError::new("rxnequal needs two reactions", 0))?)?;
                let b = reaction_from_value(argv.get(1).ok_or_else(|| LangError::new("rxnequal needs two reactions", 0))?)?;
                let mut allow_reverse = false;
                let mut proton_mismatch = false;
                let mut i = 2usize;
                while i + 1 < argv.len() {
                    if let Value::Str(key) = &argv[i] {
                        let flag = argv.get(i + 1).and_then(|v| v.scalar().ok()).unwrap_or(0.0) != 0.0;
                        match key.to_ascii_lowercase().as_str() {
                            "reverse" => allow_reverse = flag,
                            "protonmismatch" | "proton" => proton_mismatch = flag,
                            other => return Err(LangError::new(format!("rxnequal: unknown option '{other}'"), 0)),
                        }
                    }
                    i += 2;
                }
                Ok(vec![Value::Num(reactions_equal(&a, &b, allow_reverse, proton_mismatch) as u8 as f64)])
            }
            "rxnbalance" => {
                let r = reaction_from_value(argv.first().ok_or_else(|| LangError::new("rxnbalance needs a reaction", 0))?)?;
                let formulas = as_string_list(argv.get(1).ok_or_else(|| LangError::new("rxnbalance needs formulas", 0))?, "formulas")?;
                let species = r.species();
                let signed = r.signed();
                if species.len() != formulas.len() {
                    return Err(LangError::new("rxnbalance: formulas must match the reaction species", 0));
                }
                let mut residual: BTreeMap<String, f64> = BTreeMap::new();
                for (i, (coef, _)) in signed.iter().enumerate() {
                    let f = formulas.get(i).map(String::as_str).unwrap_or("");
                    if f.trim().is_empty() {
                        continue;
                    }
                    let counts = parse_formula(f).map_err(|e| LangError::new(format!("rxnbalance: {e}"), 0))?;
                    for (el, n) in counts {
                        *residual.entry(el).or_insert(0.0) += coef * n;
                    }
                }
                let balanced = residual.values().all(|v| v.abs() < 1e-9);
                let mut s = StructArray::new();
                s.set_field(0, "Balanced", Value::Num(balanced as u8 as f64))?;
                let mut r2 = StructArray::new();
                for (el, v) in &residual {
                    r2.set_field(0, el, Value::Num(*v))?;
                }
                s.set_field(0, "Residual", Value::Struct(r2))?;
                Ok(vec![Value::Struct(s)])
            }
            "gprparse" => {
                let text = as_string(argv.first().ok_or_else(|| LangError::new("gprparse needs a rule", 0))?, "rule")?;
                Ok(vec![gpr_value(&parse_gpr(&text)?)?])
            }
            "gpreval" => {
                let g = gpr_from_value(argv.first().ok_or_else(|| LangError::new("gpreval needs a rule", 0))?)?;
                let genes = as_string_list(argv.get(1).ok_or_else(|| LangError::new("gpreval needs active genes", 0))?, "genes")?;
                let set: BTreeSet<String> = genes.into_iter().collect();
                Ok(vec![Value::Num(gpr_eval(&g, &set) as u8 as f64)])
            }
            "gprgenes" => {
                let g = gpr_from_value(argv.first().ok_or_else(|| LangError::new("gprgenes needs a rule", 0))?)?;
                let mut set = BTreeSet::new();
                gpr_collect(&g, &mut set);
                Ok(vec![list_value(set.into_iter().collect())])
            }
            "gprmerge" => {
                let a = gpr_from_value(argv.first().ok_or_else(|| LangError::new("gprmerge needs two rules", 0))?)?;
                let b = gpr_from_value(argv.get(1).ok_or_else(|| LangError::new("gprmerge needs two rules", 0))?)?;
                let merged = Gpr::Or(Box::new(a), Box::new(b));
                Ok(vec![Value::Str(gpr_text(&merged))])
            }
            "fastaread" => {
                let path = as_string(argv.first().ok_or_else(|| LangError::new("fastaread needs a file", 0))?, "file")?;
                let records = fasta_records(&read_text_file(&path)?);
                if nout >= 2 {
                    return Ok(vec![
                        list_value(records.iter().map(|(h, _)| h.clone()).collect()),
                        list_value(records.iter().map(|(_, s)| s.clone()).collect()),
                    ]);
                }
                let mut sa = StructArray::with_dims(records.len().max(1), 1);
                sa.add_field("Header");
                sa.add_field("Sequence");
                for (i, (h, s)) in records.iter().enumerate() {
                    sa.set_field(i, "Header", Value::Str(h.clone()))?;
                    sa.set_field(i, "Sequence", Value::Str(s.clone()))?;
                }
                Ok(vec![Value::Struct(sa)])
            }
            "fastawrite" => {
                let path = as_string(argv.first().ok_or_else(|| LangError::new("fastawrite needs a file", 0))?, "file")?;
                let (headers, seqs) = match argv.get(1) {
                    Some(Value::Struct(sa)) => {
                        let n = sa.rows();
                        let mut h = Vec::with_capacity(n);
                        let mut q = Vec::with_capacity(n);
                        for i in 0..n {
                            h.push(sa.field_at(i, "Header").and_then(|v| as_string(v, "Header").ok()).unwrap_or_default());
                            q.push(sa.field_at(i, "Sequence").and_then(|v| as_string(v, "Sequence").ok()).unwrap_or_default());
                        }
                        (h, q)
                    }
                    Some(v) => {
                        let h = as_string_list(v, "headers")?;
                        let q = as_string_list(argv.get(2).ok_or_else(|| LangError::new("fastawrite needs sequences", 0))?, "sequences")?;
                        (h, q)
                    }
                    None => return Err(LangError::new("fastawrite needs headers and sequences", 0)),
                };
                if headers.len() != seqs.len() {
                    return Err(LangError::new("fastawrite: headers and sequences must have the same length", 0));
                }
                let mut out = String::new();
                for (h, s) in headers.iter().zip(seqs.iter()) {
                    out.push('>');
                    out.push_str(h.trim_start_matches('>'));
                    out.push('\n');
                    for chunk in s.as_bytes().chunks(80) {
                        out.push_str(&String::from_utf8_lossy(chunk));
                        out.push('\n');
                    }
                }
                crate::write_file(std::path::Path::new(&path), out.as_bytes())
                    .map_err(|e| LangError::new(format!("fastawrite: cannot write {path}: {e}"), 0))?;
                Ok(vec![])
            }
            "genbankread" => {
                let path = as_string(argv.first().ok_or_else(|| LangError::new("genbankread needs a file", 0))?, "file")?;
                let records = genbank_records(&read_text_file(&path)?);
                let n = records.len().max(1);
                let mut sa = StructArray::with_dims(n, 1);
                for r in &records {
                    if let Value::Struct(s) = r {
                        for f in s.field_names() {
                            sa.add_field(&f);
                        }
                    }
                }
                for (i, r) in records.iter().enumerate() {
                    if let Value::Struct(s) = r {
                        for f in s.field_names() {
                            if let Some(v) = s.field_at(0, &f) {
                                sa.set_field(i, &f, v.clone())?;
                            }
                        }
                    }
                }
                Ok(vec![Value::Struct(sa)])
            }
            _ => Err(LangError::new(format!("{name}: not a bio builtin"), 0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_parsing_and_weights() {
        let c = parse_formula("C6H12O6").unwrap();
        assert_eq!(c, vec![("C".into(), 6.0), ("H".into(), 12.0), ("O".into(), 6.0)]);
        let c = parse_formula("C55H80CoN15O11").unwrap();
        assert_eq!(c.iter().find(|(e, _)| e == "Co").map(|(_, n)| *n), Some(1.0));
        let c = parse_formula("C62H90N13O14P.C10H11N5O3.Co").unwrap();
        assert_eq!(c.iter().find(|(e, _)| e == "C").map(|(_, n)| *n), Some(72.0));
        assert!(parse_formula("6C").is_err());
    }

    #[test]
    fn reaction_and_gpr_parsing() {
        let r = parse_reaction("2 H2O + ATP <=> ADP + Pi").unwrap();
        assert_eq!(r.reactants.len(), 2);
        assert!(r.reversible);
        let ions = split_side("H+ + OH-").unwrap();
        assert_eq!(ions.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), vec!["H+", "OH-"]);
        assert!(reactions_equal(&r, &parse_reaction("ADP + Pi <=> 2 H2O + ATP").unwrap(), true, false));
        let g = parse_gpr("g1 & (g2 | g3)").unwrap();
        let set: BTreeSet<String> = ["g1".into(), "g3".into()].into_iter().collect();
        assert!(gpr_eval(&g, &set));
        let mut genes = BTreeSet::new();
        gpr_collect(&g, &mut genes);
        assert_eq!(genes.len(), 3);
    }
}
