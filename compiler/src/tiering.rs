//! Decides what gets compiled and when, compiles it on other cores, and runs compiled code.
//!
//! trace: decision.backend.tiers
//! trace: decision.backend.two_strengths
//!
//! The interpreter counts each procedure's calls and each loop's rounds. Warm code goes to Cranelift's quick
//! strength, very hot code to its speed strength. A loop that gets hot is jumped into mid-loop (on-stack
//! replacement): compiled code entered at the loop's top picks up the frame as the interpreter left it.
//!
//! Compiling happens on other cores by default; `in_place` compiles on the interpreter's own thread, so the two can be
//! measured against each other. The forced modes compile every procedure before running, so every golden test runs
//! all of its code at that strength.

use crate::bytecode::Program;
use crate::interpreter::{Machine, Stopped};
use crate::ir::{self, Entry};
use crate::jit::{self, Answer, Compiled, Strength};
use crate::passes;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

/// Calls or loop rounds before a procedure is compiled at the quick strength; measured, see `REPORT`.
pub const WARM: u32 = 1_000;
/// Calls or loop rounds before it is compiled at the speed strength.
pub const HOT: u32 = 10_000;

struct Job {
    procedure: u32,
    entry: Entry,
    strength: Strength,
    /// A table of answers to build, rather than code.
    table: bool,
}

pub struct Tiers {
    program: Arc<Program>,
    /// Compiled code for each procedure's start, read by compiled code too, so its layout is `usize`s.
    pub entries: Vec<AtomicUsize>,
    /// Compiled code entered at each loop's top, the latest ready.
    loops: Vec<Vec<AtomicUsize>>,
    /// Speed code entered at each loop's top, which quick loop code moves into.
    speed_loops: Vec<Vec<AtomicUsize>>,
    calls: Vec<AtomicU32>,
    rounds: Vec<Vec<AtomicU32>>,
    /// What has been asked for, so nothing is asked twice: quick and speed, for the start and for each loop.
    asked: Mutex<std::collections::HashSet<(u32, Entry, Strength)>>,
    sender: Mutex<Option<Sender<Job>>>,
    in_place: bool,
    tiered: bool,
    /// The thresholds this run uses: `WARM` and `HOT`, or tiny ones for testing every move between tiers.
    pub warm: u32,
    pub hot: u32,
    /// A compile that failed; in a forced mode this is TAKL breaking.
    pub broken: AtomicBool,
    pub compiled: AtomicU32,
    pub compile_nanos: AtomicUsize,
    /// Each procedure's table of answers once built, as a pointer to its first entry; read by compiled code too.
    pub tables: Vec<AtomicUsize>,
    /// Which procedures get a table, and its shape.
    pub shapes: Vec<Option<crate::precompute::Shape>>,
    tables_asked: Mutex<std::collections::HashSet<u32>>,
    pub table_nanos: AtomicUsize,
    /// Times the interpreter jumped into a compiled loop.
    pub jumps_in: AtomicU32,
    pub started: std::time::Instant,
    /// When each entry became ready, from the start: for measuring compiling elsewhere against in place.
    pub ready: Mutex<Vec<(String, std::time::Duration)>>,
}

impl Tiers {
    fn new(program: &Arc<Program>, tiered: bool, in_place: bool, (warm, hot): (u32, u32)) -> Tiers {
        let count = program.procedures.len();
        Tiers {
            program: Arc::clone(program),
            entries: (0..count).map(|_| AtomicUsize::new(0)).collect(),
            loops: program
                .procedures
                .iter()
                .map(|procedure| procedure.loops.iter().map(|_| AtomicUsize::new(0)).collect())
                .collect(),
            speed_loops: program
                .procedures
                .iter()
                .map(|procedure| procedure.loops.iter().map(|_| AtomicUsize::new(0)).collect())
                .collect(),
            calls: (0..count).map(|_| AtomicU32::new(0)).collect(),
            rounds: program
                .procedures
                .iter()
                .map(|procedure| procedure.loops.iter().map(|_| AtomicU32::new(0)).collect())
                .collect(),
            asked: Mutex::new(std::collections::HashSet::new()),
            sender: Mutex::new(None),
            in_place,
            tiered,
            warm,
            hot,
            broken: AtomicBool::new(false),
            compiled: AtomicU32::new(0),
            compile_nanos: AtomicUsize::new(0),
            jumps_in: AtomicU32::new(0),
            tables: (0..count).map(|_| AtomicUsize::new(0)).collect(),
            shapes: (0..count as u32).map(|procedure| crate::precompute::eligible(program, procedure)).collect(),
            tables_asked: Mutex::new(std::collections::HashSet::new()),
            table_nanos: AtomicUsize::new(0),
            started: std::time::Instant::now(),
            ready: Mutex::new(Vec::new()),
        }
    }

    /// Compiles one entry of one procedure now, and installs it.
    fn compile_now(&self, procedure: u32, entry: Entry, strength: Strength) {
        let started = std::time::Instant::now();
        let promote = self.tiered && strength == Strength::Quick;
        let mut function = ir::build_with(&self.program, procedure, entry, promote);
        match strength {
            Strength::Quick => passes::quick(&mut function),
            Strength::Speed => passes::speed(&self.program, &mut function),
        }
        let addresses = jit::Addresses {
            entries: self.entries.as_ptr(),
            rounds: self.rounds[procedure as usize].as_ptr(),
            tables: self.tables.as_ptr(),
            shapes: &self.shapes,
            hot: self.hot,
        };
        match jit::compile(&self.program, &function, strength, &addresses) {
            Ok(code) => {
                match entry {
                    Entry::Start => self.entries[procedure as usize].store(code, Ordering::Release),
                    Entry::Loop(loop_id) => {
                        self.loops[procedure as usize][loop_id as usize].store(code, Ordering::Release);
                        if strength == Strength::Speed {
                            self.speed_loops[procedure as usize][loop_id as usize].store(code, Ordering::Release);
                        }
                    }
                }
                self.compiled.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut ready) = self.ready.lock() {
                    let name =
                        format!("{} {:?} {:?}", self.program.procedures[procedure as usize].name, entry, strength);
                    ready.push((name, self.started.elapsed()));
                }
            }
            Err(problem) => {
                if std::env::var("TAKL_DEBUG").is_ok() {
                    eprintln!("compiling {} failed: {}", self.program.procedures[procedure as usize].name, problem);
                }
                self.broken.store(true, Ordering::Relaxed);
            }
        }
        self.compile_nanos.fetch_add(started.elapsed().as_nanos() as usize, Ordering::Relaxed);
    }

    // trace: decision.compiler.precompute
    /// Builds a procedure's table of answers now, and publishes it.
    fn build_now(&self, procedure: u32) {
        let Some(shape) = self.shapes[procedure as usize].clone() else { return };
        let started = std::time::Instant::now();
        let table = crate::precompute::build(&self.program, procedure, shape);
        // The table lives as long as the program runs.
        let entries: &'static [crate::precompute::Entry] = Box::leak(table.entries.into_boxed_slice());
        self.tables[procedure as usize].store(entries.as_ptr() as usize, Ordering::Release);
        self.table_nanos.fetch_add(started.elapsed().as_nanos() as usize, Ordering::Relaxed);
        if let Ok(mut ready) = self.ready.lock() {
            ready.push((format!("{} table", self.program.procedures[procedure as usize].name), self.started.elapsed()));
        }
    }

    fn ask_table(self: &Arc<Self>, procedure: u32) {
        if self.shapes[procedure as usize].is_none() {
            return;
        }
        {
            let Ok(mut asked) = self.tables_asked.lock() else { return };
            if !asked.insert(procedure) {
                return;
            }
        }
        if self.in_place {
            self.build_now(procedure);
            return;
        }
        self.send(Job { procedure, entry: Entry::Start, strength: Strength::Speed, table: true });
    }

    /// Asks for an entry to be compiled, on another core unless compiling in place.
    fn ask(self: &Arc<Self>, procedure: u32, entry: Entry, strength: Strength) {
        {
            let Ok(mut asked) = self.asked.lock() else { return };
            if !asked.insert((procedure, entry, strength)) {
                return;
            }
        }
        if self.in_place {
            self.compile_now(procedure, entry, strength);
            return;
        }
        self.send(Job { procedure, entry, strength, table: false });
    }

    fn send(self: &Arc<Self>, job: Job) {
        let Ok(mut sender) = self.sender.lock() else { return };
        if sender.is_none() {
            let (send, receive) = channel::<Job>();
            let tiers = Arc::downgrade(self);
            let spawned = std::thread::Builder::new().name("takl compiler".to_string()).spawn(move || {
                while let Ok(job) = receive.recv() {
                    let Some(tiers) = tiers.upgrade() else { return };
                    if job.table {
                        tiers.build_now(job.procedure);
                    } else {
                        tiers.compile_now(job.procedure, job.entry, job.strength);
                    }
                }
            });
            if spawned.is_err() {
                return;
            }
            *sender = Some(send);
        }
        if let Some(sender) = sender.as_ref() {
            let _ = sender.send(job);
        }
    }

    /// Runs compiled code for a procedure when it is ready; counts the call when it isn't.
    #[inline(always)]
    pub fn call(&self, machine: &mut Machine, procedure: u32, base: usize) -> Option<Result<u64, Stopped>> {
        let table = self.tables[procedure as usize].load(Ordering::Acquire);
        if table != 0
            && let Some(value) = self.look_up(table, machine, procedure, base)
        {
            return Some(Ok(value));
        }
        let code = self.entries[procedure as usize].load(Ordering::Acquire);
        if code != 0 {
            if self.tiered {
                self.count_call(procedure);
            }
            return Some(run(code, machine, base));
        }
        if self.tiered {
            self.count_call(procedure);
        }
        None
    }

    // trace: decision.compiler.precompute
    /// A call's answer from its table, when it has one and the call wouldn't go too deep.
    fn look_up(&self, table: usize, machine: &mut Machine, procedure: u32, base: usize) -> Option<u64> {
        let shape = self.shapes[procedure as usize].as_ref()?;
        let frame = machine.frame_at(base);
        // SAFETY: the callee's frame holds its arguments in its first registers.
        let arguments: Vec<u64> = (0..shape.counts.len()).map(|index| unsafe { *frame.add(index) }).collect();
        let index = shape.index(&arguments)?;
        // SAFETY: `table` points at `shape.inputs()` entries, leaked so they live as long as the program.
        let entry = unsafe { *(table as *const crate::precompute::Entry).add(index) };
        if entry.meta & crate::precompute::HAS_ANSWER == 0 {
            return None;
        }
        // The caller is at `depth - 1`; the call's run would go `entry depth` deeper.
        if machine.depth - 1 + (entry.meta & 0xffff_ffff) > crate::interpreter::MOST_CALLS {
            return None;
        }
        Some(entry.value)
    }

    #[inline(always)]
    fn count_call(&self, procedure: u32) {
        let calls = self.calls[procedure as usize].fetch_add(1, Ordering::Relaxed) + 1;
        if calls == self.warm || calls == self.hot {
            self.warmed(procedure, calls);
        }
    }

    #[cold]
    fn warmed(&self, procedure: u32, calls: u32) {
        if let Some(tiers) = self.me() {
            let strength = if calls >= self.hot { Strength::Speed } else { Strength::Quick };
            tiers.ask(procedure, Entry::Start, strength);
            if strength == Strength::Speed {
                tiers.ask_table(procedure);
            }
        }
    }

    fn me(&self) -> Option<Arc<Tiers>> {
        ME.with(|me| me.borrow().as_ref().and_then(std::sync::Weak::upgrade))
    }

    /// Counts a loop's round; true when compiled code for the loop is ready to jump into.
    #[inline(always)]
    pub fn loop_is_hot(&self, procedure: u32, loop_id: u16) -> bool {
        if !self.tiered {
            return false;
        }
        let rounds = self.rounds[procedure as usize][loop_id as usize].fetch_add(1, Ordering::Relaxed) + 1;
        if rounds == self.warm || rounds == self.hot {
            self.loop_warmed(procedure, loop_id, rounds);
        }
        self.loops[procedure as usize][loop_id as usize].load(Ordering::Relaxed) != 0
    }

    #[cold]
    fn loop_warmed(&self, procedure: u32, loop_id: u16, rounds: u32) {
        if let Some(tiers) = self.me() {
            let strength = if rounds >= self.hot { Strength::Speed } else { Strength::Quick };
            tiers.ask(procedure, Entry::Loop(loop_id), strength);
            // The procedure itself is worth compiling for its next call.
            tiers.ask(procedure, Entry::Start, strength);
        }
    }

    // trace: decision.backend.bytecode
    /// Jumps into compiled code at a loop's top, which runs the rest of the procedure.
    pub fn jump_in(
        &self,
        machine: &mut Machine,
        procedure: u32,
        loop_id: u16,
        base: usize,
    ) -> Option<Result<u64, Stopped>> {
        let code = self.loops[procedure as usize][loop_id as usize].load(Ordering::Acquire);
        if code == 0 {
            return None;
        }
        self.jumps_in.fetch_add(1, Ordering::Relaxed);
        Some(run(code, machine, base))
    }

    /// Speed code for a loop, for quick code to move into; asks for it the first time.
    pub fn speed_loop(&self, procedure: u32, loop_id: u16) -> usize {
        let code = self.speed_loops[procedure as usize][loop_id as usize].load(Ordering::Acquire);
        if code == 0
            && let Some(tiers) = self.me()
        {
            tiers.ask(procedure, Entry::Loop(loop_id), Strength::Speed);
            tiers.ask(procedure, Entry::Start, Strength::Speed);
        }
        if code != 0 {
            self.jumps_in.fetch_add(1, Ordering::Relaxed);
        }
        code
    }

    /// What the tiers did, for `TAKL_STATS`.
    pub fn report(&self) -> String {
        let mut out = format!(
            "{} compiled, {:.2} ms compiling and {:.2} ms building tables {}, {} jumps into compiled loops, ran {:.2} ms\n",
            self.compiled.load(Ordering::Relaxed),
            self.compile_nanos.load(Ordering::Relaxed) as f64 / 1e6,
            self.table_nanos.load(Ordering::Relaxed) as f64 / 1e6,
            if self.in_place { "in place" } else { "on another core" },
            self.jumps_in.load(Ordering::Relaxed),
            self.started.elapsed().as_secs_f64() * 1e3,
        );
        if let Ok(ready) = self.ready.lock() {
            for (name, at) in ready.iter() {
                out.push_str(&format!("  ready at {:8.3} ms: {}\n", at.as_secs_f64() * 1e3, name));
            }
        }
        out
    }
}

thread_local! {
    static ME: std::cell::RefCell<Option<std::sync::Weak<Tiers>>> = const { std::cell::RefCell::new(None) };
}

fn run(code: usize, machine: &mut Machine, base: usize) -> Result<u64, Stopped> {
    // SAFETY: `code` is a function Cranelift made with the `Compiled` signature, and `base` is the frame it expects.
    let compiled: Compiled = unsafe { std::mem::transmute::<usize, Compiled>(code) };
    let frame = machine.frame_at(base);
    let answer: Answer = unsafe { compiled(machine as *mut Machine, frame) };
    if answer.status == 0 { Ok(answer.value) } else { Err(Stopped) }
}

/// Starts the tiers a mode asks for; the interpreter alone needs none.
pub fn start(program: &Arc<Program>, mode: crate::Mode) -> Option<Arc<Tiers>> {
    start_with(program, mode, std::env::var("TAKL_IN_PLACE").is_ok_and(|value| !value.is_empty()))
}

pub fn start_with(program: &Arc<Program>, mode: crate::Mode, in_place: bool) -> Option<Arc<Tiers>> {
    let (strength, in_place, thresholds) = match mode {
        crate::Mode::Interpret => return None,
        crate::Mode::Quick => (Some(Strength::Quick), in_place, (WARM, HOT)),
        crate::Mode::Speed => (Some(Strength::Speed), in_place, (WARM, HOT)),
        // `TAKL_WARM` and `TAKL_HOT` override the thresholds, for measuring them.
        crate::Mode::Tiered => (None, in_place, (measured("TAKL_WARM", WARM), measured("TAKL_HOT", HOT))),
        // Every move between tiers, on every program: compiled in place so it happens at the same round each run.
        crate::Mode::TieredEager => (None, true, (2, 4)),
    };
    let tiers = Arc::new(Tiers::new(program, strength.is_none(), in_place, thresholds));
    ME.with(|me| *me.borrow_mut() = Some(Arc::downgrade(&tiers)));
    if let Some(strength) = strength {
        // Forced: everything compiled before it runs, and at the speed strength every table built.
        if strength == Strength::Speed {
            for procedure in 0..program.procedures.len() as u32 {
                tiers.build_now(procedure);
            }
        }
        for procedure in 0..program.procedures.len() as u32 {
            tiers.compile_now(procedure, Entry::Start, strength);
        }
    }
    Some(tiers)
}

fn measured(name: &str, default: u32) -> u32 {
    std::env::var(name).ok().and_then(|value| value.parse().ok()).filter(|value| *value > 0).unwrap_or(default)
}
