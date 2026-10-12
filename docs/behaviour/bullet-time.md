# Bullet time

How Max Payne 2 runs bullet time, from the game mode's per-frame update in `MaxPayne2.exe`
(named `SlowMotion_Update` in our Ghidra project, 0x0042a6d0). Read 2026-10-11. Every
constant comes from `data/globalconfig.txt` `[slowmotion]` and `difficulty.txt`; the code
only combines them. All timers below run on real (unscaled) time.

## State

| Name | Range | Meaning |
|---|---|---|
| meter | 0–1 | the hourglass |
| blend | 0–1 | how far bullet time has faded in |
| efficiency | 0–1 | the "adrenaline" earned from kills in bullet time |
| active | bool | bullet time switched on |

## Each frame

1. **Kill bonus to the meter:** the meter goes up by the killed enemy's skin bonus
   (`SlowMotionBonusForKiller`) times the difficulty's `BulletTimeGainedPerKillMultiplier`,
   clamped to 1.
2. **Shootdodge:** the player may shootdodge while meter > `ShootDodgeThreshold`. A
   shootdodge made outside bullet time takes `ShootDodgePenalty` off the meter (0 in retail).
3. **When active:**
   - blend rises by `dt / InBlendTime`;
   - the meter drains by `dt / SlowMotionTime`, and at 0 bullet time ends (end messages play);
   - the heartbeat sound repeats every `HeartbeatSoundInterval`.
4. **When inactive:** blend falls by `dt / OutBlendTime`, and the meter refills by
   `dt / RegenerationTime`.
5. **Efficiency:**
   - On a kill, efficiency blends linearly from its current value to
     `min(1, current + EfficiencyIncreasePerKill)` over `EfficiencyIncreaseBlendTime`.
   - After that it holds for `EfficiencyDecreaseDelayAfterKill` seconds, then falls by
     `EfficiencyDecreaseSpeed` per second down to 0.
6. **Speeds:**
   - world speed = `blend · lerp(MinEfficiencyGameSpeed, MaxEfficiencyGameSpeed, efficiency) + (1 − blend)`
   - player time = `world speed · (blend · lerp(1, MaxEfficiencyPlayerTime, efficiency) + (1 − blend))`

   The player's own clock is the world's slowed clock multiplied by a factor that grows with
   efficiency. With no kills, Max slows down as much as everyone else (0.6). At full efficiency
   the world runs at 0.2 and Max at 0.2 × 3 = 0.6.

Shootdodge slow motion is separate. The shootdodge animations change the game speed
themselves through frame messages (`Gm_ChangeGameSpeed(0.2, 0.3)` at frame 10, back to
`(1.0, 0.5)` at frame 30 for the forward dive; see `skins/Male_Player.h`).

Still unknown: how mouse aiming relates to player time (MP2 aims in real time) and what the
second `Gm_ChangeGameSpeed` argument blends.
