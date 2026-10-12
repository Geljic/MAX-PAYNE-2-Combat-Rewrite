# Enemy data in `payne.data`

Written from reading the retail files. Emitter: `enemies()` in `tools/extract/src/generate.rs`.
Behaviour facts (what the numbers mean) are in `docs/behaviour/enemies.md`.

## Classes
`data/database/ai_ranges.h` (in `MP2_Init`) is a list of `#define C0x_NAME value` lines, one block
per class C00..C05. `generate` reads them as plain defines and writes one `[enemyclass C0x]` per
class: `health`, `aiming_speed`, `head_turning_speed`, `trigger_happiness_angle`,
`initial_inaccuracy`, `stable_inaccuracy`, `stabilization_time`, `field_of_vision`, `idle_timeout`
and `hearing_radius` (the three group radii on one line). The `S_*` and `ENEMY_*` defines in the
same header are not classes.

## Skins and how one picks its animation set
Every `skins/*.txt` with an `[AI]` section becomes `[enemyskin NAME]` (76 in the retail install).
- `class`: the `C0x_` prefix of the constants the `[AI]` section uses. A skin whose `[AI]` section
  holds literal numbers (for example `Enemy_MaxPayne`) has no class and carries those numbers
  itself under the same keys.
- Capsule, slow-motion bonus, shooting multiplier, airborne speed and shootdodge frames come from
  its `[Properties]`.
- `style`: a skin chooses its animation overlay by `#include`-ing a header from `skins/`.
  - `Male_Sharp.h` gives `SHARP` (C04 and C05 BlackOps, `S_Bodyguard_*`).
  - `Male_Normal.h` gives `NORMAL` (C02 bosses, C03 semi-Cleaners and the C03 Mafioso boss).
  - `Male_NPC.h` is for bystanders, NYPD and security guards. It has no style in payne; the
    skin gets `style = none` and an extra `overlay = npc` line.
  - No include at all (every C01 Cleaner and Mafioso, and the C03 Russians) means no overlay: the
    skeleton's own default set. Those skins are `style = none`.
  - A commented-out include does not count (`S_Hospital_Commando` has one for `Male_Sharp.h`,
    so it is `style = none`).
- There are no `Lazy_*` headers in this install, although `Lazy_*.kf2` clips exist.

## Animation names
Max's clips keep their plain slot names (`RUNFORWARD`; the `CHARANIM_` prefix dropped). Each
`[anim ...]` section has the same keys as Max's (`id`, `file`, `movement`, `msg`, `source`).
- `SHARP/<SLOT>` and `NORMAL/<SLOT>`: every entry of `Male_Sharp.h` / `Male_Normal.h`
  (77 and 28 slots). `source = sharp` / `normal`.
  They cover run, walk and strafe sets, their crouch (`C_`) and two-handed (`2_`) variants,
  and for Sharp the pistol and Colt Commando shooting poses. `Male_Normal.h` has only the
  crouch-set (`C_*`) entries, so a Normal enemy runs with the base set.
- `ENEMY/<SLOT>`: the base set for an enemy with no overlay. The skeleton file
  (`MP2_Init/.../skeletons/male.txt`) holds a default clip for each slot; Max's skin
  (`MaxPayne.txt` with `Male_Player.h`) replaces some. A slot gets an `ENEMY/` entry where the
  default differs from Max's clip (47 slots: the run and walk set, dodges, crouch pose and
  the per-weapon shooting poses). `source = enemy`. Where Max uses the skeleton default, the
  plain name already is the enemy's clip.
- Evidence the base set is the skeleton default and not `Male_NPC.h`: the ordinary C01 skins
  include neither a style header nor `Male_NPC.h`, and have no animation entries of their own.
- Lookup order for an enemy of style S: `S/<SLOT>`, then `ENEMY/<SLOT>`, then `<SLOT>`.

Reactions and deaths:
- Hit reactions (`VERYSMALLDAMAGE` to `VERYLARGEDAMAGE`, `EXPLOSIONDAMAGE`, `FIREDAMAGE`,
  `MELEEDAMAGE`) and `KNOCKFORWARDLEFT/RIGHT`, `KNOCKBACKWARDLEFT/RIGHT` are skeleton defaults that
  Max does not replace, so they resolve under the plain name.
- Clips exist for `BURNINGDEATH`, `BURNINGDEATH2`, `FALLINGDEATH`, `GRENADEDEATH` and
  `CUSTOMDEATH1-4` only. `RANDOMDEATH1-4`, `SHOTGUNDEATH` and `SHOTGUNDEATH2` are slots
  with no clip (the skeleton only reverts them to `POSE`): an ordinary death is the ragdoll
  (`RagdollDeathAnimationName = Sub_RagdollDeath`).

## Enemy projectiles
The `_Enemy` bullets carry `speed = 50` and `speed_random = 0.05`, like the player ones at 150.
The exceptions are the game's own: sniper and Dragunov bullets are 400 for both sides (random 0),
the enemy grenade is 10 (player 20) and the Molotov is 10 for both.
