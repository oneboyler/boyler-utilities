//! A plain radix-2 complex FFT (no dependency). Used once per noise, to make its loop.

/// In place, length a power of two. `inverse` flips the sign of the exponent (no 1/N scaling either way).
pub fn fft(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len();
    assert!(n.is_power_of_two() && im.len() == n, "fft: length must be a power of two");
    if n < 2 {
        return;
    }
    // bit reversal
    let shift = usize::BITS - n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> shift;
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    // twiddles for the largest stage, made in f64 (the smaller stages read every 2nd, 4th ... entry)
    let half = n / 2;
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut wr = Vec::with_capacity(half);
    let mut wi = Vec::with_capacity(half);
    for k in 0..half {
        let a = sign * 2.0 * std::f64::consts::PI * k as f64 / n as f64;
        wr.push(a.cos() as f32);
        wi.push(a.sin() as f32);
    }
    let mut len = 2;
    while len <= n {
        let h = len / 2;
        let step = n / len;
        for start in (0..n).step_by(len) {
            for j in 0..h {
                let (c, s) = (wr[j * step], wi[j * step]);
                let (a, b) = (start + j, start + j + h);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_plain_dft() {
        let n = 64;
        let x: Vec<f32> = (0..n).map(|i| ((i * 7 + 3) % 11) as f32 - 5.0).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0f32; n]);
        fft(&mut re, &mut im, false);
        for k in 0..n {
            let (mut sr, mut si) = (0.0f64, 0.0f64);
            for (t, v) in x.iter().enumerate() {
                let a = -2.0 * std::f64::consts::PI * (k * t) as f64 / n as f64;
                sr += f64::from(*v) * a.cos();
                si += f64::from(*v) * a.sin();
            }
            assert!((re[k] as f64 - sr).abs() < 1e-3 && (im[k] as f64 - si).abs() < 1e-3, "bin {k}");
        }
    }

    #[test]
    fn forward_then_inverse_gives_n_times_the_signal() {
        let n = 1024;
        let x: Vec<f32> = (0..n).map(|i| (i as f32 * 0.37).sin() + 0.25 * (i as f32 * 1.91).cos()).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0f32; n]);
        fft(&mut re, &mut im, false);
        fft(&mut re, &mut im, true);
        for i in 0..n {
            assert!((re[i] / n as f32 - x[i]).abs() < 1e-4 && (im[i] / n as f32).abs() < 1e-4);
        }
    }
}
