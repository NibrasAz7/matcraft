//! Bytecode scaffold — a compile+execute fast path for loop bodies
//! (roadmap: tree-walk → bytecode VM → Cranelift JIT).
//!
//! `for`/`while` bodies are compiled once at loop entry and run as a flat
//! op stream instead of re-walking the AST every iteration. Ops are
//! *structured* (nested `Vec<Op>` blocks, no jump patching) and every
//! value operation delegates to the same `Session` methods the
//! tree-walk uses — `binary`, `unary_op`, `eval_ident`, `eval` — so
//! semantics are identical by construction.
//!
//! `Flow` still propagates: `break`/`continue`/`return` bubble out of
//! nested `If`/block ops to the enclosing loop driver exactly as they do
//! through `exec_block`. Anything outside the subset compiles to
//! [`Op::Exec`]/[`EOp::Eval`], which forwards to `exec_stmt`/`eval`.
//!
//! Cranelift lift point: `EOp` over `Num`/`Load`/`Bin`/`Un`/`Range` is a
//! pure f64 scalar subgraph once `Value::Num` is unboxed — a JIT pass
//! can lower just those runs and keep `Eval` as the deopt path.

use super::*;

/// Expression-level ops — a flat postfix stream run on a `Value` stack.
#[derive(Debug)]
pub(crate) enum EOp {
    /// Literal push (`Num`, imaginary literal, `Str`).
    Const(Value),
    /// `x` — the `Ident` arm: binding → constant → zero-arg call.
    Load(String),
    Bin(BinOp),
    Un(UnOp),
    /// `a:b` or `a:step:b` — `true` when a step operand is on the stack.
    Range(bool),
    /// `a && b` / `a || b`: lhs is popped; `rhs` runs only when the lhs
    /// does not decide the result. Pushes `0`/`1`.
    ShortCirc {
        and: bool,
        rhs: Vec<EOp>,
    },
    /// Everything outside the subset — `self.eval` verbatim.
    Eval(Expr),
}

/// Statement-level ops — nested blocks, not flat jumps.
#[derive(Debug)]
pub(crate) enum Op {
    /// Any statement outside the subset — `self.exec_stmt` verbatim.
    Exec(Stmt),
    /// `name = expr` (no subscript/field target).
    Assign {
        name: String,
        expr: Vec<EOp>,
        suppress: bool,
    },
    /// Bare expression statement — binds `ans`, echoes unless suppressed.
    Echo {
        expr: Vec<EOp>,
        suppress: bool,
    },
    If {
        arms: Vec<(Option<Vec<EOp>>, Vec<Op>)>,
    },
    For {
        var: String,
        iter: Vec<EOp>,
        body: Vec<Op>,
    },
    While {
        cond: Vec<EOp>,
        body: Vec<Op>,
    },
    /// `break` / `continue` / `return` — propagate like the tree-walk.
    Flow(Flow),
}

/// `f(...)` / `c{...}` bare statements can expand to a comma-separated
/// list (`eval_multi` echoes each element) — those stay on `exec_stmt`.
fn csl_root(e: &Expr) -> bool {
    matches!(e, Expr::CallOrIndex(..) | Expr::BraceIndex(..) | Expr::BraceIdx(..) | Expr::Index(..) | Expr::DynField(..))
}

fn compile_expr(e: &Expr) -> Vec<EOp> {
    match e {
        Expr::Num(v) => vec![EOp::Const(Value::Num(*v))],
        Expr::CNum(v) => vec![EOp::Const(Value::Cplx(CMatrix::scalar(CNum::new(0.0, *v))))],
        Expr::Str(s) => vec![EOp::Const(Value::Str(s.clone()))],
        Expr::Ident(n) => vec![EOp::Load(n.clone())],
        Expr::Binary(op @ (BinOp::ScAnd | BinOp::ScOr), a, b) => {
            let mut ops = compile_expr(a);
            ops.push(EOp::ShortCirc { and: *op == BinOp::ScAnd, rhs: compile_expr(b) });
            ops
        }
        Expr::Binary(op, a, b) => {
            let mut ops = compile_expr(a);
            ops.extend(compile_expr(b));
            ops.push(EOp::Bin(*op));
            ops
        }
        Expr::Unary(op, e) => {
            let mut ops = compile_expr(e);
            ops.push(EOp::Un(*op));
            ops
        }
        Expr::Range(a, step, b) => {
            let mut ops = compile_expr(a);
            if let Some(s) = step {
                ops.extend(compile_expr(s));
            }
            ops.extend(compile_expr(b));
            ops.push(EOp::Range(step.is_some()));
            ops
        }
        _ => vec![EOp::Eval(e.clone())],
    }
}

fn compile_stmt(s: &Stmt) -> Op {
    match s {
        Stmt::Assign { name, index: None, value, suppress, .. } => Op::Assign { name: name.clone(), expr: compile_expr(value), suppress: *suppress },
        Stmt::Expr(e, suppress) if !csl_root(e) => Op::Echo { expr: compile_expr(e), suppress: *suppress },
        Stmt::If { arms } => Op::If { arms: arms.iter().map(|(c, b)| (c.as_ref().map(compile_expr), compile_block(b))).collect() },
        Stmt::For { var, iter, body } => Op::For { var: var.clone(), iter: compile_expr(iter), body: compile_block(body) },
        Stmt::While { cond, body } => Op::While { cond: compile_expr(cond), body: compile_block(body) },
        Stmt::Break => Op::Flow(Flow::Break),
        Stmt::Continue => Op::Flow(Flow::Continue),
        Stmt::Return => Op::Flow(Flow::Return),
        _ => Op::Exec(s.clone()),
    }
}

/// Compile a statement list once — called at loop entry so the op stream
/// is reused every iteration.
pub(crate) fn compile_block(body: &[Stmt]) -> Vec<Op> {
    body.iter().map(compile_stmt).collect()
}

/// Number of iterations `for v = it` runs — the column count of `it`,
/// matching the tree-walk's per-type iteration.
impl Session {
    pub(crate) fn for_ncols(&self, it: &Value) -> Result<usize, LangError> {
        let tmp: Matrix;
        let m = match it {
            Value::Cell(ca) => return Ok(ca.cols),
            Value::Cplx(cm) => return Ok(cm.cols()),
            Value::Struct(sa) => return Ok(sa.cols()),
            // Index `Value::Mat` in place — materializing a numeric copy
            // per column would make long ranges quadratic.
            Value::Mat(mm) => mm,
            _ => {
                tmp = it.to_numeric()?;
                &tmp
            }
        };
        // On N-D arrays the tail dims merge into the column count.
        Ok(if m.ndims() > 2 {
            if m.rows() > 0 {
                m.numel() / m.rows()
            } else {
                0
            }
        } else {
            m.cols()
        })
    }

    /// The loop variable's value at column `c` — a rows×1 slice of `it`
    /// (scalar for a row vector), per type like the tree-walk.
    pub(crate) fn for_col(&self, it: &Value, c: usize) -> Result<Value, LangError> {
        let tmp: Matrix;
        let m = match it {
            Value::Cell(ca) => {
                let mut items = Vec::with_capacity(ca.rows);
                for r in 0..ca.rows {
                    items.push(ca.get(r, c).cloned().unwrap_or_else(CellArray::empty));
                }
                return Ok(Value::Cell(CellArray { rows: ca.rows, cols: 1, items }));
            }
            Value::Cplx(cm) => {
                let mut colv = Vec::with_capacity(cm.rows());
                for r in 0..cm.rows() {
                    colv.push(cm.get(r, c).unwrap_or(CNum::real(0.0)));
                }
                let colm = CMatrix::from_col_major(cm.rows(), 1, colv).map_err(lang)?;
                return Ok(cmat_or_value(colm));
            }
            Value::Struct(sa) => {
                let mut sub = StructArray::with_dims(sa.rows(), 1);
                for (name, vals) in &sa.fields {
                    let col: Vec<Value> =
                        (0..sa.rows()).map(|r| vals.get(r + c * sa.rows()).cloned().unwrap_or_else(|| Value::Mat(Matrix::zeros(0, 0)))).collect();
                    sub.fields.push((name.clone(), col));
                }
                return Ok(Value::Struct(sub));
            }
            Value::Mat(mm) => mm,
            _ => {
                tmp = it.to_numeric()?;
                &tmp
            }
        };
        let mut colv = Vec::with_capacity(m.rows());
        for r in 0..m.rows() {
            colv.push(m.get_linear(c.saturating_mul(m.rows()).saturating_add(r)).unwrap_or(0.0));
        }
        Ok(Value::Mat(Matrix::col_vector(colv)))
    }

    /// Run a compiled expression on a `Value` stack.
    pub(crate) fn vm_expr(&mut self, ops: &[EOp]) -> Result<Value, LangError> {
        let mut stack: Vec<Value> = Vec::with_capacity(8);
        for op in ops {
            match op {
                EOp::Const(v) => stack.push(v.clone()),
                EOp::Load(name) => {
                    let v = self.eval_ident(name)?.ok_or_else(|| LangError::new("expression produced no value", 0))?;
                    stack.push(v);
                }
                EOp::Bin(op) => {
                    let b = stack.pop().ok_or_else(|| LangError::new("vm stack underflow", 0))?;
                    let a = stack.pop().ok_or_else(|| LangError::new("vm stack underflow", 0))?;
                    stack.push(self.binary(*op, a, b)?);
                }
                EOp::Un(op) => {
                    let v = stack.pop().ok_or_else(|| LangError::new("vm stack underflow", 0))?;
                    let r = self.unary_op(*op, v)?.ok_or_else(|| LangError::new("expression produced no value", 0))?;
                    stack.push(r);
                }
                EOp::Range(has_step) => {
                    let hi = stack.pop().ok_or_else(|| LangError::new("vm stack underflow", 0))?.scalar()?;
                    let st = if *has_step { stack.pop().ok_or_else(|| LangError::new("vm stack underflow", 0))?.scalar()? } else { 1.0 };
                    let lo = stack.pop().ok_or_else(|| LangError::new("vm stack underflow", 0))?.scalar()?;
                    stack.push(range_mat(lo, st, hi)?);
                }
                EOp::ShortCirc { and, rhs } => {
                    let lhs = stack.pop().ok_or_else(|| LangError::new("vm stack underflow", 0))?.truthy()?;
                    let r = if (*and && !lhs) || (!*and && lhs) { !*and } else { self.vm_expr(rhs)?.truthy()? };
                    stack.push(Value::Num(if r { 1.0 } else { 0.0 }));
                }
                EOp::Eval(e) => stack.push(self.eval(e)?),
            }
        }
        stack.pop().ok_or_else(|| LangError::new("expression produced no value", 0))
    }

    /// Run a compiled block; returns `Flow` exactly like `exec_block`.
    pub(crate) fn vm_block(&mut self, ops: &[Op], out: &mut String) -> Result<Flow, LangError> {
        for op in ops {
            let f = match op {
                Op::Exec(s) => self.exec_stmt(s, out)?,
                Op::Assign { name, expr, suppress } => {
                    let v = self.vm_expr(expr)?;
                    self.put_var(name, v.clone());
                    if !suppress {
                        let long = self.fmt_long;
                        out.push_str(&format!("{name} =\n\n{}\n\n", fmt_value(&v, long)));
                    }
                    Flow::Normal
                }
                Op::Echo { expr, suppress } => {
                    let v = self.vm_expr(expr)?;
                    self.put_var("ans", v.clone());
                    if !suppress {
                        let long = self.fmt_long;
                        out.push_str(&format!("ans =\n\n{}\n\n", fmt_value(&v, long)));
                    }
                    Flow::Normal
                }
                Op::If { arms } => {
                    let mut f = Flow::Normal;
                    for (cond, body) in arms {
                        let take = match cond {
                            Some(c) => self.vm_expr(c)?.truthy()?,
                            None => true,
                        };
                        if take {
                            f = self.vm_block(body, out)?;
                            break;
                        }
                    }
                    f
                }
                Op::For { var, iter, body } => {
                    let it = self.vm_expr(iter)?;
                    let ncols = self.for_ncols(&it)?;
                    let mut f = Flow::Normal;
                    for c in 0..ncols {
                        let v = self.for_col(&it, c)?;
                        self.put_var(var, v);
                        match self.vm_block(body, out)? {
                            Flow::Normal => {}
                            Flow::Break => break,
                            Flow::Continue => continue,
                            Flow::Return => {
                                f = Flow::Return;
                                break;
                            }
                        }
                    }
                    f
                }
                Op::While { cond, body } => {
                    let mut n = 0usize;
                    let mut f = Flow::Normal;
                    while self.vm_expr(cond)?.truthy()? {
                        n += 1;
                        if n > MAX_LOOP_ITERS {
                            return Err(LangError::new("loop iteration limit exceeded", 0));
                        }
                        match self.vm_block(body, out)? {
                            Flow::Normal => {}
                            Flow::Break => break,
                            Flow::Continue => continue,
                            Flow::Return => {
                                f = Flow::Return;
                                break;
                            }
                        }
                    }
                    f
                }
                Op::Flow(f) => *f,
            };
            if f != Flow::Normal {
                return Ok(f);
            }
        }
        Ok(Flow::Normal)
    }
}
