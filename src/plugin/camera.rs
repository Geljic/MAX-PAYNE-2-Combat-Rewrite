//! MP2's third-person camera: behind and slightly right of Max, looking along the aim, pulled in
//! by walls. The view direction passes through the crosshair at the screen centre; that ray is what
//! the sim aims along.

use bevy::prelude::*;

use super::{PayneInput, PayneSim, PayneWorld, bv, pv};
use crate::math::Vec3 as PVec;

#[derive(Component)]
pub struct PayneCamera;

#[derive(Resource)]
pub struct CameraRig {
    /// Distance behind the pivot (m). ESTIMATE: MP2's camera distance lives in its camera config.
    pub dist: f32,
    /// Sideways offset to the right shoulder (m). ESTIMATE.
    pub shoulder: f32,
    /// Pivot height above the chest (m). ESTIMATE.
    pub lift: f32,
    /// Vertical field of view (rad).
    pub fov: f32,
    /// Extra yaw around Max (rad), for looking at him from the front or side. While non-zero the
    /// crosshair ray is not sent to the sim.
    pub orbit: f32,
    /// Look at target N (spawn order) from in front of it instead of following Max; for scripted
    /// close-ups of enemies. The crosshair ray is not sent to the sim meanwhile.
    pub focus: Option<usize>,
    /// Distance actually used after wall pull-in, and the smoothed pivot height.
    cur_len: f32,
    pivot_h: f32,
    pos: Vec3,
    dir: Vec3,
    valid: bool,
}

impl Default for CameraRig {
    fn default() -> Self {
        CameraRig {
            dist: 2.7,
            shoulder: 0.55,
            lift: 0.25,
            fov: 65f32.to_radians(),
            orbit: 0.0,
            focus: None,
            cur_len: 2.7,
            pivot_h: 1.8,
            pos: Vec3::ZERO,
            dir: Vec3::NEG_Z,
            valid: false,
        }
    }
}

impl CameraRig {
    /// The crosshair ray (origin, direction) from the last camera update.
    pub fn ray(&self) -> Option<(PVec, PVec)> {
        (self.valid && self.orbit == 0.0 && self.focus.is_none())
            .then(|| (pv(self.pos), pv(self.dir)))
    }
    pub fn position(&self) -> Vec3 {
        self.pos
    }
}

pub fn spawn(mut commands: Commands, rig: Res<CameraRig>) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: rig.fov,
            ..default()
        }),
        PayneCamera,
        Transform::from_xyz(0.0, 2.5, 3.0),
    ));
}

pub fn follow(
    time: Res<Time<Real>>,
    sim: Res<PayneSim>,
    input: Res<PayneInput>,
    world: Res<PayneWorld>,
    mut rig: ResMut<CameraRig>,
    mut cam: Query<&mut Transform, With<PayneCamera>>,
) {
    let Ok(mut t) = cam.single_mut() else { return };
    let (p, tu) = (&sim.0.player, &sim.0.tuning);
    let dt = time.delta_secs();
    let yaw = p.yaw + input.look[0];
    let pitch = (p.pitch + input.look[1]).clamp(tu.pitch_min, tu.pitch_max);

    let want_h = p.chest(tu).y - p.pos.y + rig.lift;
    let k = 1.0 - (-10.0 * dt).exp();
    rig.pivot_h += (want_h - rig.pivot_h) * k;
    let pivot = bv(p.pos) + Vec3::Y * rig.pivot_h;

    if let Some(i) = rig.focus
        && let Some(e) = sim.0.targets.get(i)
    {
        // Standing: four metres in front of the enemy and two to its left. Moving: 4.5 m to the
        // side of its path, so the run cycle shows in profile. Crouched: 4.5 m to its right. Chest height, looking at it.
        let at = bv(e.pos) + Vec3::Y * (1.2 * e.stance);
        let v = e.ai.as_ref().map_or(Vec3::ZERO, |ai| bv(ai.vel));
        let v = Vec3::new(v.x, 0.0, v.z);
        let off = if v.length() > 0.5 {
            let d = v.normalize();
            Vec3::new(-d.z, 0.0, d.x) * 4.5
        } else if e.ai.as_ref().is_some_and(|ai| ai.crouched) {
            // Crouched behind cover that is in front of it: from its side.
            bv(crate::math::yaw_right(e.yaw)) * 4.5
        } else {
            bv(crate::math::yaw_dir(e.yaw)) * 4.0 - bv(crate::math::yaw_right(e.yaw)) * 2.0
        };
        let eye = bv(e.pos) + off + Vec3::Y * (1.7 * e.stance.max(0.6));
        *t = Transform::from_translation(eye).looking_at(at, Vec3::Y);
        rig.pos = eye;
        rig.dir = (at - eye).normalize_or(Vec3::NEG_Z);
        rig.valid = true;
        return;
    }
    let orbit = rig.orbit;
    let shoulder = if orbit == 0.0 { rig.shoulder } else { 0.0 };
    let turn = Quat::from_rotation_y(yaw + orbit);
    let rot = turn * Quat::from_rotation_x(pitch);
    let want = pivot + turn * Vec3::X * shoulder + rot * Vec3::Z * rig.dist;
    let to = want - pivot;
    let len = to.length();
    let dir = to / len.max(1e-4);
    // Pull in against walls; ease back out.
    let allowed = world
        .0
        .raycast(pv(pivot), pv(dir), len + 0.25)
        .map_or(len, |h| (h.dist - 0.25).clamp(0.3, len));
    let k = 1.0 - (-6.0 * dt).exp();
    rig.cur_len = if allowed < rig.cur_len {
        allowed
    } else {
        rig.cur_len + (allowed - rig.cur_len) * k
    };
    let pos = pivot + dir * rig.cur_len;
    t.translation = pos;
    t.rotation = rot;
    rig.pos = pos;
    rig.dir = rot * Vec3::NEG_Z;
    rig.valid = true;
}
