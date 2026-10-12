//! Game data loaded at runtime from `assets/generated/payne.data`, which the extractor writes
//! from the player's own MP2 install. Nothing from the game is compiled in beyond the ESTIMATE
//! defaults, so the crate builds and runs without the game.
//!
//! Format: plain text, `#` comments, sections with ordered (repeatable) keys:
//! ```text
//! [character MaxPayne]
//! capsule_top = 1.95
//! [anim SHOOTDODGEFORWARD]
//! frames = 51
//! fps = 30
//! distance = 5.8
//! root = 0 0.02 0.07 ...   # root-motion curve: distance from the start at each frame (m)
//! root_dir = 0 1           # travel direction in the clip's frame (x right, z forward)
//! msg = 10 Gm_ChangeGameSpeed 0.2 0.3
//! ```
//! [`DataFile`] is the generic parser; [`GameData`] is the typed view that overrides
//! [`Tuning`], the weapon roster, the [`MoveSet`], the ragdoll skeleton and the enemy
//! [`TargetDef`], plus the enemy classes, skins and clip speeds ([`EnemyData`]) and the
//! [`Difficulty`]. Section kinds understood: `character`, `slowmotion`, `difficulty`, `level`,
//! `anim`, `weapon`, `projectile`, `ragdoll`, `enemyclass`, `enemyskin`. Unknown kinds and keys
//! are kept and reported, not errors.

use std::path::Path;

use crate::enemy::{Difficulty, EnemyData};
use crate::moves::{AnimMsg, MoveDef, MoveSet, Msg};
use crate::ragdoll::SkeletonDef;
use crate::target::TargetDef;
use crate::tuning::Tuning;
use crate::weapon::{FireMode, WeaponClass, WeaponDef, default_roster, spread_units_to_rad};

/// Parse a number as MP2 writes them: `1.5`, `7%` (→ 0.07), `TRUE`/`FALSE` (→ 1/0).
pub fn parse_number(s: &str) -> Option<f32> {
    let s = s.trim().trim_end_matches(';').trim().trim_matches('"');
    if let Some(p) = s.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().map(|v| v / 100.0);
    }
    match s.to_ascii_lowercase().as_str() {
        "true" | "yes" => Some(1.0),
        "false" | "no" => Some(0.0),
        _ => s.parse::<f32>().ok(),
    }
}

fn parse_flag(s: &str) -> Option<bool> {
    parse_number(s).map(|v| v != 0.0)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    pub kind: String,
    pub name: String,
    /// Keys and values in file order; keys may repeat.
    pub entries: Vec<(String, String)>,
}

impl Section {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    pub fn get_all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.entries
            .iter()
            .filter(move |(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    pub fn num(&self, key: &str) -> Option<f32> {
        self.get(key).and_then(parse_number)
    }
    pub fn flag(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(parse_flag)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DataError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "payne.data line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for DataError {}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DataFile {
    pub sections: Vec<Section>,
}

impl DataFile {
    pub fn parse(text: &str) -> Result<DataFile, DataError> {
        let mut sections: Vec<Section> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(head) = line.strip_prefix('[') {
                let head = head
                    .strip_suffix(']')
                    .ok_or(DataError {
                        line: i + 1,
                        message: "unclosed [".into(),
                    })?
                    .trim();
                let (kind, name) = head.split_once(char::is_whitespace).unwrap_or((head, ""));
                sections.push(Section {
                    kind: kind.to_ascii_lowercase(),
                    name: name.trim().to_string(),
                    entries: Vec::new(),
                });
                continue;
            }
            let (k, v) = line.split_once('=').ok_or(DataError {
                line: i + 1,
                message: format!("expected key = value, got `{line}`"),
            })?;
            let sec = sections.last_mut().ok_or(DataError {
                line: i + 1,
                message: "key before any [section]".into(),
            })?;
            sec.entries
                .push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
        Ok(DataFile { sections })
    }

    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Section> + 'a {
        self.sections.iter().filter(move |s| s.kind == kind)
    }

    /// Section by kind and (case-insensitive) name.
    pub fn section<'a>(&'a self, kind: &'a str, name: &str) -> Option<&'a Section> {
        self.of_kind(kind)
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }
}

/// Everything the simulation takes from game data, with ESTIMATE defaults where data is missing.
#[derive(Clone, Debug, PartialEq)]
pub struct GameData {
    pub tuning: Tuning,
    pub weapons: Vec<WeaponDef>,
    pub moves: MoveSet,
    pub skeleton: SkeletonDef,
    /// Definition for enemies (health, kill bonus) from the enemy character section.
    pub target: TargetDef,
    /// Enemy classes, skins, clip speeds and AI tuning.
    pub enemies: EnemyData,
    /// The difficulty in use: `normal.default` when present (MP2's normal level).
    pub difficulty: Difficulty,
    /// Every difficulty level in the data, to choose from.
    pub difficulties: Vec<Difficulty>,
    /// Keys and sections that were not understood (for a log line, not an error).
    pub unknown: Vec<String>,
}

impl Default for GameData {
    fn default() -> Self {
        GameData {
            tuning: Tuning::default(),
            weapons: default_roster(),
            moves: MoveSet::default(),
            skeleton: SkeletonDef::humanoid(),
            target: TargetDef::default(),
            enemies: EnemyData::default(),
            difficulty: Difficulty::default(),
            difficulties: Vec::new(),
            unknown: Vec::new(),
        }
    }
}

/// Character section keys that are not Tuning names.
const CHARACTER_EXTRA: &[&str] = &[
    "slow_motion_bonus_for_killer",
    "health_bonus_for_killer",
    "skeleton",
];

impl GameData {
    /// Load `path`; a missing file gives the defaults. A malformed file is an error.
    pub fn load(path: impl AsRef<Path>) -> Result<GameData, DataError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(GameData::from_file(
                &DataFile::parse(&text)?,
                "MaxPayne",
                "C01_Mafioso_A",
            )),
            Err(_) => Ok(GameData::default()),
        }
    }

    /// Typed view of a parsed file. `player` and `enemy` name the `[character ...]` sections to use.
    pub fn from_file(file: &DataFile, player: &str, enemy: &str) -> GameData {
        let mut g = GameData::default();
        if let Some(s) = file.section("character", player) {
            for (k, v) in &s.entries {
                let known = parse_number(v).is_some_and(|n| g.tuning.set(k, n));
                if !known && !CHARACTER_EXTRA.contains(&k.as_str()) {
                    g.unknown.push(format!("character.{k}"));
                }
            }
        }
        if let Some(s) = file.section("character", enemy) {
            if let Some(h) = s.num("max_health") {
                g.target.health = h;
            }
            if let Some(b) = s.num("slow_motion_bonus_for_killer") {
                g.target.slowmo_bonus = b;
            }
        }
        // Difficulty sections feed `difficulty` below, not Tuning.
        for s in file.of_kind("slowmotion").chain(file.of_kind("level")) {
            for (k, v) in &s.entries {
                if !parse_number(v).is_some_and(|n| g.tuning.set(k, n)) {
                    g.unknown.push(format!("{}.{k}", s.kind));
                }
            }
        }
        for s in file.of_kind("anim") {
            apply_anim(&mut g.moves, s);
        }
        // Directions without messages of their own play the forward dive's / get-up's timeline.
        for set in [&mut g.moves.shootdodge, &mut g.moves.getup] {
            let fwd = set[0].msgs.clone();
            for m in set.iter_mut().skip(1) {
                if m.msgs.is_empty() {
                    m.msgs = fwd.clone();
                }
            }
        }
        for s in file.of_kind("weapon") {
            apply_weapon(&mut g.weapons, s, file);
        }
        for s in file.of_kind("ragdoll") {
            apply_ragdoll(&mut g.skeleton, &mut g.tuning, s);
        }
        g.enemies = EnemyData::from_file(file);
        g.difficulties = Difficulty::all(file);
        if let Some(d) = Difficulty::from_file(file, "normal.default") {
            g.difficulty = d;
        }
        // The default enemy (spawn_target) takes the C01_Mafioso_A skin when it is loaded.
        if let Some(k) = g.enemies.skins.iter().find(|k| k.name.eq_ignore_ascii_case(enemy)) {
            g.target.health = k.class.health;
            g.target.slowmo_bonus = k.slowmo_bonus;
        }
        g
    }
}

fn strip_anim_name(n: &str) -> String {
    let u = n.to_ascii_uppercase();
    u.strip_prefix("CHARANIM_").unwrap_or(&u).to_string()
}

/// `msg = <frame> <Function> <args...>`, args separated by spaces or commas.
fn parse_msg(v: &str) -> Option<AnimMsg> {
    let mut parts = v
        .split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .filter(|p| !p.is_empty());
    let frame = parse_number(parts.next()?)?;
    let func = parts.next()?;
    let args: Vec<&str> = parts.collect();
    Some(AnimMsg {
        frame,
        msg: Msg::parse(func, &args),
    })
}

fn apply_anim(moves: &mut MoveSet, s: &Section) {
    let name = strip_anim_name(&s.name);
    let msgs: Vec<AnimMsg> = s.get_all("msg").filter_map(parse_msg).collect();
    let update = |m: &mut MoveDef| {
        if let Some(f) = s.num("frames") {
            m.frames = f;
        }
        if let Some(f) = s.num("fps") {
            m.fps = f;
        }
        // `movement` may be the root-motion file name; only a number is a distance.
        if let Some(d) = s.num("distance").or_else(|| s.num("movement")) {
            m.distance = d;
        }
        // `root = d0 d1 ...`: the root-motion curve, one distance per frame.
        if let Some(r) = s.get("root") {
            let curve: Vec<f32> = r.split_whitespace().filter_map(parse_number).collect();
            if curve.len() >= 2 {
                m.distance = *curve.last().unwrap_or(&m.distance);
                m.root = curve;
            }
        }
        if let Some(d) = s.get("root_dir") {
            let v: Vec<f32> = d.split_whitespace().filter_map(parse_number).collect();
            if let [x, z] = v[..] {
                m.root_dir = Some([x, z]);
            }
        }
        m.msgs = msgs.clone();
    };
    for m in moves.all_mut() {
        if m.name == name {
            update(m);
        }
    }
    // The whirl has a left and a right version; either feeds the spin.
    if name == "WHIRLRIGHT" && moves.spin.msgs.is_empty() {
        update(&mut moves.spin);
    }
}

fn apply_weapon(roster: &mut Vec<WeaponDef>, s: &Section, file: &DataFile) {
    let idx = match roster
        .iter()
        .position(|w| w.name.eq_ignore_ascii_case(&s.name))
    {
        Some(i) => i,
        None => {
            let dual = s.name.to_ascii_lowercase().contains("dual");
            let mut d = default_roster()[0].clone();
            d.name = s.name.clone();
            d.dual = dual;
            d.class = if dual {
                WeaponClass::DualPistol
            } else {
                WeaponClass::Pistol
            };
            roster.push(d);
            roster.len() - 1
        }
    };
    let w = &mut roster[idx];
    if let Some(v) = s.num("clip_size") {
        w.magazine = v as u32;
        if w.reload_amount > 1 {
            w.reload_amount = w.magazine;
        }
    }
    if let Some(v) = s.num("pocket_size") {
        w.pocket = v as u32;
    }
    if let Some(v) = s.num("reload_amount") {
        // MP2 writes ReloadAmount = -1 for "fill the whole magazine"; only shotguns load 1 or 2.
        w.reload_amount = if v < 0.0 { w.magazine } else { v as u32 };
    }
    if let Some(v) = s.num("damage") {
        w.damage = v;
    }
    if let Some(v) = s.num("penetration") {
        w.penetration = v;
    }
    if let Some(v) = s.num("shooting_frequency") {
        w.shooting_frequency = v;
        if !roster_has_mode(&s.name) {
            // ESTIMATE: MP2's data has no auto flag; fast weapons are automatic.
            w.fire_mode = if v > 5.0 {
                FireMode::Auto
            } else {
                FireMode::Semi
            };
        }
    }
    if let Some(v) = s.num("slowmo_multiplier") {
        w.slowmo_multiplier = v;
    }
    if let Some(v) = s.num("bullet_spread") {
        w.spread_base = spread_units_to_rad(v);
    }
    if let Some(v) = s.flag("two_handed") {
        w.two_handed = v;
    }
    if let Some(v) = s.flag("shootdodge_allowed") {
        w.shootdodge_allowed = v;
    }
    if let Some(v) = s.num("pellets") {
        w.pellets = v.max(1.0) as u32;
    }
    // Pellets from the shoot animation: frame-0 P_CreateProjectile(Bullet*, n) messages.
    let mut pellets = 0;
    for m in s.get_all("msg") {
        let parts: Vec<&str> = m
            .split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() >= 4
            && parts[1].eq_ignore_ascii_case("P_CreateProjectile")
            && parts[2].to_ascii_lowercase().starts_with("bullet")
        {
            pellets += parse_number(parts[3]).unwrap_or(1.0) as u32;
        }
    }
    if pellets > 0 {
        w.pellets = pellets;
    }
    if let Some(p) = s
        .get("projectile")
        .and_then(|n| file.section("projectile", n.trim_matches('"')))
    {
        if let Some(v) = p.num("speed") {
            w.projectile_speed = v;
        }
        if let Some(v) = p.num("speed_random") {
            w.projectile_speed_random = v;
        }
    }
    // Enemies fire the `_Enemy` twin of the player's projectile (the SHOOT animation's
    // P_CreateProjectile, e.g. BulletBerettaSingle_Enemy): same damage, BULLETVELOCITY_ENEMY speed.
    let enemy_name = s
        .get("projectile")
        .map(|n| n.trim_matches('"').replace("_Player", "_Enemy"))
        .or_else(|| {
            s.get_all("msg").find_map(|m| {
                m.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
                    .find(|p| p.to_ascii_lowercase().ends_with("_enemy"))
                    .map(str::to_string)
            })
        });
    if let Some(p) = enemy_name.and_then(|n| file.section("projectile", &n)) {
        if let Some(v) = p.num("speed") {
            w.enemy_projectile_speed = v;
        }
        if let Some(v) = p.num("speed_random") {
            w.enemy_projectile_speed_random = v;
        }
    }
}

fn roster_has_mode(name: &str) -> bool {
    default_roster()
        .iter()
        .any(|w| w.name.eq_ignore_ascii_case(name))
}

/// `bone = <name> | weight <w> | restitution <r> | friction <f>`. Relative weights become
/// masses scaled to the body's current total; restitution and friction average into Tuning.
fn apply_ragdoll(sk: &mut SkeletonDef, tu: &mut Tuning, s: &Section) {
    let total: f32 = sk.joints.iter().map(|j| j.mass).sum();
    let mut weights: Vec<(usize, f32)> = Vec::new();
    let (mut rest, mut fric, mut n) = (0.0, 0.0, 0.0);
    for v in s.get_all("bone") {
        let mut fields = v.split('|').map(str::trim);
        let Some(name) = fields.next() else { continue };
        let mut w = None;
        for f in fields {
            let (k, val) = f.split_once(char::is_whitespace).unwrap_or((f, ""));
            match (k.to_ascii_lowercase().as_str(), parse_number(val)) {
                ("weight", Some(x)) => w = Some(x),
                ("restitution", Some(x)) => {
                    rest += x;
                    n += 1.0;
                }
                ("friction", Some(x)) => fric += x,
                _ => {}
            }
        }
        if let (Some(j), Some(w)) = (sk.joint(name.trim_matches('"')), w) {
            weights.push((j, w));
        }
    }
    let wsum: f32 = weights.iter().map(|(_, w)| w).sum();
    if wsum > 0.0 {
        for (j, w) in weights {
            sk.joints[j].mass = total * w / wsum;
        }
    }
    if n > 0.0 {
        tu.ragdoll_restitution = rest / n;
        tu.ragdoll_friction = fric / n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "
# test
[character MaxPayne]
capsule_top = 1.9
max_health = 300
slow_motion_bonus_for_killer = 0%

[character C01_Mafioso_A]
max_health = 60
slow_motion_bonus_for_killer = 0.17

[slowmotion]
slowmotion_time = 10
threshold = 5%

[anim CHARANIM_SHOOTDODGEFORWARD]
frames = 60
fps = 30
distance = 6
msg = 10 Gm_ChangeGameSpeed 0.25 0.3
msg = 14 C_Jump 4
root = 0 0.5 1.5 3 6
root_dir = 0 1

[weapon PumpShotgun]
clip_size = 8
bullet_spread = 500
projectile = BulletPumpShotgun_Player
msg = 0 P_CreateProjectile BulletShotgun_Player 1
msg = 0 P_CreateProjectile BulletPumpShotgun_Player 8
msg = 0 PS_StartEffect muzzle_pumpshotgun

[projectile BulletPumpShotgun_Player]
speed = 120

[projectile BulletPumpShotgun_Enemy]
speed = 40
speed_random = 0.1

[difficulty normal.default]
initial_inaccuracy_multiplier = 1.5
health_gained_from_painkiller = 25%

[difficulty normal.properties]
auto_heal_level = 0.2

[difficulty hard.default]
initial_inaccuracy_multiplier = 0.5

[ragdoll highdetail]
bone = Bip01 Head | weight 10 | restitution 0.3 | friction 0.6
";

    #[test]
    fn typed_view_overrides_defaults() {
        let f = DataFile::parse(SAMPLE).unwrap();
        let g = GameData::from_file(&f, "MaxPayne", "C01_Mafioso_A");
        assert_eq!(g.tuning.capsule_top, 1.9);
        assert_eq!(g.tuning.max_health, 300.0);
        assert_eq!(g.tuning.slowmotion_time, 10.0);
        assert!((g.tuning.threshold - 0.05).abs() < 1e-6);
        assert_eq!(g.target.health, 60.0);
        assert_eq!(g.target.slowmo_bonus, 0.17);
        let sd = &g.moves.shootdodge[0];
        assert_eq!((sd.frames, sd.distance), (60.0, 6.0));
        assert_eq!(sd.msgs[1].msg, Msg::Jump(4.0));
        assert_eq!(sd.root, vec![0.0, 0.5, 1.5, 3.0, 6.0]);
        assert_eq!(sd.root_dir, Some([0.0, 1.0]));
        assert!((sd.distance_at(1.5 / 30.0) - 1.0).abs() < 1e-5);
        // Other directions without data keep a timeline.
        assert!(!g.moves.shootdodge[2].msgs.is_empty());
        let pump = g.weapons.iter().find(|w| w.name == "PumpShotgun").unwrap();
        assert_eq!(
            (pump.magazine, pump.pellets, pump.projectile_speed),
            (8, 9, 120.0)
        );
        assert!((pump.spread_base - 0.2).abs() < 1e-6);
        assert_eq!(
            (pump.enemy_projectile_speed, pump.enemy_projectile_speed_random),
            (40.0, 0.1)
        );
        assert_eq!(g.difficulty.name, "normal.default");
        assert_eq!(g.difficulty.initial_inaccuracy_multiplier, 1.5);
        assert!((g.difficulty.health_gained_from_painkiller - 0.25).abs() < 1e-6);
        assert_eq!(g.difficulty.auto_heal_level, 0.2);
        assert_eq!(g.difficulty.stable_inaccuracy_multiplier, 1.3);
        assert_eq!(g.difficulties.len(), 2);
        assert!((g.tuning.ragdoll_restitution - 0.3).abs() < 1e-6);
        assert!(g.unknown.is_empty(), "{:?}", g.unknown);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let g = GameData::load("/nonexistent/payne.data").unwrap();
        assert_eq!(g, GameData::default());
        assert!(DataFile::parse("key = 1").is_err());
    }
}
