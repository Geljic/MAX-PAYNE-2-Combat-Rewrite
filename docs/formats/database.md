# Max Payne 2 text database

Written from reading the retail game's own files. Nothing here is copied from
other tools. Parser: `tools/extract/src/database.rs`. Emitter:
`tools/extract/src/generate.rs`, which writes `assets/generated/payne.data`.

## Where the files live

Each archive extracts to `<archive>/data/...`. The game sees all archives as one
tree and so does the extractor (scan order: `MP2_Init`, `MP2_Data`, then the rest
alphabetically; first hit wins, a duplicate with different bytes is reported as a
conflict; the retail set has none). Paths are case-insensitive and use
backslashes in the files.

`data/globalconfig.txt`, `data/difficulty.txt` and `data/defaultvalues.txt` sit at
`data/` level, everything else under `data/database/`.

## Syntax

A C-preprocessor subset wrapped around an INI-like tree.

- Comments: `// ...` and `/* ... */`. Quotes protect the markers.
- `#include <..\x.h>` or `"x.h"`: tried relative to the including file, then
  `data/database/`, then `data/`. A missing include is reported, not fatal.
- `#define NAME value`: names are case-insensitive; the value is the rest of
  the line. Object-like only (no function macros, no `#if`) in this data set.
- `[Name]` starts a section. If the next token is `{`, the section's contents
  are the braces. If not, it is *open*: the `Key = Value;` statements that follow
  belong to it until the next header or the end of the enclosing block.
  That is why `[Animation] Index = X; Filename = "y";` and `[Bone] Name = "z";`
  work on one line, and why `[Properties] { ... }` after an `[Animation]` is a
  sibling, not a child.
- `Key = Value;` statements, several per line. A value is a sequence of pieces
  up to the `;`: numbers (`12`, `0.05`, `0x7FFF`), percentages (`7%`), words
  (`YES NO TRUE FALSE`, identifiers, unquoted paths like `anim\Run.kf2`),
  tuples `( 0, 0.08, 0.07 )` and quoted strings with `\"` escapes.
- Adjacent pieces concatenate like C string literals once any is a string,
  which is how messages embed constants:
  `"RightHandWeapon->WS_AnimateShooting(" WEAPONANIM_PLAYERSHOOT ");"`.
  Some messages also leave inner quotes unescaped
  (`"f( "AI", "x" );"`); the same rule reads them correctly.

Indexes such as `CHARANIM_SHOOTDODGEFORWARD` are macros for the animation id.
The emitter keeps the name (minus the `XXXANIM_` prefix) and writes the number
as `id`.

## Messages

A `[Message] Frame = N; String = "receiver->Function( args );"` inside an
animation's `[Properties]` fires a script call when the animation reaches frame
`N`. Frames are animation frames at the game's animation rate. Receivers seen:
`this` (the character or weapon itself), `Maxpayne_Gamemode` (global game
state: game speed, bullet time) and `RightHandWeapon` / `LeftHandWeapon`.

## Files that matter for gunplay

| File | What it gives |
|------|---------------|
| `skins/MaxPayne.txt` | Max's capsule, health, shootdodge cut-off frames, hit targets (which bones count as head / torso / limbs, armour multipliers), start inventory |
| `skins/Male_Player.h` | Max's animation set: file, root-motion file, messages per animation (footsteps, shootdodge speed changes, shooting) |
| `skeletons/male.txt` | Sub-animations (face, torso overlays) Max's messages play through `C_SubAnimate` |
| `characteranimid.h`, `weaponanimid.h` | Name to id tables |
| `weapons/*.txt` | Per weapon: clip, pocket, rate, offsets, flags, weapon animations, which projectile the player's shot creates |
| `shooting.h` (Init) | Shared constants: bullet speed, damage, penetration, spread, slow-motion rate multipliers per weapon |
| `projectiles/*.txt` | Bullets, shells, clips, blood, explosions: speed, damage, gravity, ricochet, messages |
| `ragdolls/*.txt` | Rigid bodies and joint limits per death ragdoll detail level |
| `data/globalconfig.txt` | Bullet time meter, character physics, camera, painkiller |
| `data/difficulty.txt` | Per difficulty level multipliers (bullet time gained per kill, auto aim size, health regeneration) |
| `data/defaultvalues.txt` | Default key bindings only; nothing emitted |

## payne.data

Plain text. `[kind name]` header, then `key = value` lines; keys repeat and
order is kept. Values: booleans `true`/`false`, tuples space-separated,
percentages as fractions (`7%` is `0.07`), strings unquoted. Keys that the
extractor does not rename are the game's key in `snake_case`. File paths are
relative to `data/database/`, forward slashes, real casing, and have been
checked to exist.

Calls are written `receiver.Function arg arg`, with quoted arguments keeping
their quotes. Function names are case-insensitive in the game; the file uses the
first casing seen so a given call prints one way.

### `[character MaxPayne]`
`skeleton` (folder under `skeletons/`), `high_detail_ragdoll`, `low_detail_ragdoll`
(ragdoll file names), `capsule_top`, `capsule_bottom`, `capsule_radius` (metres,
character controller), `airborne_speed` (m/s horizontal speed while airborne),
`max_health`, `can_shootdodge_frame` (the dodge can still be turned into a
shootdodge up to this animation frame), `can_abort_shootdodge_frame`, other skin
flags, `ai_*` (the AI tuning block, used by enemies of this skin).
`target = NAME | type T | no_penetration_multiplier a | full_penetration_multiplier b | material m | bones A, B, ...`
is one hit zone: damage multipliers for bullets that do not / fully penetrate armour
and the bones that belong to it. `on_init_msg` / `on_death_msg` are start and death calls.

### `[anim NAME]` and `[subanim NAME]`
`id`, `file` (keyframe animation), `movement` (root-motion file, positions only),
`reference` (script name), `revert` (animation to play after), `msg = FRAME CALL`.
Messages of note: `Gamemode.Gm_ChangeGameSpeed target seconds` (game speed
multiplier ramp: first arg is the new speed, second is blend time in seconds),
`Gamemode.Gm_EnableBulletTime on/off`, `this.C_Jump n`. `subanim` entries come from
the skeleton file and are named by their `[Reference]`.

### `[weapon NAME]`
`geometry = file | distance d` per level of detail, `clip_size`, `pocket_size`
(max spare rounds), `shooting_frequency` (shots per second), `slowmo_multiplier`
(the file's `ShootingFrequencyMultiplier`: fire-rate factor in bullet time),
`two_handed_bones`, `two_handed_animation`, `shootdodge_allowed`,
`auto_aiming_allowed`, `bullet_offset` / `empty_shell_offset` / `clip_offset`
(metres, weapon-local spawn points), `slot`, `inventory_id`, `weapon_id`, aiming
and sniper-zoom values, then the player's bullet: `projectile`, and copied from
it `damage` (hit points), `penetration` (0..1, how much armour it ignores),
`bullet_spread` (the game's `Accuracy`: larger is looser; the unit is the game's
own, lower is straighter), `bullet_speed` (m/s).
`anim = NAME id file` starts a weapon animation; its messages follow as
`msg = NAME FRAME CALL`. Weapons without a player projectile (Empty, Melee,
Painkiller) produce a warning.

### `[projectile NAME]`
Every projectile a weapon creates, plus ones those create (clips, shells, blood).
`speed` (m/s), `speed_random` (fractional jitter), `damage`, `accuracy`,
`penetration_value`, `gravity` (multiplier of normal gravity), ricochet limits,
`explosion_delay` (seconds), flags, `on_decal` / `on_character_hit` calls and
animation messages (`msg = ANIM FRAME CALL`, e.g. explosions).

### `[ragdoll NAME]`
`geometry`, `cross_mode`, `collision_decal`.
`bone = NAME | weight w | restitution r | friction f [| inertia i]`
(relative mass, bounciness, friction, optional inertia scale; parents come before children).
`joint = BONE1 | BONE2 | twist_axis x y z | plane_axis x y z | twist_min | twist_max | cone_min | cone_max | plane_min | plane_max | friction`
(limits in degrees as written in the file). `remap = BONE | apparent_parent P`
attaches a bone that has no body to a bone that does.

### `[slowmotion global]`
Bullet time meter from `globalconfig.txt`: `slowmotion_time` (seconds a full
meter lasts), `regeneration_time` (seconds from empty to full), `threshold`,
`turn_off_penalty`, `shoot_dodge_penalty`, `shoot_dodge_threshold` (fractions of
the meter), `efficiency_*` (the speed-up that kills grant and how it decays),
`min/max_efficiency_game_speed_multiplier` (world speed at low and full
efficiency), `max_efficiency_player_time_multiplier`, `in_blend_time`,
`out_blend_time`, `efficiency_increase_blend_time` (seconds), `music_volume`,
`heartbeat_sound_interval`.

### `[game]`
Numeric and boolean keys of `globalconfig.txt`, prefixed by section:
`character_*` (falling and landing thresholds, slope limits, capsule limits,
shootdodge target offset, fall damage), `projectile_*`, `camera_*` (follow
offsets, field of view), `gamemode_*`, `painkiller_*`.

### `[difficulty LEVEL.SECTION]`
For levels `normal`, `nightmare`, `hell`, `timedmode` and sections `properties`,
`minimum`, `default`, `maximum` (the game interpolates between them by its
adaptive difficulty value): numeric keys such as
`bullet_time_gained_per_kill_multiplier`, `auto_aim_capsule_radius_multiplier`,
`health_regenerate_multiplier`.
