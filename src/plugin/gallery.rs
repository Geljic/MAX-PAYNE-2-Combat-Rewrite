//! Shooting gallery for the range sandbox: bullet holes on any level geometry, accuracy stats per
//! paper board, a HUD panel and an inspect camera.
//!
//! The boards themselves are plain boxes in the [`crate::world::gallery`] lane so bullets stop on
//! them; [`spawn_gallery`] draws the paper, rings and labels. Holes come from `Event::Hit` with
//! `target: None`, so walls, crates and the floor get them too. Keys: Backspace or G clears holes
//! and stats, V toggles the inspect camera on the board last hit.

use bevy::prelude::*;
use std::collections::VecDeque;

use super::{PayneEvent, PayneSim, bv, camera};
use crate::world::gallery as lane;

/// Most holes kept; the oldest is removed first.
const MAX_HOLES: usize = 2000;
/// Gap between a surface and its decal (m), so decals never z-fight.
const HOLE_LIFT: f32 = 0.006;

/// Per-board numbers.
#[derive(Clone, Debug, Default)]
pub struct BoardStats {
    pub hits: u32,
    pub score: u32,
    /// Most recent holes as [right, up] metres from the bullseye, newest last, at most `GROUP_N`.
    pub recent: VecDeque<[f32; 2]>,
}

impl BoardStats {
    /// Group size of the recent holes (cm).
    pub fn group_cm(&self) -> f32 {
        let p: Vec<[f32; 2]> = self.recent.iter().copied().collect();
        lane::group_size(&p) * 100.0
    }
    /// Mean [right, up] offset of the recent holes (cm).
    pub fn offset_cm(&self) -> [f32; 2] {
        let p: Vec<[f32; 2]> = self.recent.iter().copied().collect();
        let m = lane::mean_offset(&p);
        [m[0] * 100.0, m[1] * 100.0]
    }
}

/// What the gallery measured since the last reset, for the HUD and for scripts and tests.
#[derive(Resource, Clone, Debug)]
pub struct GalleryStats {
    /// Bullets fired (a shotgun blast counts every pellet).
    pub shots: u32,
    /// Bullets that landed on a board.
    pub hits: u32,
    pub boards: Vec<BoardStats>,
    /// Board hit last: (index, ring score).
    pub last: Option<(usize, u32)>,
}

impl Default for GalleryStats {
    fn default() -> Self {
        GalleryStats {
            shots: 0,
            hits: 0,
            boards: vec![BoardStats::default(); lane::DISTS.len()],
            last: None,
        }
    }
}

impl GalleryStats {
    pub fn hit_percent(&self) -> f32 {
        if self.shots == 0 { 0.0 } else { self.hits as f32 * 100.0 / self.shots as f32 }
    }
    /// Record a bullet that struck board `i` at `off` ([right, up] m from the bullseye).
    pub fn record(&mut self, i: usize, off: [f32; 2]) -> u32 {
        let score = lane::ring_score(off[0].hypot(off[1]));
        let b = &mut self.boards[i];
        b.hits += 1;
        b.score += score;
        b.recent.push_back(off);
        while b.recent.len() > lane::GROUP_N {
            b.recent.pop_front();
        }
        self.hits += 1;
        self.last = Some((i, score));
        score
    }
}

/// Inspect camera state. Scripts set it with `inspect=N|last|off`.
#[derive(Resource, Default)]
pub struct GalleryView {
    pub inspect: bool,
    /// Board to look at; none means the one hit last (or the 10 m board before any hit).
    pub board: Option<usize>,
}

#[derive(Resource, Default)]
struct Holes(VecDeque<Entity>);

#[derive(Resource)]
struct HoleAssets {
    disc: Handle<Mesh>,
    hole: Handle<StandardMaterial>,
    rim: Handle<StandardMaterial>,
}

#[derive(Component)]
struct HudPanel;

pub fn build(app: &mut App, hud: bool) {
    app.init_resource::<GalleryStats>()
        .init_resource::<GalleryView>()
        .init_resource::<Holes>()
        .add_systems(Startup, setup_assets)
        .add_systems(Update, (record_events, controls).chain())
        .add_systems(
            PostUpdate,
            inspect_camera
                .after(camera::follow)
                .before(TransformSystems::Propagate),
        );
    if hud {
        app.add_systems(Startup, spawn_hud).add_systems(Update, update_hud);
    }
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let unlit = |c: Color| StandardMaterial {
        base_color: c,
        unlit: true,
        ..default()
    };
    commands.insert_resource(HoleAssets {
        disc: meshes.add(Circle::new(1.0)),
        hole: mats.add(unlit(Color::srgb(0.02, 0.02, 0.02))),
        rim: mats.add(unlit(Color::srgb(0.78, 0.76, 0.7))),
    });
}

/// Hits become holes, shots and board hits become stats.
fn record_events(
    mut commands: Commands,
    mut events: MessageReader<PayneEvent>,
    assets: Option<Res<HoleAssets>>,
    sim: Res<PayneSim>,
    mut stats: ResMut<GalleryStats>,
    mut holes: ResMut<Holes>,
) {
    use crate::event::Event::*;
    let Some(assets) = assets else { return };
    for PayneEvent(ev) in events.read() {
        match ev {
            Shot { weapon, .. } => {
                stats.shots += sim.0.weapons.get(*weapon).map_or(1, |w| w.pellets.max(1));
            }
            Hit { target: None, point, normal, .. } => {
                if let Some((i, off)) = lane::board_hit(*point, *normal) {
                    stats.record(i, off);
                }
                let n = bv(*normal).normalize_or_zero();
                if n == Vec3::ZERO {
                    continue;
                }
                let rot = Quat::from_rotation_arc(Vec3::Z, n);
                // A dark hole on a lighter rim; the rim sits a hair behind the hole.
                let parent = commands
                    .spawn((
                        Transform::from_translation(bv(*point) + n * HOLE_LIFT).with_rotation(rot),
                        Visibility::default(),
                    ))
                    .id();
                for (mat, r, z) in [(&assets.rim, 0.022, -0.001), (&assets.hole, 0.012, 0.0)] {
                    commands.spawn((
                        Mesh3d(assets.disc.clone()),
                        MeshMaterial3d(mat.clone()),
                        Transform::from_xyz(0.0, 0.0, z).with_scale(Vec3::splat(r)),
                        ChildOf(parent),
                    ));
                }
                holes.0.push_back(parent);
                while holes.0.len() > MAX_HOLES {
                    if let Some(old) = holes.0.pop_front() {
                        commands.entity(old).despawn();
                    }
                }
            }
            _ => {}
        }
    }
}

fn controls(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut stats: ResMut<GalleryStats>,
    mut holes: ResMut<Holes>,
    mut view: ResMut<GalleryView>,
) {
    if keys.just_pressed(KeyCode::Backspace) || keys.just_pressed(KeyCode::KeyG) {
        for e in holes.0.drain(..) {
            commands.entity(e).despawn();
        }
        *stats = GalleryStats::default();
    }
    if keys.just_pressed(KeyCode::KeyV) {
        view.inspect = !view.inspect;
        view.board = None;
    }
}

/// Eased camera in front of the chosen board while inspecting. Runs after the normal follow
/// camera, which keeps tracking Max underneath so the aim ray is unchanged.
fn inspect_camera(
    time: Res<Time<Real>>,
    view: Res<GalleryView>,
    stats: Res<GalleryStats>,
    mut cam: Query<&mut Transform, With<camera::PayneCamera>>,
    mut cur: Local<Option<Transform>>,
) {
    let Ok(mut t) = cam.single_mut() else { return };
    if !view.inspect {
        *cur = None;
        return;
    }
    let i = view.board.or(stats.last.map(|l| l.0)).unwrap_or(1);
    let c = bv(lane::centre(i.min(lane::DISTS.len() - 1)));
    let look = c + Vec3::Y * 0.2;
    let goal = Transform::from_translation(look + Vec3::Z * 2.6).looking_at(look, Vec3::Y);
    let k = 1.0 - (-4.0 * time.delta_secs()).exp();
    let from = cur.unwrap_or(*t);
    let next = Transform {
        translation: from.translation.lerp(goal.translation, k),
        rotation: from.rotation.slerp(goal.rotation, k),
        scale: Vec3::ONE,
    };
    *cur = Some(next);
    *t = next;
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.9)),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        HudPanel,
        Node {
            position_type: PositionType::Absolute,
            right: px(14),
            top: px(10),
            padding: UiRect::all(px(8)),
            ..default()
        },
    ));
}

fn side(v: f32, pos: &str, neg: &str) -> String {
    format!("{:.1} {}", v.abs(), if v >= 0.0 { pos } else { neg })
}

fn update_hud(sim: Res<PayneSim>, stats: Res<GalleryStats>, mut panel: Query<&mut Text, With<HudPanel>>) {
    let Ok(mut t) = panel.single_mut() else { return };
    let weapon = &sim.0.weapons[sim.0.player.weapon().def].name;
    let mut s = format!(
        "GALLERY  {weapon}\nshots {}   hits {}   {:.0}%\n",
        stats.shots,
        stats.hits,
        stats.hit_percent()
    );
    match stats.last {
        Some((i, score)) => {
            let b = &stats.boards[i];
            let o = b.offset_cm();
            s += &format!(
                "last: {score} at {:.0} m\ngroup ({}): {:.1} cm\noffset: {}, {} cm\nboard total {} / {} hits",
                lane::DISTS[i],
                b.recent.len(),
                b.group_cm(),
                side(o[0], "R", "L"),
                side(o[1], "up", "down"),
                b.score,
                b.hits
            );
        }
        None => s += "last: -\nBackspace/G reset   V inspect",
    }
    t.0 = s;
}

/// Draw the lane: firing line, paper, rings and distance labels (the collision boxes come from
/// the world; draw those with `spawn_boxes`).
pub fn spawn_gallery(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
) {
    let unlit = |c: Color| StandardMaterial {
        base_color: c,
        unlit: true,
        ..default()
    };
    let disc = meshes.add(Circle::new(1.0));
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    let white = mats.add(unlit(Color::srgb(0.96, 0.96, 0.96)));
    let sign_mat = mats.add(unlit(Color::srgb(0.12, 0.13, 0.15)));
    let paper = meshes.add(Rectangle::new(
        (lane::BOARD_HALF_W - 0.05) * 2.0,
        (lane::BOARD_HALF_H - 0.05) * 2.0,
    ));
    let paper_mat = mats.add(unlit(Color::srgb(0.93, 0.91, 0.84)));
    // Ring materials by score index: 10 red, then alternating dark and pale.
    let ring_mats: Vec<Handle<StandardMaterial>> = (0..lane::RINGS)
        .map(|k| {
            let score = lane::RINGS - k;
            mats.add(unlit(if score == 10 {
                Color::srgb(0.8, 0.12, 0.1)
            } else if score % 2 == 1 {
                Color::srgb(0.1, 0.1, 0.11)
            } else {
                Color::srgb(0.9, 0.88, 0.8)
            }))
        })
        .collect();
    for (i, d) in lane::DISTS.iter().enumerate() {
        let c = bv(lane::centre(i));
        commands.spawn((
            Mesh3d(paper.clone()),
            MeshMaterial3d(paper_mat.clone()),
            Transform::from_translation(c + Vec3::Z * 0.001),
        ));
        // Outer ring first; each disc a fraction of a millimetre nearer the shooter.
        for k in 0..lane::RINGS {
            let r = lane::RING_W * (lane::RINGS - k) as f32;
            commands.spawn((
                Mesh3d(disc.clone()),
                MeshMaterial3d(ring_mats[k as usize].clone()),
                Transform::from_translation(c + Vec3::Z * (0.002 + 0.0004 * k as f32))
                    .with_scale(Vec3::splat(r)),
            ));
        }
        let sign = Vec3::new(c.x, lane::CENTRE_Y + lane::BOARD_HALF_H + 0.25, c.z);
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(1.3, 0.5, 0.04))),
            MeshMaterial3d(sign_mat.clone()),
            Transform::from_translation(sign - Vec3::Z * 0.02),
        ));
        spawn_label(commands, &quad, &white, &format!("{d:.0}m"), sign + Vec3::Z * 0.003);
    }
    // firing line across the lane
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(8.0, 0.01, 0.12))),
        MeshMaterial3d(mats.add(unlit(Color::srgb(0.95, 0.9, 0.2)))),
        Transform::from_xyz(lane::LANE_X, 0.006, lane::FIRING_Z),
    ));
}

/// Seven-segment lettering (digits and a block "m") from flat quads, since Text2d needs a 2D
/// camera. `text` is centred on `at` in the XY plane, facing +Z.
fn spawn_label(commands: &mut Commands, quad: &Handle<Mesh>, mat: &Handle<StandardMaterial>, text: &str, at: Vec3) {
    // segments a (top), b, c (right, upper and lower), d (bottom), e, f (left, lower and upper), g
    const SEG: [&str; 10] = [
        "abcdef", "bc", "abdeg", "abcdg", "bcfg", "acdfg", "acdefg", "abc", "abcdefg", "abcdfg",
    ];
    let (w, h, t, gap) = (0.2, 0.34, 0.045, 0.07);
    let width = |ch: char| if ch == 'm' { 0.3 } else { w };
    let total: f32 = text.chars().map(|ch| width(ch) + gap).sum::<f32>() - gap;
    let mut x = -total / 2.0;
    let mut bar = |cx: f32, cy: f32, sx: f32, sy: f32| {
        commands.spawn((
            Mesh3d(quad.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(at + Vec3::new(cx, cy, 0.0)).with_scale(Vec3::new(sx, sy, 1.0)),
        ));
    };
    for ch in text.chars() {
        let cw = width(ch);
        let cx = x + cw / 2.0;
        if ch == 'm' {
            // a block arch with a middle leg, x-height only
            let mh = h * 0.6;
            let cy = -h / 2.0 + mh / 2.0;
            bar(cx, cy + mh / 2.0 - t / 2.0, cw, t);
            for dx in [-cw / 2.0 + t / 2.0, 0.0, cw / 2.0 - t / 2.0] {
                bar(cx + dx, cy, t, mh);
            }
        } else if let Some(d) = ch.to_digit(10) {
            for sg in SEG[d as usize].chars() {
                let (dx, dy, sx, sy) = match sg {
                    'a' => (0.0, h / 2.0 - t / 2.0, w, t),
                    'd' => (0.0, -h / 2.0 + t / 2.0, w, t),
                    'g' => (0.0, 0.0, w, t),
                    'b' => (w / 2.0 - t / 2.0, h / 4.0, t, h / 2.0),
                    'c' => (w / 2.0 - t / 2.0, -h / 4.0, t, h / 2.0),
                    'e' => (-w / 2.0 + t / 2.0, -h / 4.0, t, h / 2.0),
                    _ => (-w / 2.0 + t / 2.0, h / 4.0, t, h / 2.0),
                };
                bar(cx + dx, dy, sx, sy);
            }
        }
        x += cw + gap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_track_shots_hits_and_recent_group() {
        let mut s = GalleryStats { shots: 12, ..default() };
        // 12 holes on board 2 marching right; only the last GROUP_N count
        for n in 0..12 {
            s.record(2, [n as f32 * 0.01, 0.0]);
        }
        let b = &s.boards[2];
        assert_eq!(b.recent.len(), lane::GROUP_N);
        assert!((b.group_cm() - 9.0).abs() < 1e-3, "{}", b.group_cm());
        assert!((b.offset_cm()[0] - 6.5).abs() < 1e-3);
        assert_eq!(s.hits, 12);
        assert!((s.hit_percent() - 100.0).abs() < 1e-4);
        assert_eq!(s.last.map(|l| l.0), Some(2));
        // x = 0.11 m is ring 10 - floor(0.11 / 0.06) = 9
        assert_eq!(s.last.unwrap().1, 9);
    }
}
