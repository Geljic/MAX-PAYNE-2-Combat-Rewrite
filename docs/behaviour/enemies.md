# Enemies: what MP2's data says

Collected 2026-10-11 from the extracted database. Where this data is silent, payne's enemy AI is
its own design; MP2's real behaviour lives in level FSM scripts and AI code we have not read.

## Classes (`MP2_Init/data/database/ai_ranges.h`)

| Class | Who | Health |
|---|---|---|
| C00 | bystanders, NPCs | 45 |
| C01 | Cleaners, Mafiosos | 50 |
| C02 | Cleaner bosses | 55 |
| C03 | semi-Cleaners, Russians | 60 |
| C04 | BlackOps | 60 |
| C05 | BlackOps bosses | — |

Each enemy skin's `[AI]` section (for example `skins/C01_Mafioso_A.txt`) takes its values from
these per-class constants:

| Key | C01 | C03 | C04 | Meaning |
|---|---|---|---|---|
| AimingSpeed | 240 | 240 | 240 | how fast the aim turns (degrees per second) |
| HeadTurningSpeed | 360 | 360 | 360 | how fast the head turns (degrees per second) |
| TriggerHappinessAngle | 35 | 30 | 20 | starts shooting once the aim is within this many degrees of the target |
| InitialInaccuracy | 4.0 | 3.5 | 3.0 | metres of error per 25 m when it starts aiming |
| StableInaccuracy | 2.5 | 2.5 | 2.0 | metres per 25 m once settled |
| StabilizationTime | 3.0 | 3.0 | 2.0 | seconds to go from initial to stable |
| FieldOfVision | 150 | 150 | 150 | degrees |
| HearingGroupOne/Two/ThreeRadius | 1000 / 5 / 2 | same | same | metres it can hear each loudness group (gunshots are group one) |
| IdleTimeout | 10 | 10 | 10 | seconds |

Enemies also have:
- `SlowMotionBonusForKiller`: 0.12 (`ENEMY_SLOWMOTIONBONUS`), or 0.17 for hard enemies. This is
  the bullet-time meter the player gains per kill.
- Capsule radius 0.31 (Max's is 0.48), capsule top 1.95.
- `ShootingFrequencyMultiplier` 1.0.
- Shootdodge frames 10 and 45; enemies can shootdodge too.

## Enemy bullets
Enemy projectiles (for example `projectiles/BulletBerettaSingle_Enemy.txt`) use the same damage
and spread as the player's, but `Speed = BULLETVELOCITY_ENEMY = 50` m/s, ±5%. Player bullets fly
at 150 m/s. The slower enemy bullets are what make dodging them in bullet time possible.
`shooting.h` notes that bullets are drawn only below 100 m/s (`BULLETVELOCITY_RENDERTRESHOLD`),
so the player's own bullets become visible only once bullet time slows them below that.

## Difficulty (`MP2_Init/data/difficulty.txt`; "default" is normal)

| Key | minimum | default | maximum |
|---|---|---|---|
| InitialInaccuracyMultiplier | 5.0 | 1.3 | 0.5 |
| StableInaccuracyMultiplier | 3.0 | 1.3 | — |
| HealthRegenerateMultiplier (% of max health per s) | 5% | 4% | 2% |
| HealthGainedImmediately | 10% | 5% | 1% |
| HealthGainedFromPainkiller | 30% | 20% | 15% |
| BulletTimeGainedPerKillMultiplier | 2.0 | 1.0 | 0.6 |
| AutoAimCapsuleRadius/HeightMultiplier | 8 / 3 | 4 / 2 | 1 / 1 |

## Animations
Enemies use the male skeleton's default set, overlaid with a style set:
- `skins/Male_Normal.h`, `Male_Sharp.h` or `Male_Lazy*`;
- clip files are prefixed `Normal_`, `Sharp_` and `Lazy_` (for example `Sharp_Run_Forward.kf2`
  with a `_mov` file);
- the style sets also give combat runs, confusion and shooting animations, each with footstep
  sounds on given frames.

Hit reactions are the SMALLDAMAGE, MEDIUMDAMAGE and LARGEDAMAGE clips and the `KNOCK*` clips.
Deaths are `RANDOMDEATH1–4`, `SHOTGUNDEATH`, `GRENADEDEATH`, `FALLINGDEATH` and so on, plus the
ragdoll (`RagdollDeathAnimationName = Sub_RagdollDeath`).

## payne's design (core AI, 2026-10-12)

MP2's real enemy behaviour lives in per-level FSM scripts and AI code we have not read. payne
uses the data above wherever it exists and its own design for the rest. The code is
`src/enemy.rs`; enemies are targets with a brain (`Sim::spawn_enemy(pos, yaw, skin)`).

### State machine (`enemy::Mode`)

| Mode | Does | Leaves when |
|---|---|---|
| Idle | stands at its post | sees Max → Combat; hears him, is shot at or is told → Alerted |
| Alerted | turns to look at the noise | sees Max → Combat; after `alert_look_time` → Search there |
| Combat | holds position, aims, shoots; sidesteps now and then | shot at and cover nearby → TakeCover; out of sight for `lose_sight_time` → Search |
| TakeCover | runs to the cover point | arrives → InCover |
| InCover | crouches for `hide_time`, stands for `peek_time` and shoots if it sees Max | flanked (seen while crouched) → Combat; `max_unseen_peeks` peeks without seeing him → Search |
| Search | runs to the last known position, then looks around (`CONFUSED`) | sees Max → Combat; no news for the class's `IdleTimeout` → Idle |

Being hit or nearly hit (a player bullet within `near_miss_radius` of the chest) counts as being
shot at. Flinching or staggering (the target's own reactions) stops moving and shooting.

### Perception
- **Sight:** within `FieldOfVision` (150°, centred on the body's facing), within `sight_range`,
  and a clear line from the eyes (`eye_height`, scaled when crouched) to Max's chest or the top of
  his hit capsule. Only level geometry blocks sight; other enemies do not.
- **Hearing:** a noise is heard within the class's radius for its loudness group. Max's gunshots
  are group one (1000 m, from MP2's data). payne's own choice: running footsteps (faster than
  `loud_footstep_speed`) and hard landings are group two (5 m); walking, crouch-walking and soft
  landings are group three (2 m). An enemy's own gunshot is a group-one noise for the others, who
  look where the shooter aimed. MP2 itself makes footstep noise with `pressurewave_footstep`
  projectiles on the run clips' footstep frames.
- **Alerting:** an enemy that first spots Max tells every enemy within `alert_radius` (15 m,
  ESTIMATE) where he is. Each enemy sends `Event::EnemyAlerted` once per alert, until it is idle
  again.

### Aiming and shooting (MP2's data, payne's reading of it)
- The gun turns towards Max's chest at `AimingSpeed` deg/s; the body turns at `HeadTurningSpeed`.
- It fires only once the gun is within `TriggerHappinessAngle` of Max, so the first shots of a
  turning enemy come from a gun still sweeping in.
- Inaccuracy is a disc around the gun direction whose radius is metres of error per 25 m:
  `InitialInaccuracy × InitialInaccuracyMultiplier` when it starts aiming, settling linearly to
  `StableInaccuracy × StableInaccuracyMultiplier` over `StabilizationTime` seconds of aiming.
  Shots land uniformly over the disc (so the mean miss is 2/3 of the radius). The weapon's own
  scatter (`Accuracy / 2500`) is added on top. Losing sight of Max for `reacquire_time` (1 s,
  ESTIMATE) restarts the settling when he is seen again.
- Fire rate: the weapon's `ShootingFrequency` × the skin's `ShootingFrequencyMultiplier`.
  Enemies carry a roster weapon (Beretta by default) with a magazine and reload; spare rounds never
  run out (ASSUMPTION).
- Bullets are real projectiles at the `_Enemy` projectile's speed (50 m/s ±5%). They never hit
  their shooter; they hit Max's hit capsule, other enemies (friendly fire, no meter for Max) and
  the level. Max's hit capsule: radius `player_hit_radius` (0.35 m, ESTIMATE standing in for his
  target bones), upright when standing, shorter crouched, lying along the body (length
  `prone_hit_length`) in a dive, prone and get-up. Nothing hits him while a dodge roll has
  projectile collisions off, or during the spin's immortality.
- Everything runs on the world clock: bullet time slows enemies, their aim and their bullets.

### Cover
`World::cover_points()` lists candidate spots (default none; `TestWorld` puts points about a metre
apart, 0.45 m out from every box at least 0.9 m tall). A spot counts as cover when the line from
Max's chest to `cover_test_height` (0.9 m) above it is blocked. Spots the enemy can also shoot over
when standing are preferred (others cost `full_cover_penalty`), then the nearest within
`cover_radius`; two enemies do not take the same spot.

### Player health
- Painkillers (`Input::painkiller`; `Event::Painkiller`): `HealthGainedImmediately` of maximum
  health at once, then regeneration at `HealthRegenerateMultiplier` per second until the painkiller
  has given `HealthGainedFromPainkiller` in all (ASSUMPTION: the immediate part counts towards
  the total). Normal: +12.5 at once, then 10/s up to +50 in all. Refused at full health.
  Carried: 3 at the start, 8 at most (ESTIMATEs).
- Auto-heal: below the mode's `AutoHealLevel` (15%) health creeps back up to it at
  `AutoHealHealthIncreasePerSecond` (1%/s). ASSUMPTION about what these `.properties` keys mean.
- Tuning's `regeneration_time` (90 s) is the bullet-time meter's recharge, not health.
- Kill bonus: the victim's `SlowMotionBonusForKiller` × `BulletTimeGainedPerKillMultiplier` (now
  a difficulty value; the Tuning field of the same name is gone).

### Animation (`Target::enemy_anim`)
The slot is chosen from the state: `SMALLDAMAGE` (flinch), `LARGEDAMAGE` (stagger), `CROUCH`
(behind cover), `RUN*` / `WALK*` in eight directions relative to the facing, `CONFUSED`
(searching), else `STAND`. Hosts resolve it with `clip_candidates(style, slot)`. A dead enemy has
no slot: an ordinary MP2 death has no clip, only the ragdoll. Move speeds come from the same
lookup on the clips' root curves (`RUNFORWARD`, `RUNLEFT`, `RUNBACKWARD`, `WALKFORWARD`); a curve
in `payne.data` has no frame rate, so payne assumes 50 fps for runs and 30 for walks (Max's
measured rates) unless `EnemyData::measure_speeds(&anims)` reads the exact rate from `anims.bin`.

### Values
MP2 data: everything in the tables above (classes, skins, difficulty, enemy bullet speed), the
enemy capsule, the run/walk root curves. ESTIMATE (`AiTuning` and Tuning): eye height 1.6 m,
crouch scale 0.7, sight range 100 m, alert radius 15 m, lose sight 2 s, reacquire 1 s, alert look
1 s, sidestep every 2–5 s for 0.5–1 s, cover radius 12 m, cover cooldown 4 s, cover test height
0.9 m, hide 1.5 s, peek 2.5 s, 2 unseen peeks, near miss 1.5 m, loud footsteps above 3 m/s,
fallback speeds (run 5.0, strafe 4.6, back 3.75, walk 1.2 m/s), acceleration 30 m/s², player hit
radius 0.35 m, prone hit length 1.7 m, painkillers 3 / 8.
