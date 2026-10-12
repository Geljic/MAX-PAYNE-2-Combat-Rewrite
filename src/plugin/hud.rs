//! HUD: crosshair, health, hourglass, weapon and ammo, state readout, help overlay, bullet-time tint.

use bevy::prelude::*;

use super::{PayneCombat, PayneEvent, PayneSim, PayneUi, bv, camera::PayneCamera};
use crate::event::Event;

#[derive(Component)]
pub enum Label {
    Info,
    Weapon,
    Health,
    Painkillers,
    Death,
    Help,
}

#[derive(Component)]
pub enum Bar {
    Health,
    Hourglass,
}

#[derive(Component)]
pub struct Tint;

/// One edge strip of the red flash; `.0` is the gradient angle (rad).
#[derive(Component)]
pub struct EdgeFlash(f32);

/// One hit-direction indicator slot.
#[derive(Component)]
pub struct HitMark(usize);

/// One floating debug label per enemy (F1).
#[derive(Component)]
pub struct EnemyLabel(usize);

/// Seconds a hit-direction indicator stays up.
const MARK_LIFE: f32 = 1.0;
/// Radius of the indicator ring around the crosshair (px).
const MARK_RING: f32 = 170.0;
const MARK_W: f32 = 90.0;
const MARK_H: f32 = 20.0;
const MARKS: usize = 6;
/// Thickness of the flash strips (share of the screen height / width).
const FLASH_BAND_H: Val = Val::Percent(24.0);
const FLASH_BAND_W: Val = Val::Percent(14.0);
const LABELS: usize = 12;
/// Damage that gives a full-strength flash. ESTIMATE: MP2 scales its blood overlay by damage; this
/// is by eye against the enemy bullet damage.
const FLASH_FULL_DAMAGE: f32 = 25.0;

/// Damage feedback state: edge flash strength (0..1) and the live hit marks.
#[derive(Resource, Default)]
pub struct DamageFx {
    pub flash: f32,
    /// (world yaw-direction the shot came from as a horizontal unit vector, age in seconds).
    marks: Vec<(Vec3, f32)>,
}

const HELP: &str = "\
payne range: bindings

WASD            move (Max always faces the aim)
Mouse           look            LMB fire
R               reload          Space jump
Ctrl / C        crouch, or dodge roll while moving
Shift           shootdodge (dive; standing still in bullet time: spin reload)
RMB             bullet time (toggle)
B               shootdodge slow motion on/off (off: only bullet time slows the world)
Wheel / 1-7     weapons
T               respawn targets (range)
P               painkiller (heals over a few seconds)
F2              combat mode: spawn a wave of enemies (T: back to paper targets)
Backspace / G   clear bullet holes and gallery stats; in combat or after death, reset Max
V               inspect camera on the last board hit
Esc             release mouse, click to recapture
H               this help       F1 debug gizmos (enemy modes, sight lines)

Gamepad: left stick move, right stick look, RT fire, LT bullet time, A jump,
B crouch, X reload, RB shootdodge, Y painkiller, D-pad left/right weapon";

pub fn setup(mut commands: Commands) {
    let font = |size: f32| TextFont {
        font_size: FontSize::Px(size),
        ..default()
    };
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        BackgroundColor(Color::srgba(0.2, 0.45, 0.9, 0.0)),
        Tint,
    ));
    // crosshair at the screen centre
    for (w, h) in [(14, 2), (2, 14)] {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: percent(50),
                top: percent(50),
                width: px(w),
                height: px(h),
                margin: UiRect {
                    left: px(-(w / 2)),
                    top: px(-(h / 2)),
                    ..default()
                },
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        ));
    }
    commands.spawn((
        Text::new(""),
        font(14.0),
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        Label::Info,
        Node {
            position_type: PositionType::Absolute,
            left: px(14),
            top: px(10),
            ..default()
        },
    ));
    // hourglass: a vertical bar down the left edge
    let glass = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(20),
                top: percent(28),
                width: px(16),
                height: px(220),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexEnd,
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.6)),
        ))
        .id();
    commands.spawn((
        Node {
            width: percent(100),
            height: percent(100),
            ..default()
        },
        BackgroundColor(Color::WHITE),
        Bar::Hourglass,
        ChildOf(glass),
    ));
    // health, bottom left
    let hp = commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: px(20),
            bottom: px(20),
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            ..default()
        })
        .id();
    commands.spawn((Text::new(""), font(18.0), Label::Health, ChildOf(hp)));
    commands.spawn((Text::new(""), font(16.0), Label::Painkillers, ChildOf(hp)));
    let back = commands
        .spawn((
            Node {
                width: px(240),
                height: px(12),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            ChildOf(hp),
        ))
        .id();
    commands.spawn((
        Node {
            width: percent(100),
            height: percent(100),
            ..default()
        },
        BackgroundColor(Color::srgb(0.75, 0.15, 0.12)),
        Bar::Health,
        ChildOf(back),
    ));
    commands.spawn((
        Text::new(""),
        font(22.0),
        Label::Weapon,
        Node {
            position_type: PositionType::Absolute,
            right: px(24),
            bottom: px(20),
            ..default()
        },
    ));
    // red edge flash: a gradient strip on each edge, opaque at the screen edge and clear inwards
    // (the angle is the direction the gradient runs: down from the top edge, and so on)
    use std::f32::consts::{FRAC_PI_2, PI};
    for (angle, vertical, far, thick) in [
        (PI, true, false, FLASH_BAND_H),
        (0.0, true, true, FLASH_BAND_H),
        (FRAC_PI_2, false, false, FLASH_BAND_W),
        (3.0 * FRAC_PI_2, false, true, FLASH_BAND_W),
    ] {
        let (near_v, near_h) = (
            if far && vertical { Val::Auto } else { px(0) },
            if far && !vertical { Val::Auto } else { px(0) },
        );
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: if vertical { percent(100) } else { thick },
                height: if vertical { thick } else { percent(100) },
                top: near_v,
                left: near_h,
                bottom: if far && vertical { px(0) } else { Val::Auto },
                right: if far && !vertical { px(0) } else { Val::Auto },
                ..default()
            },
            BackgroundGradient::default(),
            EdgeFlash(angle),
        ));
    }
    for i in 0..MARKS {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: percent(50),
                top: percent(50),
                width: px(MARK_W),
                height: px(MARK_H),
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.9, 0.05, 0.03, 0.0)),
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.0)),
            UiTransform::default(),
            Visibility::Hidden,
            HitMark(i),
        ));
    }
    for i in 0..LABELS {
        commands.spawn((
            Text::new(""),
            font(13.0),
            TextColor(Color::srgb(1.0, 0.9, 0.4)),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Hidden,
            EnemyLabel(i),
        ));
    }
    commands.spawn((
        Text::new(""),
        font(40.0),
        TextColor(Color::srgb(0.9, 0.12, 0.1)),
        TextLayout::justify(Justify::Center),
        Label::Death,
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            top: percent(32),
            justify_content: JustifyContent::Center,
            ..default()
        },
    ));
    commands.spawn((
        Text::new(HELP),
        font(15.0),
        Label::Help,
        Visibility::Hidden,
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(50),
            margin: UiRect {
                left: px(-290),
                top: px(-200),
                ..default()
            },
            width: px(580),
            padding: UiRect::all(px(14)),
            ..default()
        },
    ));
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn update(
    time: Res<Time<Real>>,
    sim: Res<PayneSim>,
    ui: Res<PayneUi>,
    combat: Res<PayneCombat>,
    mut fps: Local<f32>,
    mut texts: Query<(&Label, &mut Text, &mut Visibility)>,
    mut bars: Query<(&Bar, &mut Node, &mut BackgroundColor), Without<Tint>>,
    mut tint: Query<&mut BackgroundColor, With<Tint>>,
) {
    let s = &sim.0;
    let p = &s.player;
    let bt = &s.bullet_time;
    let inst = 1.0 / time.delta_secs().max(1e-4);
    *fps += (inst - *fps) * 0.05;
    let st = &p.weapon();
    let def = &s.weapons[st.def];
    let world = s.world_time_scale();
    for (l, mut t, mut v) in &mut texts {
        match l {
            Label::Info => {
                t.0 = format!(
                    "{:.0} fps   state: {}{}\nworld x{:.2}   player x{:.2}   efficiency {:.2}\ndive slow-mo {} (B)   H help   F1 debug",
                    *fps,
                    p.state.name(),
                    if combat.god { "   GOD" } else { "" },
                    world,
                    s.player_time_scale(),
                    bt.efficiency,
                    if s.tuning.shootdodge_slowmo > 0.5 {
                        "on"
                    } else {
                        "off"
                    }
                )
            }
            Label::Weapon => {
                let reload = if st.reload.is_some() {
                    "  reloading"
                } else {
                    ""
                };
                t.0 = format!("{}\n{} / {}{}", def.name, st.mag, st.reserve, reload)
            }
            Label::Health => t.0 = format!("HEALTH {:.0}", p.health.max(0.0)),
            Label::Painkillers => {
                t.0 = if p.heal_pending > 0.5 {
                    format!(
                        "PAINKILLERS {}  (P)   healing +{:.0}",
                        p.painkillers, p.heal_pending
                    )
                } else {
                    format!("PAINKILLERS {}  (P)", p.painkillers)
                }
            }
            Label::Death => {
                if p.alive() {
                    *v = Visibility::Hidden;
                } else {
                    t.0 = "YOU DIED\npress Backspace or G to reset".into();
                    *v = Visibility::Inherited;
                }
            }
            Label::Help => {
                *v = if ui.help {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                }
            }
        }
    }
    for (b, mut n, mut c) in &mut bars {
        match b {
            Bar::Health => {
                n.width = percent((p.health / s.tuning.max_health * 100.0).clamp(0.0, 100.0))
            }
            Bar::Hourglass => {
                n.height = percent((bt.meter * 100.0).clamp(0.0, 100.0));
                // white-blue, going yellow as kills raise the efficiency
                let e = bt.efficiency.clamp(0.0, 1.0);
                c.0 = Color::srgb(0.7 + 0.3 * e, 0.82 + 0.03 * e, 1.0 - 0.9 * e);
            }
        }
    }
    if let Ok(mut c) = tint.single_mut() {
        let k = (1.0 - world).clamp(0.0, 1.0);
        c.0 = Color::srgba(0.25, 0.5, 0.95, k * 0.3);
    }
}

/// Collect hits: flash strength and where the shots came from.
pub fn damage_events(
    time: Res<Time<Real>>,
    mut events: MessageReader<PayneEvent>,
    mut fx: ResMut<DamageFx>,
    sim: Res<PayneSim>,
) {
    let dt = time.delta_secs();
    fx.flash = (fx.flash - dt * 1.4).max(0.0);
    for (_, age) in &mut fx.marks {
        *age += dt;
    }
    fx.marks.retain(|(_, age)| *age < MARK_LIFE);
    for PayneEvent(e) in events.read() {
        match e {
            Event::PlayerHit { damage, dir } => {
                fx.flash = fx.flash.max((damage / FLASH_FULL_DAMAGE).clamp(0.3, 1.0));
                // the shooter is back along the way the bullet travelled
                let from = -bv(*dir);
                let from = Vec3::new(from.x, 0.0, from.z).normalize_or_zero();
                if from != Vec3::ZERO {
                    if fx.marks.len() >= MARKS {
                        fx.marks.remove(0);
                    }
                    fx.marks.push((from, 0.0));
                }
            }
            Event::PlayerDied => fx.flash = 1.0,
            _ => {}
        }
    }
    // keep the flash up while dead
    if !sim.0.player.alive() {
        fx.flash = fx.flash.max(0.6);
    }
}

/// Draw the edge flash and the hit-direction indicators (relative to where the camera looks).
#[allow(clippy::type_complexity)]
pub fn damage_draw(
    fx: Res<DamageFx>,
    cam: Single<&Transform, With<PayneCamera>>,
    mut flashes: Query<(&EdgeFlash, &mut BackgroundGradient)>,
    mut marks: Query<
        (
            &HitMark,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut UiTransform,
            &mut Visibility,
        ),
        (Without<EdgeFlash>, Without<Label>),
    >,
) {
    for (f, mut g) in &mut flashes {
        let red = |a: f32| Color::srgba(0.85, 0.04, 0.03, a);
        *g = BackgroundGradient::from(LinearGradient::new(
            f.0,
            vec![
                ColorStop::new(red(0.8 * fx.flash), Val::Percent(0.0)),
                ColorStop::new(red(0.0), Val::Percent(100.0)),
            ],
        ));
    }
    let fwd = {
        let f = cam.forward().as_vec3();
        Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z)
    };
    let right = fwd.cross(Vec3::Y);
    for (m, mut bg, mut border, mut tf, mut vis) in &mut marks {
        let Some((from, age)) = fx.marks.get(m.0) else {
            *vis = Visibility::Hidden;
            continue;
        };
        *vis = Visibility::Inherited;
        // angle clockwise from "straight ahead" on screen
        let a = from.dot(right).atan2(from.dot(fwd));
        let (w, h) = (MARK_W, MARK_H);
        tf.translation = Val2::px(
            a.sin() * MARK_RING - w * 0.5,
            -a.cos() * MARK_RING - h * 0.5,
        );
        tf.rotation = Rot2::radians(a - std::f32::consts::FRAC_PI_2);
        let k = (1.0 - age / MARK_LIFE).clamp(0.0, 1.0);
        bg.0 = Color::srgba(0.95, 0.06, 0.03, k * 0.9);
        *border = BorderColor::all(Color::srgba(1.0, 1.0, 1.0, k));
    }
}

/// F1: a label with each live enemy's mode over its head.
#[allow(clippy::type_complexity)]
pub fn enemy_labels(
    ui: Res<PayneUi>,
    sim: Res<PayneSim>,
    cam: Single<(&Camera, &GlobalTransform), With<PayneCamera>>,
    mut labels: Query<(&EnemyLabel, &mut Text, &mut Node, &mut Visibility)>,
) {
    let (camera, gt) = *cam;
    let live: Vec<&crate::Target> = sim
        .0
        .targets
        .iter()
        .filter(|t| t.alive() && t.ai.is_some())
        .collect();
    for (l, mut text, mut node, mut vis) in &mut labels {
        let at = live.get(l.0).filter(|_| ui.debug).and_then(|t| {
            let ai = t.ai.as_ref()?;
            let p = camera
                .world_to_viewport(gt, bv(t.pos) + Vec3::Y * (2.25 * t.stance))
                .ok()?;
            Some((ai, p))
        });
        let Some((ai, p)) = at else {
            *vis = Visibility::Hidden;
            continue;
        };
        text.0 = format!("{}{}", ai.mode.name(), if ai.sees { "  sees" } else { "" });
        node.left = px(p.x - 30.0);
        node.top = px(p.y - 8.0);
        *vis = Visibility::Inherited;
    }
}
