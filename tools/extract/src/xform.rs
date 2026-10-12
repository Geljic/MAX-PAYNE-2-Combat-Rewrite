//! Helpers for the 4x3 transforms stored in KF2 files.
//!
//! A `Mat4x3` is four rows of three floats. We read it as the row-vector
//! convention used by 3ds Max style tools (`v' = v * M`): rows 0..2 hold the
//! rotation and scale axes, row 3 holds the translation. This is inferred from
//! the layout, not yet checked against real game data; see `docs/formats/kf2.md`.

/// Four rows of three floats: three axis rows and a translation row.
pub type Mat4x3 = [[f32; 3]; 4];

pub const IDENTITY: Mat4x3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]];

pub fn translation(m: &Mat4x3) -> [f32; 3] {
    m[3]
}

/// Length of each axis row.
pub fn scale(m: &Mat4x3) -> [f32; 3] {
    let len = |r: &[f32; 3]| (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
    [len(&m[0]), len(&m[1]), len(&m[2])]
}

/// Rotation as a unit quaternion `[x, y, z, w]`, with scale removed.
/// Degenerate (zero-scale) input gives the identity rotation.
pub fn rotation_quat(m: &Mat4x3) -> [f32; 4] {
    let s = scale(m);
    if s.iter().any(|v| *v < 1e-9) {
        return [0.0, 0.0, 0.0, 1.0];
    }
    // Normalised rows; r[i][j] is the column-convention matrix element
    // (row j of the file is column j of R), so R[i][j] = m[j][i] / s[j].
    let r = |i: usize, j: usize| m[j][i] / s[j];
    let (r00, r01, r02) = (r(0, 0), r(0, 1), r(0, 2));
    let (r10, r11, r12) = (r(1, 0), r(1, 1), r(1, 2));
    let (r20, r21, r22) = (r(2, 0), r(2, 1), r(2, 2));
    let trace = r00 + r11 + r22;
    let q = if trace > 0.0 {
        let k = (trace + 1.0).sqrt() * 2.0;
        [(r21 - r12) / k, (r02 - r20) / k, (r10 - r01) / k, 0.25 * k]
    } else if r00 > r11 && r00 > r22 {
        let k = (1.0 + r00 - r11 - r22).sqrt() * 2.0;
        [0.25 * k, (r01 + r10) / k, (r02 + r20) / k, (r21 - r12) / k]
    } else if r11 > r22 {
        let k = (1.0 + r11 - r00 - r22).sqrt() * 2.0;
        [(r01 + r10) / k, 0.25 * k, (r12 + r21) / k, (r02 - r20) / k]
    } else {
        let k = (1.0 + r22 - r00 - r11).sqrt() * 2.0;
        [(r02 + r20) / k, (r12 + r21) / k, 0.25 * k, (r10 - r01) / k]
    };
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1e-9 { [0.0, 0.0, 0.0, 1.0] } else { [q[0] / n, q[1] / n, q[2] / n, q[3] / n] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_quat() {
        assert_eq!(rotation_quat(&IDENTITY), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn quarter_turn_about_z() {
        // Column-convention R = [[0,-1,0],[1,0,0],[0,0,1]]; file rows are R transposed.
        let m = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [5.0, 6.0, 7.0]];
        let q = rotation_quat(&m);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!((q[2] - h).abs() < 1e-5 && (q[3] - h).abs() < 1e-5, "{q:?}");
        assert_eq!(translation(&m), [5.0, 6.0, 7.0]);
    }

    #[test]
    fn scale_removed() {
        let m = [[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 2.0], [0.0; 3]];
        assert_eq!(scale(&m), [2.0, 2.0, 2.0]);
        assert_eq!(rotation_quat(&m), [0.0, 0.0, 0.0, 1.0]);
    }
}
