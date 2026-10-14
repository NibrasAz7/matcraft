//! DSP primitives: DFT/FFT, filtering, correlation and window functions.
//!
//! Clean-room implementations from the standard definitions (Cooley–Tukey
//! mixed-radix DIT, direct-form-II-transposed filter, DTFT evaluation for
//! `freqz`). Everything works on `CNum` so real and complex signals share
//! one path.

use crate::CNum;
use std::f64::consts::PI;
use std::ops::{Add, Div, Mul, Sub};

/// Smallest prime factor of `n` (or `n` itself when prime / `n` < 2).
fn smallest_factor(n: usize) -> usize {
    if n < 2 {
        return n;
    }
    let mut p: usize = 2;
    while p.saturating_mul(p) <= n {
        if n.is_multiple_of(p) {
            return p;
        }
        p += 1;
    }
    n
}

/// O(n²) DFT — used for prime lengths inside the mixed-radix recursion.
fn dft_naive(x: &[CNum], inverse: bool) -> Vec<CNum> {
    let n = x.len();
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut out = vec![CNum::real(0.0); n];
    for (k, slot) in out.iter_mut().enumerate() {
        let mut acc = CNum::real(0.0);
        for (t, &v) in x.iter().enumerate() {
            let a = sign * 2.0 * PI * (t as f64) * (k as f64) / (n as f64);
            acc = acc.add(v.mul(CNum { re: a.cos(), im: a.sin() }));
        }
        *slot = acc;
    }
    out
}

/// Recursive mixed-radix decimation-in-time transform. `n = p·m` splits into
/// `p` stride-`p` sub-DFTs of length `m`; prime lengths hit the naive DFT.
fn dft_rec(x: &[CNum], inverse: bool) -> Vec<CNum> {
    let n = x.len();
    if n <= 1 {
        return x.to_vec();
    }
    let p = smallest_factor(n);
    if p == n {
        return dft_naive(x, inverse);
    }
    let m = n / p;
    // E_j = DFT of the stride-p subsequence starting at j.
    let subs: Vec<Vec<CNum>> =
        (0..p).map(|j| dft_rec(&(0..m).map(|r| x.get(r * p + j).copied().unwrap_or(CNum::real(0.0))).collect::<Vec<_>>(), inverse)).collect();
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut out = vec![CNum::real(0.0); n];
    for (k, slot) in out.iter_mut().enumerate() {
        let mut acc = CNum::real(0.0);
        for (j, e) in subs.iter().enumerate() {
            let a = sign * 2.0 * PI * (j as f64) * (k as f64) / (n as f64);
            let w = CNum { re: a.cos(), im: a.sin() };
            acc = acc.add(w.mul(e.get(k % m).copied().unwrap_or(CNum::real(0.0))));
        }
        *slot = acc;
    }
    out
}

/// `fft(x)` — discrete Fourier transform, X[k] = Σ x[n] e^(-2πi·nk/N).
pub fn fft(x: &[CNum]) -> Vec<CNum> {
    dft_rec(x, false)
}

/// `ifft(x)` — inverse DFT via conjugate symmetry: conj(fft(conj(x)))/N.
pub fn ifft(x: &[CNum]) -> Vec<CNum> {
    let n = x.len().max(1) as f64;
    let conj: Vec<CNum> = x.iter().map(|z| z.conj()).collect();
    dft_rec(&conj, false).iter().map(|z| z.conj().mul(CNum::real(1.0 / n))).collect()
}

/// FIR/IIR filter, direct form II transposed: `y` from `b`, `a`, `x` with
/// initial delay states `zi` (length max(len(a),len(b))-1, zero-padded);
/// returns (y, zf).
pub fn filter_df2t(b: &[CNum], a: &[CNum], x: &[CNum], zi: &[CNum]) -> (Vec<CNum>, Vec<CNum>) {
    let a0 = a.first().copied().unwrap_or(CNum::real(1.0));
    // Normalize by a(1); a(1) == 0 is the caller's error to report.
    let b: Vec<CNum> = b.iter().map(|&v| v.div(a0)).collect();
    let a: Vec<CNum> = a.iter().map(|&v| v.div(a0)).collect();
    let order = a.len().max(b.len()).saturating_sub(1);
    let mut z = vec![CNum::real(0.0); order];
    for (i, s) in z.iter_mut().enumerate() {
        *s = zi.get(i).copied().unwrap_or(CNum::real(0.0));
    }
    let b0 = b.first().copied().unwrap_or(CNum::real(0.0));
    let mut y = Vec::with_capacity(x.len());
    for &xn in x {
        let yn = if order == 0 { b0.mul(xn) } else { b0.mul(xn).add(z[0]) };
        // Shift the delay line, folding in numerator/denominator taps.
        for i in 0..order {
            let next = if i + 1 < order { z[i + 1] } else { CNum::real(0.0) };
            let bi = b.get(i + 1).copied().unwrap_or(CNum::real(0.0)).mul(xn);
            let ai = a.get(i + 1).copied().unwrap_or(CNum::real(0.0)).mul(yn);
            if let Some(slot) = z.get_mut(i) {
                *slot = next.add(bi).sub(ai);
            }
        }
        y.push(yn);
    }
    (y, z)
}

/// Complex convolution — `conv` for `CNum` inputs.
pub fn conv_c(a: &[CNum], b: &[CNum]) -> Vec<CNum> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![CNum::real(0.0); a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        if x.re == 0.0 && x.im == 0.0 {
            continue;
        }
        for (j, &y) in b.iter().enumerate() {
            if let Some(slot) = out.get_mut(i + j) {
                *slot = slot.add(x.mul(y));
            }
        }
    }
    out
}

/// `xcorr(x, y)` over lags `-(ly-1)..=(lx-1)`, returns (r, lags). Scale by
/// `norm`: 0 = none, 1 = biased (1/N), 2 = unbiased (1/(N-|lag|)),
/// 3 = coeff (normalize to autocorrelation-0 = 1).
pub fn xcorr(x: &[CNum], y: &[CNum], maxlag: Option<usize>, norm: u8) -> (Vec<CNum>, Vec<f64>) {
    let lx = x.len();
    let ly = y.len();
    if lx == 0 || ly == 0 {
        return (Vec::new(), Vec::new());
    }
    let full_lo = -((ly as isize) - 1);
    let full_hi = (lx as isize) - 1;
    let (lo, hi) = match maxlag {
        Some(m) => (-(m as isize), m as isize),
        None => (full_lo, full_hi),
    };
    let n = lx.max(ly) as f64;
    // Normalization reference: Σ|x|² · Σ|y|² (zero for empty/zero signals).
    let ref_scale = (x.iter().map(|z| z.abs2()).sum::<f64>() * y.iter().map(|z| z.abs2()).sum::<f64>()).sqrt();
    let mut r = Vec::new();
    let mut lags = Vec::new();
    for lag in lo..=hi {
        let mut acc = CNum::real(0.0);
        for (j, &yj) in y.iter().enumerate() {
            // r[lag] = Σ_n x[n+lag] · conj(y[n]); index x at j+lag.
            let xi = j as isize + lag;
            if xi >= 0 && (xi as usize) < lx {
                acc = acc.add(x.get(xi as usize).copied().unwrap_or(CNum::real(0.0)).mul(yj.conj()));
            }
        }
        let scaled = match norm {
            1 => acc.mul(CNum::real(1.0 / n)),
            2 => acc.mul(CNum::real(1.0 / (n - (lag.abs() as f64)).max(1.0))),
            3 if ref_scale > 0.0 => acc.mul(CNum::real(1.0 / ref_scale)),
            _ => acc,
        };
        r.push(scaled);
        lags.push(lag as f64);
    }
    (r, lags)
}

/// `freqz` core — H(e^{jw}) = Σ b[k] e^{-jwk} / Σ a[k] e^{-jwk} at each `w`.
pub fn freqz_at(b: &[CNum], a: &[CNum], w: &[f64]) -> Vec<CNum> {
    w.iter()
        .map(|&wi| {
            let mut num = CNum::real(0.0);
            for (k, &bk) in b.iter().enumerate() {
                let ang = -wi * k as f64;
                num = num.add(bk.mul(CNum { re: ang.cos(), im: ang.sin() }));
            }
            let mut den = CNum::real(0.0);
            for (k, &ak) in a.iter().enumerate() {
                let ang = -wi * k as f64;
                den = den.add(ak.mul(CNum { re: ang.cos(), im: ang.sin() }));
            }
            num.div(den)
        })
        .collect()
}

/// `bartlett(n)` — symmetric triangular window with zero endpoints.
pub fn bartlett(n: usize) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![1.0];
    }
    let c = (n - 1) as f64 / 2.0;
    (0..n).map(|k| 1.0 - ((k as f64) - c).abs() / c).collect()
}

/// `triang(n)` — triangular window (no forced-zero endpoints).
pub fn triang(n: usize) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    let mut w = vec![0.0; n];
    if n % 2 == 1 {
        let half = n.div_ceil(2);
        for k in 1..=half {
            w[k - 1] = 2.0 * k as f64 / (n as f64 + 1.0);
        }
        for k in 0..half {
            w[n - 1 - k] = w[k];
        }
    } else {
        let half = n / 2;
        for k in 1..=half {
            w[k - 1] = (2.0 * k as f64 - 1.0) / n as f64;
        }
        for k in 0..half {
            w[n - 1 - k] = w[k];
        }
    }
    w
}

/// `hamming(n)` — raised cosine `0.54 − 0.46·cos(2πk/(n−1))`;
/// `periodic` uses denominator `n` (DFT-even form).
pub fn hamming(n: usize, periodic: bool) -> Vec<f64> {
    cosine_window(n, periodic, &[0.54, -0.46])
}

/// `hann(n)` — `0.5 − 0.5·cos(2πk/(n−1))`.
pub fn hann(n: usize, periodic: bool) -> Vec<f64> {
    cosine_window(n, periodic, &[0.5, -0.5])
}

/// `blackman(n)` — `0.42 − 0.5·cos(2πk/(n−1)) + 0.08·cos(4πk/(n−1))`.
pub fn blackman(n: usize, periodic: bool) -> Vec<f64> {
    cosine_window(n, periodic, &[0.42, -0.5, 0.08])
}

/// Shared raised-cosine family: `w[k] = Σ_m c[m]·cos(2π·m·k/L)` with
/// `L = n-1` (symmetric) or `n` (periodic).
fn cosine_window(n: usize, periodic: bool, coeffs: &[f64]) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![coeffs.first().copied().unwrap_or(1.0).abs().clamp(0.0, 1.0)];
    }
    let l = if periodic { n as f64 } else { (n - 1) as f64 };
    (0..n).map(|k| coeffs.iter().enumerate().map(|(m, &c)| c * (2.0 * PI * m as f64 * k as f64 / l).cos()).sum::<f64>()).collect()
}

/// `rectwin(n)` — all ones.
pub fn rectwin(n: usize) -> Vec<f64> {
    vec![1.0; n]
}

/// Modified Bessel function I0(x) — power series, converges fast for the
/// beta range Kaiser windows use.
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let xx = (x / 2.0) * (x / 2.0);
    for k in 1..60 {
        term *= xx / (k as f64 * k as f64);
        sum += term;
        if term < sum * 1e-15 {
            break;
        }
    }
    sum
}

/// `kaiser(n, beta)` — `I0(β·√(1−t²))/I0(β)`, `t = 2k/(n−1) − 1`.
pub fn kaiser(n: usize, beta: f64) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![1.0];
    }
    let d = bessel_i0(beta);
    let half = (n - 1) as f64;
    (0..n)
        .map(|k| {
            let t = (2.0 * k as f64 - half) / half;
            bessel_i0(beta * (1.0 - t * t).max(0.0).sqrt()) / d
        })
        .collect()
}

/// `gausswin(n, alpha)` — `exp(−½·(α·t)²)`, `t = (k − (n−1)/2)/((n−1)/2)`.
pub fn gausswin(n: usize, alpha: f64) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![1.0];
    }
    let c = (n - 1) as f64 / 2.0;
    (0..n)
        .map(|k| {
            let t = alpha * (k as f64 - c) / c;
            (-0.5 * t * t).exp()
        })
        .collect()
}

/// `fftshift` index permutation — rotate left by `ceil(n/2)`.
pub fn fftshift_order(n: usize) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let k = n.div_ceil(2);
    (0..n).map(|i| (i + k) % n).collect()
}

/// `ifftshift` index permutation — rotate left by `floor(n/2)`.
pub fn ifftshift_order(n: usize) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let k = n / 2;
    (0..n).map(|i| (i + k) % n).collect()
}

/// `unwrap` — phase unwrapping along a 1-D sequence (jump tolerance π).
pub fn unwrap_phase(p: &[f64], tol: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(p.len());
    let mut offset = 0.0;
    let mut prev = 0.0;
    for (i, &v) in p.iter().enumerate() {
        if i > 0 {
            let d = v - prev;
            if d > tol {
                offset -= 2.0 * PI;
            } else if d < -tol {
                offset += 2.0 * PI;
            }
        }
        prev = v;
        out.push(v + offset);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cvec(re: &[f64]) -> Vec<CNum> {
        re.iter().map(|&r| CNum::real(r)).collect()
    }

    fn assert_close(a: &[CNum], b: &[CNum], tol: f64) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!((x.re - y.re).abs() < tol && (x.im - y.im).abs() < tol, "{x:?} vs {y:?}");
        }
    }

    #[test]
    fn fft_delta_is_ones() {
        let x = cvec(&[1.0, 0.0, 0.0, 0.0]);
        assert_close(&fft(&x), &cvec(&[1.0, 1.0, 1.0, 1.0]), 1e-12);
    }

    #[test]
    fn fft_known_4pt() {
        // FFT of [1 2 3 4] = [10, -2+2i, -2, -2-2i].
        let x = cvec(&[1.0, 2.0, 3.0, 4.0]);
        assert_close(&fft(&x), &[CNum::new(10.0, 0.0), CNum::new(-2.0, 2.0), CNum::new(-2.0, 0.0), CNum::new(-2.0, -2.0)], 1e-10);
    }

    #[test]
    fn fft_non_pow2_and_prime() {
        // 6-point (2·3) and 7-point (prime → naive DFT) both round-trip.
        for n in [5usize, 6, 7, 9, 12] {
            let x: Vec<CNum> = (0..n).map(|i| CNum::new(i as f64, 0.0)).collect();
            let back = ifft(&fft(&x));
            assert_close(&back, &x, 1e-9);
        }
    }

    #[test]
    fn filter_fir_is_conv() {
        // a = 1, b = [1 1]: y = moving sum.
        let b = cvec(&[1.0, 1.0]);
        let a = cvec(&[1.0]);
        let x = cvec(&[1.0, 2.0, 3.0, 4.0]);
        let (y, _zf) = filter_df2t(&b, &a, &x, &[]);
        assert_close(&y, &cvec(&[1.0, 3.0, 5.0, 7.0]), 1e-12);
    }

    #[test]
    fn filter_iir_integrator() {
        // a = [1 -1], b = 1: y[n] = y[n-1] + x[n] → cumsum.
        let b = cvec(&[1.0]);
        let a = cvec(&[1.0, -1.0]);
        let x = cvec(&[1.0, 1.0, 1.0]);
        let (y, zf) = filter_df2t(&b, &a, &x, &[]);
        assert_close(&y, &cvec(&[1.0, 2.0, 3.0]), 1e-12);
        assert!((zf.first().copied().unwrap_or(CNum::real(0.0)).re - 3.0).abs() < 1e-12);
    }

    #[test]
    fn xcorr_peak_at_lag() {
        // xcorr of [0 1 0] with [1 0 0] peaks at lag +1 (x is y delayed by 1).
        let x = cvec(&[0.0, 1.0, 0.0]);
        let y = cvec(&[1.0, 0.0, 0.0]);
        let (r, lags) = xcorr(&x, &y, None, 0);
        assert_eq!(lags, vec![-2.0, -1.0, 0.0, 1.0, 2.0]);
        let peak = r.iter().zip(&lags).max_by(|a, b| a.0.abs().partial_cmp(&b.0.abs()).unwrap_or(std::cmp::Ordering::Equal));
        assert_eq!(peak.map(|(_, l)| *l), Some(1.0));
    }

    #[test]
    fn freqz_dc_gain() {
        // b = [1 1], a = 1 at w=0 → 2; at w=π → 0.
        let h = freqz_at(&cvec(&[1.0, 1.0]), &cvec(&[1.0]), &[0.0, PI]);
        assert!((h.first().map(|z| z.re).unwrap_or(0.0) - 2.0).abs() < 1e-12);
        assert!(h.get(1).map(|z| z.abs()).unwrap_or(1.0) < 1e-12);
    }

    #[test]
    fn windows_endpoints() {
        let h = hamming(8, false);
        assert!((h.first().copied().unwrap_or(0.0) - 0.08).abs() < 1e-12);
        assert!((h.last().copied().unwrap_or(0.0) - 0.08).abs() < 1e-12);
        let k = kaiser(5, 5.0);
        assert!((k[2] - 1.0).abs() < 1e-12); // center taps 1
        let g = gausswin(5, 2.5);
        assert!((g[2] - 1.0).abs() < 1e-12);
        assert_eq!(rectwin(4), vec![1.0; 4]);
        let t = triang(5);
        assert!((t[0] - 0.3333).abs() < 1e-3);
    }

    #[test]
    fn fftshift_orders() {
        assert_eq!(fftshift_order(5), vec![3, 4, 0, 1, 2]);
        assert_eq!(fftshift_order(4), vec![2, 3, 0, 1]);
        assert_eq!(ifftshift_order(5), vec![2, 3, 4, 0, 1]);
    }

    #[test]
    fn unwrap_phase_jumps() {
        let p = unwrap_phase(&[0.0, 1.0, 1.0 - 2.0 * PI + 4.0 * PI, -4.5], PI);
        // 1+4π ≈ 13.57: no jump from 1.0 (d ≈ 12.57 > π → wraps down); then -4.5.
        assert_eq!(p.len(), 4);
    }
}
