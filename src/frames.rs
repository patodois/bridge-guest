//! From the two read-back images to what the pixel file holds.

/// Bytes per row of a buffer a texture is copied into: wgpu pads rows to this multiple
const ROW_ALIGN: usize = 256;

fn padded_row(width: usize) -> usize {
    (width * 4).div_ceil(ROW_ALIGN) * ROW_ALIGN
}

/// Straight-alpha RGBA8 rows and linear depth in metres, `width * height` each, top row first.
///
/// `colour` and `depth` are the raw read-back bytes of the colour image (`Rgba8UnormSrgb`) and of
/// the depth image (`R32Float`, `-1` = nothing drawn), rows padded to 256 bytes. A pixel is empty
/// (alpha 0, depth `+inf`) unless it has alpha, a depth that is finite and positive, and (when the
/// host has a far plane) lies within it. The contract wants colour and depth to agree about which
/// pixels are empty.
pub fn assemble(
    colour: &[u8],
    depth: &[u8],
    width: usize,
    height: usize,
    far: f32,
) -> Option<(Vec<u8>, Vec<f32>)> {
    let row = padded_row(width);
    if colour.len() < row * height || depth.len() < row * height || width == 0 || height == 0 {
        return None;
    }
    let mut rgba = vec![0u8; width * height * 4];
    let mut metres = vec![f32::INFINITY; width * height];
    for y in 0..height {
        let c = &colour[y * row..][..width * 4];
        let d = &depth[y * row..][..width * 4];
        for x in 0..width {
            let t = f32::from_le_bytes([d[x * 4], d[x * 4 + 1], d[x * 4 + 2], d[x * 4 + 3]]);
            let px = &c[x * 4..x * 4 + 4];
            if px[3] != 0 && t.is_finite() && t > 0.0 && t <= far {
                let i = y * width + x;
                rgba[i * 4..i * 4 + 4].copy_from_slice(px);
                metres[i] = t;
            }
        }
    }
    Some((rgba, metres))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn padded(width: usize, height: usize, px: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        let row = padded_row(width);
        let mut v = vec![0xEE; row * height];
        for y in 0..height {
            for x in 0..width {
                v[y * row + x * 4..][..4].copy_from_slice(&px(x, y));
            }
        }
        v
    }

    #[test]
    fn rows_are_unpadded_and_empty_pixels_agree() {
        // 3 x 2; (1,0) has colour but no depth, (2,1) has depth but no alpha
        let colour = padded(3, 2, |x, y| {
            if (x, y) == (2, 1) {
                [9, 9, 9, 0]
            } else {
                [10 * x as u8, 20, 30, 255]
            }
        });
        let depth = padded(3, 2, |x, y| {
            let t: f32 = if (x, y) == (1, 0) {
                -1.0
            } else {
                2.5 + x as f32
            };
            t.to_le_bytes()
        });
        let (rgba, m) = assemble(&colour, &depth, 3, 2, f32::INFINITY).unwrap();
        assert_eq!(rgba.len(), 3 * 2 * 4);
        assert_eq!(&rgba[0..4], &[0, 20, 30, 255]);
        assert_eq!(&rgba[4..8], &[0, 0, 0, 0], "no depth, so empty");
        assert_eq!(m[0], 2.5);
        assert_eq!(m[1], f32::INFINITY);
        assert_eq!(m[3], 2.5);
        assert_eq!(&rgba[20..24], &[0, 0, 0, 0], "no alpha, so empty");
        assert_eq!(m[5], f32::INFINITY);
    }

    #[test]
    fn the_hosts_far_plane_clips() {
        let colour = padded(2, 1, |_, _| [1, 2, 3, 255]);
        let depth = padded(2, 1, |x, _| {
            (if x == 0 { 99.0f32 } else { 101.0 }).to_le_bytes()
        });
        let (rgba, m) = assemble(&colour, &depth, 2, 1, 100.0).unwrap();
        assert_eq!(m, vec![99.0, f32::INFINITY]);
        assert_eq!(rgba[7], 0);
    }

    #[test]
    fn a_short_buffer_is_refused() {
        assert!(assemble(&[0; 10], &[0; 10], 4, 4, f32::INFINITY).is_none());
    }

    #[test]
    fn wide_rows_pad_to_256() {
        assert_eq!(padded_row(640), 2560);
        assert_eq!(padded_row(576), 2304);
        assert_eq!(padded_row(100), 512);
    }
}
