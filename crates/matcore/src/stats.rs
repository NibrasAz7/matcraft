//! Statistical distribution primitives.
//!
//! Clean-room implementations of the special functions behind the common
//! MATLAB distribution functions: error function, log-gamma, regularized
//! incomplete gamma/beta, and the normal/Exponential/Uniform/Poisson/
//! Binomial/t/F CDFs. The numerical approximations are the standard
//! Abramowitz–Stegun / Lanczos / Lentz continued-fraction forms.

use std::f64::consts::{PI, SQRT_2};

/// Error function (Abramowitz–Stegun 7.1.26, |error| < 1.5e-7).
pub fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t + 0.254_829_592) * t;
    sign * (1.0 - poly * (-x * x).exp())
}

pub fn erfc(x: f64) -> f64 {
    1.0 - erf(x)
}

pub fn norm_pdf(x: f64, mu: f64, sigma: f64) -> f64 {
    if sigma.is_nan() || sigma <= 0.0 {
        return f64::NAN;
    }
    let z = (x - mu) / sigma;
    (-0.5 * z * z).exp() / (sigma * (2.0 * PI).sqrt())
}

pub fn norm_cdf(x: f64, mu: f64, sigma: f64) -> f64 {
    if sigma.is_nan() || sigma <= 0.0 {
        return f64::NAN;
    }
    0.5 * erfc(-(x - mu) / (sigma * SQRT_2))
}

/// Standard-normal inverse CDF (Acklam's rational approximation plus one
/// Halley refinement).
pub fn norm_inv(p: f64, mu: f64, sigma: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] =
        [-5.447_609_879_822_406e1, 1.615_858_368_580_409e2, -1.556_989_798_598_866e2, 6.680_131_188_771_972e1, -1.328_068_155_288_572e1];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [7.784_695_709_041_462e-3, 3.224_671_290_700_398e-1, 2.445_134_137_142_996, 3.754_408_661_907_416];
    let p_low = 0.024_25;
    let p_high = 1.0 - p_low;
    let x = if p < p_low {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= p_high {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    mu + sigma * x
}

/// Lanczos log-gamma (g = 7, n = 9).
pub fn gammaln(x: f64) -> f64 {
    if x < 0.5 {
        return (PI / (PI * x).sin()).ln() - gammaln(1.0 - x);
    }
    const C: [f64; 9] = [
        0.999_999_999_999_81,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    let x = x - 1.0;
    let t = x + 7.5;
    let mut sum = C[0];
    for (i, &c) in C.iter().enumerate().skip(1) {
        sum += c / (x + i as f64);
    }
    0.5 * (2.0 * PI).ln() + (x + 0.5) * t.ln() - t + sum.ln()
}

fn gamma_q_cf(a: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / TINY;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..=200 {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b + an / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-15 {
            break;
        }
    }
    (-x + a * x.ln() - gammaln(a)).exp() * h
}

/// Regularized lower incomplete gamma `P(a, x)`.
pub fn gamma_p(a: f64, x: f64) -> f64 {
    if a.is_nan() || a <= 0.0 || x < 0.0 || x.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 {
        return 0.0;
    }
    if x < a + 1.0 {
        let mut ap = a;
        let mut sum = 1.0 / a;
        let mut del = sum;
        for _ in 0..200 {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        sum * (-x + a * x.ln() - gammaln(a)).exp()
    } else {
        1.0 - gamma_q_cf(a, x)
    }
}

pub fn gamma_q(a: f64, x: f64) -> f64 {
    1.0 - gamma_p(a, x)
}

fn beta_cf(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=200 {
        let m2 = 2 * m;
        let aa = (m as f64) * (b - m as f64) * x / ((qam + m2 as f64) * (a + m2 as f64));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -((a + m as f64) * (qab + m as f64) * x) / ((a + m2 as f64) * (qap + m2 as f64));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-15 {
            break;
        }
    }
    h
}

/// Regularized incomplete beta `I_x(a, b)`.
pub fn beta_i(a: f64, b: f64, x: f64) -> f64 {
    if a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 || !(0.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    if x == 0.0 {
        return 0.0;
    }
    if x == 1.0 {
        return 1.0;
    }
    let bt = (gammaln(a + b) - gammaln(a) - gammaln(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * beta_cf(a, b, x) / a
    } else {
        1.0 - bt * beta_cf(b, a, 1.0 - x) / b
    }
}

pub fn exp_pdf(x: f64, mu: f64) -> f64 {
    if mu.is_nan() || mu <= 0.0 || x < 0.0 {
        return 0.0;
    }
    (-x / mu).exp() / mu
}

pub fn exp_cdf(x: f64, mu: f64) -> f64 {
    if mu.is_nan() || mu <= 0.0 {
        return f64::NAN;
    }
    if x < 0.0 {
        0.0
    } else {
        1.0 - (-x / mu).exp()
    }
}

pub fn exp_inv(p: f64, mu: f64) -> f64 {
    if p < 0.0 || p > 1.0 {
        f64::NAN
    } else {
        -mu * (1.0 - p).ln()
    }
}

pub fn tls_pdf(x: f64, mu: f64, sigma: f64, nu: f64) -> f64 {
    if mu.is_nan() || sigma.is_nan() || sigma <= 0.0 || nu.is_nan() || nu <= 0.0 {
        return f64::NAN;
    }
    let z = (x - mu) / sigma;
    let log_pdf = gammaln(0.5 * (nu + 1.0)) - gammaln(0.5 * nu) - 0.5 * (nu * PI).ln() - sigma.ln() - 0.5 * (nu + 1.0) * (1.0 + z * z / nu).ln();
    log_pdf.exp()
}

pub fn tls_cdf(x: f64, mu: f64, sigma: f64, nu: f64) -> f64 {
    if sigma.is_nan() || sigma <= 0.0 || nu.is_nan() || nu <= 0.0 {
        return f64::NAN;
    }
    t_cdf((x - mu) / sigma, nu)
}

pub fn tls_inv(p: f64, mu: f64, sigma: f64, nu: f64) -> f64 {
    if p.is_nan() || !(0.0..=1.0).contains(&p) || sigma.is_nan() || sigma <= 0.0 || nu.is_nan() || nu <= 0.0 {
        return f64::NAN;
    }
    mu + sigma * t_inv(p, nu)
}

pub fn weibull_pdf(x: f64, a: f64, b: f64) -> f64 {
    if a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 || x < 0.0 {
        return 0.0;
    }
    if x == 0.0 {
        return if a < 1.0 {
            f64::INFINITY
        } else if a == 1.0 {
            1.0 / b
        } else {
            0.0
        };
    }
    let z = x / b;
    (a / b) * z.powf(a - 1.0) * (-z.powf(a)).exp()
}

pub fn weibull_cdf(x: f64, a: f64, b: f64) -> f64 {
    if a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 {
        return f64::NAN;
    }
    if x <= 0.0 {
        0.0
    } else {
        1.0 - (-(x / b).powf(a)).exp()
    }
}

pub fn weibull_inv(p: f64, a: f64, b: f64) -> f64 {
    if p.is_nan() || !(0.0..=1.0).contains(&p) || a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 {
        return f64::NAN;
    }
    if p == 0.0 {
        return 0.0;
    }
    if p == 1.0 {
        return f64::INFINITY;
    }
    b * (-(1.0 - p).ln()).powf(1.0 / a)
}

pub fn logn_pdf(x: f64, mu: f64, sigma: f64) -> f64 {
    if mu.is_nan() || sigma.is_nan() || sigma <= 0.0 || x <= 0.0 {
        return 0.0;
    }
    let z = (x.ln() - mu) / sigma;
    (-0.5 * z * z).exp() / (x * sigma * (2.0 * PI).sqrt())
}

pub fn logn_cdf(x: f64, mu: f64, sigma: f64) -> f64 {
    if mu.is_nan() || sigma.is_nan() || sigma <= 0.0 {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 0.0;
    }
    norm_cdf((x.ln() - mu) / sigma, 0.0, 1.0)
}

pub fn logn_inv(p: f64, mu: f64, sigma: f64) -> f64 {
    if p.is_nan() || !(0.0..=1.0).contains(&p) || mu.is_nan() || sigma.is_nan() || sigma <= 0.0 {
        return f64::NAN;
    }
    if p == 0.0 {
        return 0.0;
    }
    if p == 1.0 {
        return f64::INFINITY;
    }
    (mu + sigma * norm_inv(p, 0.0, 1.0)).exp()
}

pub fn unif_pdf(x: f64, a: f64, b: f64) -> f64 {
    if b <= a || x < a || x > b {
        0.0
    } else {
        1.0 / (b - a)
    }
}

pub fn unif_cdf(x: f64, a: f64, b: f64) -> f64 {
    if b <= a {
        f64::NAN
    } else if x < a {
        0.0
    } else if x > b {
        1.0
    } else {
        (x - a) / (b - a)
    }
}

pub fn unif_inv(p: f64, a: f64, b: f64) -> f64 {
    if !(0.0..=1.0).contains(&p) {
        f64::NAN
    } else {
        a + p * (b - a)
    }
}

pub fn poisson_pdf(k: f64, lambda: f64) -> f64 {
    if lambda.is_nan() || lambda < 0.0 || k < 0.0 || k.fract() != 0.0 {
        return 0.0;
    }
    if lambda == 0.0 {
        return if k == 0.0 { 1.0 } else { 0.0 };
    }
    (-lambda + k * lambda.ln() - gammaln(k + 1.0)).exp()
}

pub fn poisson_cdf(k: f64, lambda: f64) -> f64 {
    if lambda.is_nan() || lambda < 0.0 || k < 0.0 {
        return 0.0;
    }
    gamma_q(k.floor() + 1.0, lambda)
}

pub fn poisson_inv(p: f64, lambda: f64) -> f64 {
    if !(0.0..1.0).contains(&p) || lambda.is_nan() || lambda < 0.0 {
        return f64::NAN;
    }
    let mut hi = (lambda * 2.0 + 10.0).ceil();
    while poisson_cdf(hi, lambda) < p && hi < 1e9 {
        hi *= 2.0;
    }
    let mut lo = 0.0;
    while lo < hi {
        let mid = ((lo + hi) / 2.0).floor();
        if poisson_cdf(mid, lambda) >= p {
            hi = mid;
        } else {
            lo = mid + 1.0;
        }
    }
    lo
}

pub fn binom_pdf(k: f64, n: f64, p: f64) -> f64 {
    if n < 0.0 || n.fract() != 0.0 || k < 0.0 || k > n || k.fract() != 0.0 || !(0.0..=1.0).contains(&p) {
        return 0.0;
    }
    if p == 0.0 {
        return if k == 0.0 { 1.0 } else { 0.0 };
    }
    if p == 1.0 {
        return if k == n { 1.0 } else { 0.0 };
    }
    (gammaln(n + 1.0) - gammaln(k + 1.0) - gammaln(n - k + 1.0) + k * p.ln() + (n - k) * (1.0 - p).ln()).exp()
}

pub fn binom_cdf(k: f64, n: f64, p: f64) -> f64 {
    if n < 0.0 || n.fract() != 0.0 || !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    if k < 0.0 {
        return 0.0;
    }
    if k >= n {
        return 1.0;
    }
    beta_i(n - k.floor(), k.floor() + 1.0, 1.0 - p)
}

pub fn binom_inv(p: f64, n: f64, prob: f64) -> f64 {
    if !(0.0..1.0).contains(&p) || n < 0.0 || n.fract() != 0.0 {
        return f64::NAN;
    }
    let mut lo = 0.0;
    let mut hi = n;
    while lo < hi {
        let mid = ((lo + hi) / 2.0).floor();
        if binom_cdf(mid, n, prob) >= p {
            hi = mid;
        } else {
            lo = mid + 1.0;
        }
    }
    lo
}

/// Student-t CDF.
pub fn t_cdf(t: f64, nu: f64) -> f64 {
    if nu.is_nan() || nu <= 0.0 {
        return f64::NAN;
    }
    let x = nu / (nu + t * t);
    let tail = 0.5 * beta_i(nu / 2.0, 0.5, x);
    if t >= 0.0 {
        1.0 - tail
    } else {
        tail
    }
}

/// F CDF.
pub fn f_cdf(f: f64, d1: f64, d2: f64) -> f64 {
    if f <= 0.0 {
        return 0.0;
    }
    if d1.is_nan() || d1 <= 0.0 || d2.is_nan() || d2 <= 0.0 {
        return f64::NAN;
    }
    let x = d1 * f / (d1 * f + d2);
    beta_i(d1 / 2.0, d2 / 2.0, x)
}

/// Digamma function `psi(x)` (asymptotic expansion with recurrence).
pub fn digamma(mut x: f64) -> f64 {
    if x.is_nan() || x <= 0.0 {
        return f64::NAN;
    }
    let mut result = 0.0;
    while x < 20.0 {
        result -= 1.0 / x;
        x += 1.0;
    }
    let inv = 1.0 / x;
    let inv2 = inv * inv;
    result + x.ln() - 0.5 * inv - inv2 * (1.0 / 12.0 - inv2 * (1.0 / 120.0 - inv2 / 252.0))
}

/// Trigamma function `psi'(x)`.
pub fn trigamma(mut x: f64) -> f64 {
    if x.is_nan() || x <= 0.0 {
        return f64::NAN;
    }
    let mut result = 0.0;
    while x < 20.0 {
        result += 1.0 / (x * x);
        x += 1.0;
    }
    let inv = 1.0 / x;
    let inv2 = inv * inv;
    result + inv * (1.0 + 0.5 * inv + inv2 * (1.0 / 6.0 - inv2 * (1.0 / 30.0 - inv2 / 42.0)))
}

/// Gamma PDF with shape `a` and scale `b`.
pub fn gamma_pdf(x: f64, a: f64, b: f64) -> f64 {
    if a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 || x < 0.0 {
        return 0.0;
    }
    if x == 0.0 {
        return if a < 1.0 {
            f64::INFINITY
        } else if a == 1.0 {
            1.0 / b
        } else {
            0.0
        };
    }
    ((a - 1.0) * x.ln() - x / b - a * b.ln() - gammaln(a)).exp()
}

/// Gamma CDF with shape `a` and scale `b`.
pub fn gamma_cdf(x: f64, a: f64, b: f64) -> f64 {
    if a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 {
        return f64::NAN;
    }
    if x <= 0.0 {
        0.0
    } else {
        gamma_p(a, x / b)
    }
}

/// Gamma quantile with shape `a` and scale `b`.
pub fn gamma_inv(p: f64, a: f64, b: f64) -> f64 {
    if p.is_nan() || !(0.0..=1.0).contains(&p) || a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 {
        return f64::NAN;
    }
    if p == 0.0 {
        return 0.0;
    }
    if p == 1.0 {
        return f64::INFINITY;
    }
    let mut hi = (a * b).max(1.0);
    while gamma_cdf(hi, a, b) < p && hi < 1e12 {
        hi *= 2.0;
    }
    let mut lo = 0.0;
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if gamma_cdf(mid, a, b) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Beta PDF with shape parameters `a` and `b`.
pub fn beta_pdf(x: f64, a: f64, b: f64) -> f64 {
    if a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 || !(0.0..=1.0).contains(&x) {
        return 0.0;
    }
    if x == 0.0 {
        return if a < 1.0 {
            f64::INFINITY
        } else if a == 1.0 {
            b
        } else {
            0.0
        };
    }
    if x == 1.0 {
        return if b < 1.0 {
            f64::INFINITY
        } else if b == 1.0 {
            a
        } else {
            0.0
        };
    }
    ((a - 1.0) * x.ln() + (b - 1.0) * (1.0 - x).ln() - gammaln(a) - gammaln(b) + gammaln(a + b)).exp()
}

/// Beta CDF.
pub fn beta_cdf(x: f64, a: f64, b: f64) -> f64 {
    if a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 {
        return f64::NAN;
    }
    beta_i(a, b, x.clamp(0.0, 1.0))
}

/// Beta quantile.
pub fn beta_inv(p: f64, a: f64, b: f64) -> f64 {
    if p.is_nan() || !(0.0..=1.0).contains(&p) || a.is_nan() || a <= 0.0 || b.is_nan() || b <= 0.0 {
        return f64::NAN;
    }
    if p == 0.0 {
        return 0.0;
    }
    if p == 1.0 {
        return 1.0;
    }
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if beta_cdf(mid, a, b) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

pub fn chi2_pdf(x: f64, nu: f64) -> f64 {
    gamma_pdf(x, nu / 2.0, 2.0)
}

pub fn chi2_cdf(x: f64, nu: f64) -> f64 {
    gamma_cdf(x, nu / 2.0, 2.0)
}

pub fn chi2_inv(p: f64, nu: f64) -> f64 {
    gamma_inv(p, nu / 2.0, 2.0)
}

/// Student-t PDF.
pub fn t_pdf(t: f64, nu: f64) -> f64 {
    if nu.is_nan() || nu <= 0.0 || t.is_nan() {
        return f64::NAN;
    }
    let log = gammaln((nu + 1.0) / 2.0) - gammaln(nu / 2.0) - 0.5 * (nu * std::f64::consts::PI).ln() - (nu + 1.0) / 2.0 * (1.0 + t * t / nu).ln();
    log.exp()
}

/// Student-t quantile.
pub fn t_inv(p: f64, nu: f64) -> f64 {
    if p.is_nan() || p <= 0.0 || p >= 1.0 || nu.is_nan() || nu <= 0.0 {
        return f64::NAN;
    }
    let mut lo = -1.0f64;
    let mut hi = 1.0f64;
    while t_cdf(lo, nu) > p && lo > -1e12 {
        lo *= 2.0;
    }
    while t_cdf(hi, nu) < p && hi < 1e12 {
        hi *= 2.0;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if t_cdf(mid, nu) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// F PDF.
pub fn f_pdf(x: f64, d1: f64, d2: f64) -> f64 {
    if d1.is_nan() || d1 <= 0.0 || d2.is_nan() || d2 <= 0.0 || x < 0.0 {
        return 0.0;
    }
    if x == 0.0 {
        return if d1 < 2.0 {
            f64::INFINITY
        } else if d1 == 2.0 {
            1.0
        } else {
            0.0
        };
    }
    let a = d1 / 2.0;
    let b = d2 / 2.0;
    let log = a * (d1 / d2).ln() + (a - 1.0) * x.ln() - (gammaln(a) + gammaln(b) - gammaln(a + b)) - (a + b) * (1.0 + d1 * x / d2).ln();
    log.exp()
}

/// F quantile.
pub fn f_inv(p: f64, d1: f64, d2: f64) -> f64 {
    if p.is_nan() || !(0.0..=1.0).contains(&p) || d1.is_nan() || d1 <= 0.0 || d2.is_nan() || d2 <= 0.0 {
        return f64::NAN;
    }
    if p == 0.0 {
        return 0.0;
    }
    if p == 1.0 {
        return f64::INFINITY;
    }
    let mut hi = 1.0;
    while f_cdf(hi, d1, d2) < p && hi < 1e12 {
        hi *= 2.0;
    }
    let mut lo = 0.0;
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if f_cdf(mid, d1, d2) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_known_values() {
        assert!((norm_cdf(0.0, 0.0, 1.0) - 0.5).abs() < 1e-8);
        assert!((norm_cdf(1.96, 0.0, 1.0) - 0.975_002_104_851_780_1).abs() < 1e-6);
        assert!((norm_pdf(0.0, 0.0, 1.0) - 0.398_942_280_401_432_7).abs() < 1e-12);
        assert!((norm_inv(0.975, 0.0, 1.0) - 1.959_963_984_540_054).abs() < 1e-8);
        assert!((norm_inv(norm_cdf(1.234, 0.0, 1.0), 0.0, 1.0) - 1.234).abs() < 1e-6);
    }

    #[test]
    fn gamma_beta_known_values() {
        assert!((gamma_p(1.0, 1.0) - (1.0 - (-1f64).exp())).abs() < 1e-12);
        assert!((beta_i(2.0, 3.0, 0.5) - 0.6875).abs() < 1e-12);
        assert!((gammaln(5.0) - 24f64.ln()).abs() < 1e-12);
    }

    #[test]
    fn discrete_distributions() {
        assert!((poisson_pdf(3.0, 2.0) - 0.180_447_044_315_483_6).abs() < 1e-12);
        assert!((poisson_cdf(3.0, 2.0) - 0.857_123_460_498_547).abs() < 1e-12);
        assert!((binom_pdf(3.0, 10.0, 0.5) - 0.117_187_5).abs() < 1e-12);
        assert!((binom_cdf(3.0, 10.0, 0.5) - 0.171_875).abs() < 1e-12);
        assert_eq!(poisson_inv(0.857_123_456_790_209, 2.0), 3.0);
        assert_eq!(binom_inv(0.171_875, 10.0, 0.5), 3.0);
    }

    #[test]
    fn t_and_f() {
        assert!((t_cdf(0.0, 10.0) - 0.5).abs() < 1e-12);
        assert!((t_cdf(1.96, 1_000_000.0) - 0.975_002).abs() < 1e-5);
        assert!((f_cdf(1.0, 10.0, 10.0) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn continuous_distribution_known_values() {
        // Reference values from SciPy 1.16.
        assert!((gamma_pdf(2.0, 3.0, 2.0) - 0.091_969_860_292_860_57).abs() < 1e-12);
        assert!((gamma_cdf(2.0, 3.0, 2.0) - 0.080_301_397_071_394_18).abs() < 1e-12);
        assert!((gamma_inv(0.3, 3.0, 2.0) - 3.827_551_588_254_124).abs() < 1e-9);
        assert!((beta_pdf(0.5, 2.0, 3.0) - 1.5).abs() < 1e-12);
        assert!((beta_cdf(0.5, 2.0, 3.0) - 0.6875).abs() < 1e-12);
        assert!((beta_inv(0.3, 2.0, 3.0) - 0.272_383_942_075_105_36).abs() < 1e-9);
        assert!((chi2_pdf(2.0, 2.0) - 0.183_939_720_585_721_14).abs() < 1e-12);
        assert!((chi2_cdf(2.0, 2.0) - 0.632_120_558_828_557_7).abs() < 1e-12);
        assert!((chi2_inv(0.95, 3.0) - 7.814_727_903_251_179).abs() < 1e-8);
        assert!((t_pdf(0.0, 10.0) - 0.389_108_383_966_031_04).abs() < 1e-12);
        assert!((t_inv(0.975, 10.0) - 2.228_138_851_964_938_5).abs() < 1e-9);
        assert!((f_pdf(1.0, 5.0, 5.0) - 0.424_413_181_578_386_87).abs() < 1e-12);
        assert!((f_cdf(1.0, 5.0, 5.0) - 0.5).abs() < 1e-12);
        assert!((f_inv(0.95, 5.0, 5.0) - 5.050_329_057_632_646).abs() < 1e-8);
    }

    #[test]
    fn weibull_lognormal_reference_values() {
        // SciPy 1.16 reference values.
        assert!((weibull_pdf(1.2, 2.0, 3.0) - 0.227_238_343_724_323_03).abs() < 1e-12);
        assert!((weibull_cdf(1.2, 2.0, 3.0) - 0.147_856_211_033_788_63).abs() < 1e-12);
        assert!((weibull_inv(0.147_856_211_033_788_63, 2.0, 3.0) - 1.2).abs() < 1e-12);
        assert!((logn_pdf(1.0, 0.0, 1.0) - 0.398_942_280_401_432_7).abs() < 1e-12);
        assert!((logn_cdf(1.0, 0.0, 1.0) - 0.5).abs() < 1e-7);
        assert!((logn_inv(0.5, 0.0, 1.0) - 1.0).abs() < 1e-12);
        assert_eq!(weibull_pdf(-1.0, 2.0, 3.0), 0.0);
        assert_eq!(logn_pdf(0.0, 0.0, 1.0), 0.0);
        assert!((tls_pdf(1.0, 0.0, 1.0, 5.0) - 0.219_679_797_350_980_53).abs() < 1e-12);
        assert!((tls_cdf(1.0, 0.0, 1.0, 5.0) - 0.818_391_266_175_438_8).abs() < 1e-9);
        assert!((tls_inv(0.9, 0.0, 1.0, 5.0) - 1.475_884_048_855_821_6).abs() < 1e-9);
        assert!((tls_pdf(3.0, 2.0, 3.0, 7.0) - 0.120_495_861_973_248_4).abs() < 1e-12);
    }

    #[test]
    fn digamma_trigamma() {
        let euler = -std::f64::consts::EULER_GAMMA;
        assert!((digamma(1.0) - euler).abs() < 1e-10);
        assert!((digamma(0.5) - (euler - 2.0 * 2f64.ln())).abs() < 1e-10);
        assert!((trigamma(1.0) - std::f64::consts::PI * std::f64::consts::PI / 6.0).abs() < 1e-10);
    }
}
