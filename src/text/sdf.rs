//! Signed distance field generation for SDF font atlases. Used at bake time only.
//!
//! Input is a glyph coverage mask rendered at `supersample` times the base size. Output is a
//! single-channel field at the base size, padded by `spread` texels on each side:
//!
//! `value = clamp(0.5 - d / (2 * spread), 0, 1)`, stored as `u8`
//!
//! where `d` is the signed distance to the glyph edge in base-size pixels (positive outside).
//! So `128` is the edge, `255` is `spread` px or more inside, and `0` is `spread` px or more outside.

/// Supersample factor used when baking SDF glyphs.
pub(crate) const SUPERSAMPLE: u32 = 4;

/// Stand-in for infinity. Kept finite so `INF - INF` is `0`, not NaN, in the 1D transform.
const INF: f64 = 1e20;

/// A glyph distance field plus its placement at the base size.
#[derive(Debug)]
pub(crate) struct SdfGlyph {
    /// One byte per texel, row-major.
    pub(crate) values: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Bearing of the padded field, in base-size pixels (same convention as the mask).
    pub(crate) left: i32,
    pub(crate) top: i32,
}

/// Builds a distance field from a coverage mask.
///
/// `mask` is `mask_width * mask_height` coverage bytes, rendered at `supersample` times the base
/// size. `left` / `top` are its bearing in those supersampled pixels: `left` is the x offset of
/// the first column from the pen, and `top` is the height of the first row above the baseline
/// (y up). `spread` is in base-size pixels and must be at least 1.
pub(crate) fn glyph_sdf(
    mask: &[u8],
    mask_width: u32,
    mask_height: u32,
    left: i32,
    top: i32,
    supersample: u32,
    spread: u32,
) -> SdfGlyph {
    assert!(supersample >= 1 && spread >= 1);
    assert_eq!(mask.len(), mask_width as usize * mask_height as usize);
    let k = supersample as i32;
    let s = spread as i32;
    let (mask_w, mask_h) = (mask_width as i32, mask_height as i32);

    // Output texel bounds, snapped outward to whole base-size pixels, plus the padding.
    let out_left = left.div_euclid(k) - s;
    let out_right = ceil_div(left + mask_w, k) + s;
    let out_top = ceil_div(top, k) + s;
    let out_bottom = (top - mask_h).div_euclid(k) - s;
    let out_w = (out_right - out_left) as usize;
    let out_h = (out_top - out_bottom) as usize;

    // Supersampled grid covering exactly the output texels.
    let grid_w = out_w * k as usize;
    let grid_h = out_h * k as usize;
    let offset_x = (left - out_left * k) as usize;
    let offset_y = (out_top * k - top) as usize;

    // Squared distance to the nearest inside / outside point. Partly covered pixels are
    // seeded with a sub-pixel estimate of the edge distance (as in Mapbox tiny-sdf), which
    // keeps the edge position accurate even though the transform itself is pixel-based.
    let mut to_inside = vec![INF; grid_w * grid_h];
    let mut to_outside = vec![0.0; grid_w * grid_h];
    for row in 0..mask_h as usize {
        for col in 0..mask_w as usize {
            let coverage = mask[row * mask_w as usize + col];
            if coverage == 0 {
                continue;
            }
            let index = (offset_y + row) * grid_w + offset_x + col;
            if coverage == 255 {
                to_inside[index] = 0.0;
                to_outside[index] = INF;
            } else {
                let d = 0.5 - f64::from(coverage) / 255.0;
                to_inside[index] = if d > 0.0 { d * d } else { 0.0 };
                to_outside[index] = if d < 0.0 { d * d } else { 0.0 };
            }
        }
    }
    edt_2d(&mut to_inside, grid_w, grid_h);
    edt_2d(&mut to_outside, grid_w, grid_h);

    // Sample the distance at each output texel center. For even `k` the center falls between
    // 2x2 supersampled pixels, so average those (1 pixel for odd `k`). Averaging the whole
    // k x k block would flatten the distance peak inside thin strokes and break hairlines.
    let (first, taps) = if k % 2 == 0 {
        (k as usize / 2 - 1, 2)
    } else {
        (k as usize / 2, 1)
    };
    let scale = 1.0 / (f64::from(k) * 2.0 * f64::from(s) * (taps * taps) as f64);
    let mut values = vec![0u8; out_w * out_h];
    for out_y in 0..out_h {
        for out_x in 0..out_w {
            let mut sum = 0.0;
            for sub_y in 0..taps {
                let row = (out_y * k as usize + first + sub_y) * grid_w + out_x * k as usize + first;
                for index in row..row + taps {
                    sum += to_inside[index].sqrt() - to_outside[index].sqrt();
                }
            }
            // `sum` is in supersampled pixels; `scale` averages, converts and normalizes it.
            let value = 0.5 - sum * scale;
            values[out_y * out_w + out_x] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }

    SdfGlyph {
        values,
        width: out_w as u32,
        height: out_h as u32,
        left: out_left,
        top: out_top,
    }
}

fn ceil_div(a: i32, b: i32) -> i32 {
    -(-a).div_euclid(b)
}

/// In-place squared Euclidean distance transform (Felzenszwalb-Huttenlocher), rows then columns.
fn edt_2d(grid: &mut [f64], width: usize, height: usize) {
    let longest = width.max(height);
    let mut f = vec![0.0; longest];
    let mut v = vec![0usize; longest];
    let mut z = vec![0.0; longest + 1];
    for x in 0..width {
        edt_1d(grid, x, width, height, &mut f, &mut v, &mut z);
    }
    for y in 0..height {
        edt_1d(grid, y * width, 1, width, &mut f, &mut v, &mut z);
    }
}

/// 1D transform of `length` cells starting at `offset`, `stride` apart: lower envelope of
/// the parabolas `(q - p)^2 + f(p)`.
fn edt_1d(
    grid: &mut [f64],
    offset: usize,
    stride: usize,
    length: usize,
    f: &mut [f64],
    v: &mut [usize],
    z: &mut [f64],
) {
    if length == 0 {
        return;
    }
    v[0] = 0;
    z[0] = -INF;
    z[1] = INF;
    f[0] = grid[offset];
    let mut k = 0usize;
    for q in 1..length {
        f[q] = grid[offset + q * stride];
        let qf = q as f64;
        loop {
            let r = v[k];
            let rf = r as f64;
            let s = (f[q] - f[r] + qf * qf - rf * rf) / (qf - rf) / 2.0;
            if s <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            if s > z[k] {
                k += 1;
            }
            // When `k == 0` and `s <= z[0]` (only possible with `z[0] = -INF`), the new
            // parabola replaces the first one.
            v[k] = q;
            z[k] = s;
            z[k + 1] = INF;
            break;
        }
    }
    let mut k = 0usize;
    for q in 0..length {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let r = v[k];
        let d = q as f64 - r as f64;
        grid[offset + q * stride] = f[r] + d * d;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes a stored value back to a signed distance in base-size pixels.
    fn decode(value: u8, spread: u32) -> f64 {
        (0.5 - f64::from(value) / 255.0) * 2.0 * f64::from(spread)
    }

    fn at(glyph: &SdfGlyph, x: u32, y: u32) -> u8 {
        glyph.values[(y * glyph.width + x) as usize]
    }

    #[test]
    fn one_dimensional_transform_matches_brute_force() {
        let seeds = [INF, INF, 0.0, INF, INF, INF, 2.0, INF, 0.0, INF];
        let mut grid = seeds.to_vec();
        let n = grid.len();
        let (mut f, mut v, mut z) = (vec![0.0; n], vec![0; n], vec![0.0; n + 1]);
        edt_1d(&mut grid, 0, 1, n, &mut f, &mut v, &mut z);
        for q in 0..n {
            let expected = (0..n)
                .map(|p| (q as f64 - p as f64).powi(2) + seeds[p])
                .fold(INF, f64::min);
            assert_eq!(grid[q], expected, "cell {q}");
        }
    }

    #[test]
    fn two_dimensional_transform_matches_brute_force() {
        let (w, h) = (7, 5);
        let points = [(1, 1), (5, 3)];
        let mut grid = vec![INF; w * h];
        for &(x, y) in &points {
            grid[y * w + x] = 0.0;
        }
        edt_2d(&mut grid, w, h);
        for y in 0..h {
            for x in 0..w {
                let expected = points
                    .iter()
                    .map(|&(px, py)| {
                        (x as f64 - px as f64).powi(2) + (y as f64 - py as f64).powi(2)
                    })
                    .fold(INF, f64::min);
                assert_eq!(grid[y * w + x], expected, "({x}, {y})");
            }
        }
    }

    #[test]
    fn placement_adds_padding_and_snaps_to_whole_pixels() {
        // 10x6 supersampled mask, bearing (5, 9) at k = 4: covers x 5..15 and y 3..9 (y up).
        let mask = vec![255u8; 10 * 6];
        let glyph = glyph_sdf(&mask, 10, 6, 5, 9, 4, 2);
        // x: floor(5 / 4) - 2 = -1 to ceil(15 / 4) + 2 = 6. y: floor(3 / 4) - 2 = -2 to
        // ceil(9 / 4) + 2 = 5.
        assert_eq!((glyph.left, glyph.top), (-1, 5));
        assert_eq!((glyph.width, glyph.height), (7, 7));
        assert_eq!(glyph.values.len(), 49);
    }

    #[test]
    fn square_has_correct_sign_and_distances() {
        // 8x8 base-size square, supersampled 4x, spread 4.
        let (k, spread) = (4, 4);
        let side = 8 * k;
        let mask = vec![255u8; (side * side) as usize];
        let glyph = glyph_sdf(&mask, side, side, 0, side as i32, k, spread);
        assert_eq!((glyph.width, glyph.height), (16, 16));
        assert_eq!((glyph.left, glyph.top), (-4, 12));

        // The square covers texels 4..12 on both axes. Texel centers sit at half pixels, so the
        // texels on each side of the left edge are 0.5 px from it. Fully covered or empty
        // supersampled pixels measure center to center, which overstates distances by up to
        // half a supersampled pixel (0.125 px here), equally on both sides of the edge.
        let mid = 8;
        let tolerance = 0.5 / k as f64 + 0.01;
        assert!((decode(at(&glyph, 4, mid), spread) - -0.5).abs() < tolerance);
        assert!((decode(at(&glyph, 3, mid), spread) - 0.5).abs() < tolerance);
        assert!((decode(at(&glyph, 1, mid), spread) - 2.5).abs() < tolerance);
        // The edge sits exactly halfway between the two texels.
        assert_eq!(u16::from(at(&glyph, 4, mid)) + u16::from(at(&glyph, 3, mid)), 255);
        // Texel 8 is 3.5 px from the nearest (right) edge. The far corner is outside the
        // spread, so it saturates.
        assert!((decode(at(&glyph, mid, mid), spread) - -3.5).abs() < tolerance);
        assert_eq!(at(&glyph, 0, 0), 0);
        // Symmetric across the square's center.
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(at(&glyph, x, y), at(&glyph, 15 - x, y));
                assert_eq!(at(&glyph, x, y), at(&glyph, x, 15 - y));
            }
        }
    }

    #[test]
    fn circle_distances_match_analytic_values() {
        // Circle of radius 6 base px, supersampled 4x with box-filtered coverage.
        let (k, spread) = (4u32, 4u32);
        let radius = 6.0 * k as f64;
        let size = (2.0 * radius) as u32;
        let center = radius;
        let sub = 8;
        let mut mask = vec![0u8; (size * size) as usize];
        for y in 0..size {
            for x in 0..size {
                let mut hits = 0;
                for sy in 0..sub {
                    for sx in 0..sub {
                        let px = x as f64 + (sx as f64 + 0.5) / sub as f64 - center;
                        let py = y as f64 + (sy as f64 + 0.5) / sub as f64 - center;
                        if px * px + py * py <= radius * radius {
                            hits += 1;
                        }
                    }
                }
                mask[(y * size + x) as usize] = (hits * 255 / (sub * sub)) as u8;
            }
        }
        let glyph = glyph_sdf(&mask, size, size, 0, size as i32, k, spread);

        // Field texel centers relative to the circle center, in base px.
        let base_center = center / k as f64 + spread as f64;
        let mut checked = 0;
        for y in 0..glyph.height {
            for x in 0..glyph.width {
                let dx = x as f64 + 0.5 - base_center;
                let dy = y as f64 + 0.5 - base_center;
                let expected = (dx * dx + dy * dy).sqrt() - radius / k as f64;
                if expected.abs() < spread as f64 - 0.5 {
                    let got = decode(at(&glyph, x, y), spread);
                    assert!(
                        (got - expected).abs() < 0.25,
                        "({x}, {y}): got {got}, expected {expected}"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 100);
    }

    #[test]
    fn empty_mask_is_fully_outside() {
        let glyph = glyph_sdf(&[0; 16], 4, 4, 0, 4, 4, 2);
        assert!(glyph.values.iter().all(|&v| v == 0));
    }
}
