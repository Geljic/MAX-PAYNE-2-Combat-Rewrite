//! Shared harness: a sim on a test world, stepped by input scripts, with an event log and a
//! trace of player states (printed when an `until` fails).
#![allow(dead_code)]

use payne::{Event, Input, Sim, State, TestWorld, Vec3};

pub const DT: f32 = Sim::DT;

pub struct T {
    pub w: TestWorld,
    pub sim: Sim,
    pub events: Vec<Event>,
    pub trace: Vec<String>,
    /// Real ticks stepped.
    pub ticks: usize,
}

pub fn flat() -> T {
    on(TestWorld::flat(), Vec3::ZERO)
}

pub fn on(w: TestWorld, spawn: Vec3) -> T {
    T {
        w,
        sim: Sim::new(spawn, 0.0),
        events: Vec::new(),
        trace: Vec::new(),
        ticks: 0,
    }
}

pub fn still() -> Input {
    Input::default()
}
pub fn mv(x: f32, y: f32) -> Input {
    Input {
        move_axis: [x, y],
        ..Default::default()
    }
}
pub fn fwd() -> Input {
    mv(0.0, 1.0)
}

impl T {
    pub fn step(&mut self, i: Input) {
        self.sim.step(&self.w, &i);
        self.ticks += 1;
        self.events.extend(self.sim.drain_events());
        let name = self.sim.player.state.name();
        if self.trace.last().is_none_or(|l| !l.starts_with(name)) {
            let p = self.sim.player.pos;
            self.trace.push(format!(
                "{name} @{:.2},{:.2},{:.2} t{}",
                p.x, p.y, p.z, self.ticks
            ));
        }
    }

    pub fn hold(&mut self, secs: f32, i: Input) {
        for _ in 0..(secs / DT).round() as usize {
            self.step(i);
        }
    }

    /// Step with `i` until `done`; returns real seconds taken. Panics with the trace after `max`.
    pub fn until(&mut self, max: f32, i: Input, done: impl Fn(&Sim) -> bool) -> f32 {
        for k in 0..(max / DT) as usize {
            if done(&self.sim) {
                return k as f32 * DT;
            }
            self.step(i);
        }
        panic!(
            "not reached in {max}s: state={:?} pos={:?}\ntrace: {}",
            self.sim.player.state,
            self.sim.player.pos,
            self.trace.join(" → ")
        );
    }

    /// Press a button for one tick (on top of `base`), then release it for one tick.
    pub fn tap(&mut self, base: Input, set: impl Fn(&mut Input)) {
        let mut i = base;
        set(&mut i);
        self.step(i);
        self.step(base);
    }

    pub fn saw(&self, state: &str) -> bool {
        self.trace.iter().any(|l| l.starts_with(state))
    }

    pub fn state(&self) -> State {
        self.sim.player.state
    }

    /// Make every weapon perfectly accurate (no spread, no speed jitter, no bloom).
    pub fn accurate(&mut self) {
        for w in &mut self.sim.weapons {
            w.spread_base = 0.0;
            w.spread_per_shot = 0.0;
            w.projectile_speed_random = 0.0;
            w.recoil = 0.0;
        }
    }

    /// Select inventory slot `i` and wait out the switch.
    pub fn select(&mut self, i: usize) {
        self.step(Input {
            select_weapon: Some(i),
            ..Default::default()
        });
        self.hold(
            0.5,
            Input {
                select_weapon: Some(i),
                ..Default::default()
            },
        );
    }

    /// Input aiming the camera ray straight from the right-hand muzzle at `p`.
    pub fn aim_at(&self, base: Input, p: Vec3) -> Input {
        let o = self.sim.player.muzzle(payne::Hand::Right, &self.sim.tuning);
        Input {
            camera_ray: Some((o, (p - o).normalize_or_zero())),
            ..base
        }
    }

    pub fn count(&self, f: impl Fn(&Event) -> bool) -> usize {
        self.events.iter().filter(|e| f(e)).count()
    }
}
