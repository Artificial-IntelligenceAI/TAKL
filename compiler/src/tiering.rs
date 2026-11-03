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

const NONE: u8 = 0;
const ASKED: u8 = 1;

struct Job {
    procedure: u32,
    entry: Entry,
    strength: Strength,
}

pub struct Tiers {
    program: Arc<Program>,
    /// Compiled code for each procedure's start, read by compiled code too, so its layout is `usize`s.
    pub entries: Vec<AtomicUsize>,
    /// Compiled code entered at each loop's top.
    loops: Vec<Vec<AtomicUsize>>,
    calls: Vec<AtomicU32>,
    rounds: Vec<Vec<AtomicU32>>,
    /// What has been asked for, so nothing is asked twice: quick and speed, for the start and for each loop.
    asked: Mutex<std::collections::HashSet<(u32, Entry, Strength)>>,
    sender: Mutex<Option<Sender<Job>>>,
    in_place: bool,
    tiered: bool,
    /// A compile that failed; in a forced mode this is TAKL breaking.
    pub broken: AtomicBool,
    pub compiled: AtomicU32,
    pub compile_nanos: AtomicUsize,
    _state: u8,
}

impl Tiers {
    fn new(program: &Arc<Program>, tiered: bool, in_place: bool) -> Tiers {
        let count = program.procedures.len();
        Tiers {
            program: Arc::clone(program),
            entries: (0..count).map(|_| AtomicUsize::new(0)).collect(),
            loops: program.procedures.iter().map(|procedure| procedure.loops.iter().map(|_| AtomicUsize::new(0)).collect()).collect(),
            calls: (0..count).map(|_| AtomicU32::new(0)).collect(),
            rounds: program.procedures.iter().map(|procedure| procedure.loops.iter().map(|_| AtomicU32::new(0)).collect()).collect(),
            asked: Mutex::new(std::collections::HashSet::new()),
            sender: Mutex::new(None),
            in_place,
            tiered,
            broken: AtomicBool::new(false),
            compiled: AtomicU32::new(0),
            compile_nanos: AtomicUsize::new(0),
            _state: NONE,
        }
    }

    /// Compiles one entry of one procedure now, and installs it.
    fn compile_now(&self, procedure: u32, entry: Entry, strength: Strength) {
        let started = std::time::Instant::now();
        let mut function = ir::build(&self.program, procedure, entry);
        match strength {
            Strength::Quick => passes::quick(&mut function),
            Strength::Speed => passes::speed(&self.program, &mut function),
        }
        match jit::compile(&self.program, &function, strength, self.entries.as_ptr()) {
            Ok(code) => {
                match entry {
                    Entry::Start => self.entries[procedure as usize].store(code, Ordering::Release),
                    Entry::Loop(loop_id) => self.loops[procedure as usize][loop_id as usize].store(code, Ordering::Release),
                }
                self.compiled.fetch_add(1, Ordering::Relaxed);
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
        let Ok(mut sender) = self.sender.lock() else { return };
        if sender.is_none() {
            let (send, receive) = channel::<Job>();
            let tiers = Arc::downgrade(self);
            let spawned = std::thread::Builder::new().name("takl compiler".to_string()).spawn(move || {
                while let Ok(job) = receive.recv() {
                    let Some(tiers) = tiers.upgrade() else { return };
                    tiers.compile_now(job.procedure, job.entry, job.strength);
                }
            });
            if spawned.is_err() {
                return;
            }
            *sender = Some(send);
        }
        if let Some(sender) = sender.as_ref() {
            let _ = sender.send(Job { procedure, entry, strength });
        }
    }

    /// Runs compiled code for a procedure when it is ready; counts the call when it isn't.
    #[inline(always)]
    pub fn call(&self, machine: &mut Machine, procedure: u32, base: usize) -> Option<Result<u64, Stopped>> {
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

    #[inline(always)]
    fn count_call(&self, procedure: u32) {
        let calls = self.calls[procedure as usize].fetch_add(1, Ordering::Relaxed) + 1;
        if calls == WARM || calls == HOT {
            self.warmed(procedure, calls);
        }
    }

    #[cold]
    fn warmed(&self, procedure: u32, calls: u32) {
        if let Some(tiers) = self.me() {
            let strength = if calls >= HOT { Strength::Speed } else { Strength::Quick };
            tiers.ask(procedure, Entry::Start, strength);
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
        if rounds == WARM || rounds == HOT {
            self.loop_warmed(procedure, loop_id, rounds);
        }
        self.loops[procedure as usize][loop_id as usize].load(Ordering::Relaxed) != 0
    }

    #[cold]
    fn loop_warmed(&self, procedure: u32, loop_id: u16, rounds: u32) {
        if let Some(tiers) = self.me() {
            let strength = if rounds >= HOT { Strength::Speed } else { Strength::Quick };
            tiers.ask(procedure, Entry::Loop(loop_id), strength);
            // The procedure itself is worth compiling for its next call.
            tiers.ask(procedure, Entry::Start, strength);
        }
    }

    // trace: decision.backend.bytecode
    /// Jumps into compiled code at a loop's top, which runs the rest of the procedure.
    pub fn jump_in(&self, machine: &mut Machine, procedure: u32, loop_id: u16, base: usize) -> Option<Result<u64, Stopped>> {
        let code = self.loops[procedure as usize][loop_id as usize].load(Ordering::Acquire);
        if code == 0 {
            return None;
        }
        Some(run(code, machine, base))
    }

    /// Waits for every compile asked for; for measuring.
    pub fn compiled_count(&self) -> u32 {
        self.compiled.load(Ordering::Relaxed)
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
    start_with(program, mode, std::env::var("TAKL_IN_PLACE").is_ok())
}

pub fn start_with(program: &Arc<Program>, mode: crate::Mode, in_place: bool) -> Option<Arc<Tiers>> {
    let strength = match mode {
        crate::Mode::Interpret => return None,
        crate::Mode::Quick => Some(Strength::Quick),
        crate::Mode::Speed => Some(Strength::Speed),
        crate::Mode::Tiered => None,
    };
    let tiers = Arc::new(Tiers::new(program, strength.is_none(), in_place));
    ME.with(|me| *me.borrow_mut() = Some(Arc::downgrade(&tiers)));
    if let Some(strength) = strength {
        // Forced: everything compiled before it runs.
        for procedure in 0..program.procedures.len() as u32 {
            tiers.compile_now(procedure, Entry::Start, strength);
        }
    }
    Some(tiers)
}
