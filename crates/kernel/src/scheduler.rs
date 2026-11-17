//! Original deterministic single-core reference scheduler. Guest time is a
//! model: one virtual ns per interpreted instruction, not a hardware frequency.
use crate::handles::ThreadObject;
use emulator_a64::Cpu;
use std::cell::Cell;
use std::rc::Rc;

pub const MAX_THREADS: usize = 256;
/// TLS pages of threads 0..LOW_TLS_THREADS sit below the config page; later
/// threads use a second arena near the top of the 39-bit address space.
pub const THREAD_TLS_ARENA: u64 = 0x7ffe_0000;
pub const LOW_TLS_THREADS: usize = 16;
pub const HIGH_TLS_ARENA: u64 = 0x7f_f000_0000;

/// Guest address of thread `id`'s 4 KiB TLS page.
pub fn thread_tls_page(id: usize) -> u64 {
    if id < LOW_TLS_THREADS {
        THREAD_TLS_ARENA + id as u64 * 4096
    } else {
        HIGH_TLS_ARENA + (id - LOW_TLS_THREADS) as u64 * 4096
    }
}

/// Whether `[address, end)` overlaps any TLS arena.
pub fn overlaps_tls(address: u64, end: u64) -> bool {
    let low_end = THREAD_TLS_ARENA + LOW_TLS_THREADS as u64 * 4096;
    let high_end = HIGH_TLS_ARENA + (MAX_THREADS - LOW_TLS_THREADS) as u64 * 4096;
    (address < low_end && THREAD_TLS_ARENA < end) || (address < high_end && HIGH_TLS_ARENA < end)
}
pub const THREAD_QUANTUM: u32 = 64;
const MAIN_PRIORITY: u8 = 44;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerError {
    ThreadLimit,
    ClockOverflow,
    UnsupportedSleep(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleOutcome {
    Ready,
    Blocked,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SchedulerStats {
    pub threads_created: usize,
    pub context_switches: u64,
    pub blocking_waits: u64,
    pub timeouts: u64,
    pub virtual_time_ns: u64,
    pub cancelled_waits: u64,
}

/// Diagnostic snapshot of one guest thread (saved context; the running
/// thread's live registers belong to the caller's CPU instead).
#[derive(Clone, Debug)]
pub struct ThreadSummary {
    pub id: usize,
    pub state: String,
    pub priority: u8,
    pub ideal_core: i32,
    pub core_mask: u64,
    pub pc: u64,
    pub lr: u64,
    pub fp: u64,
    pub args: [u64; 4],
}

#[derive(Clone, Debug)]
pub struct WaitTarget(pub Rc<Cell<bool>>);
impl WaitTarget {
    pub fn signaled(&self) -> bool {
        self.0.get()
    }
}

#[derive(Debug)]
enum State {
    Created,
    Runnable,
    Waiting {
        targets: Vec<WaitTarget>,
        deadline: Option<u64>,
    },
    Sleeping {
        deadline: u64,
    },
    Exited,
}
#[derive(Debug)]
struct Thread {
    object: ThreadObject,
    cpu: Cpu,
    priority: u8,
    ideal_core: i32,
    core_mask: u64,
    ready_order: u128,
    state: State,
    cancel_pending: bool,
}
#[derive(Debug)]
pub struct Scheduler {
    threads: Vec<Thread>,
    current: usize,
    active_core: u8,
    quantum_used: u32,
    yield_requested: bool,
    /// Set whenever thread state may have changed outside the running thread's
    /// instruction stream (SVCs, thread management). While clear and the quantum
    /// is not exhausted, `prepare` keeps the live CPU without a full reschedule.
    dirty: bool,
    /// Earliest Waiting/Sleeping deadline, refreshed on every full reschedule.
    next_deadline: u64,
    ready_order: u128,
    stats: SchedulerStats,
}
impl Scheduler {
    pub fn new(main: ThreadObject) -> Self {
        Self {
            threads: vec![Thread {
                object: main,
                cpu: Cpu::default(),
                priority: MAIN_PRIORITY,
                ideal_core: 0,
                core_mask: 7, // Cores 0, 1, 2
                ready_order: 0,
                state: State::Runnable,
                cancel_pending: false,
            }],
            current: 0,
            active_core: 0,
            quantum_used: 0,
            yield_requested: false,
            dirty: true,
            next_deadline: u64::MAX,
            ready_order: 0,
            stats: SchedulerStats {
                threads_created: 1,
                ..SchedulerStats::default()
            },
        }
    }
    pub fn next_thread(&self) -> Result<usize, SchedulerError> {
        if self.threads.len() >= MAX_THREADS {
            Err(SchedulerError::ThreadLimit)
        } else {
            Ok(self.threads.len())
        }
    }
    pub fn add(&mut self, object: ThreadObject, cpu: Cpu, priority: u8, ideal_core: i32) {
        self.dirty = true;
        let mask = if (0..3).contains(&ideal_core) {
            1u64 << ideal_core
        } else {
            7u64
        };
        self.threads.push(Thread {
            object,
            cpu,
            priority,
            ideal_core,
            core_mask: mask,
            ready_order: 0,
            state: State::Created,
            cancel_pending: false,
        });
        self.stats.threads_created += 1;
    }
    fn next_order(&mut self) -> u128 {
        // At most one enqueue per guest instruction, plus 16 simultaneous wakes.
        // Clock overflow is checked before guest execution can reach this bound.
        self.ready_order += 1;
        self.ready_order
    }
    pub fn start(&mut self, id: usize) -> bool {
        self.dirty = true;
        if !matches!(self.threads[id].state, State::Created) {
            return false;
        }
        let order = self.next_order();
        self.threads[id].ready_order = order;
        self.threads[id].state = State::Runnable;
        true
    }
    fn deadline(&self, ns: u64) -> Result<u64, SchedulerError> {
        self.stats
            .virtual_time_ns
            .checked_add(ns)
            .ok_or(SchedulerError::ClockOverflow)
    }
    pub fn wait(&mut self, targets: Vec<WaitTarget>, timeout: i64) -> Result<(), SchedulerError> {
        self.dirty = true;
        let deadline = if timeout < 0 {
            None
        } else {
            Some(self.deadline(timeout as u64)?)
        };
        self.threads[self.current].state = State::Waiting { targets, deadline };
        self.stats.blocking_waits += 1;
        Ok(())
    }
    pub fn take_cancellation(&mut self) -> bool {
        self.dirty = true;
        let pending = std::mem::take(&mut self.threads[self.current].cancel_pending);
        if pending {
            self.stats.cancelled_waits += 1;
        }
        pending
    }
    pub fn cancel_wait(&mut self, id: usize) {
        self.dirty = true;
        if matches!(self.threads[id].state, State::Waiting { .. }) {
            let order = self.next_order();
            let thread = &mut self.threads[id];
            // Replacing the wait drops retained objects and its deadline. It
            // cannot later turn an interrupted wait into a signal or timeout.
            thread.state = State::Runnable;
            thread.ready_order = order;
            thread.cpu.x[0] = crate::RESULT_INTERRUPTED;
            thread.cancel_pending = false;
            self.stats.cancelled_waits += 1;
        } else if !matches!(self.threads[id].state, State::Exited) {
            // Coalesce repeated requests. SleepThread is not a synchronization
            // wait, so cancellation remains pending without waking the sleeper.
            self.threads[id].cancel_pending = true;
        }
    }
    pub fn sleep(&mut self, ns: i64) -> Result<(), SchedulerError> {
        self.dirty = true;
        if ns == 0 {
            self.yield_requested = true;
        } else if ns == -1 || ns == -2 {
            // Yield with core migration / to any thread: also give up this
            // core's slice so other simulated cores get to run.
            self.yield_requested = true;
            self.quantum_used = THREAD_QUANTUM;
        } else if ns > 0 {
            self.threads[self.current].state = State::Sleeping {
                deadline: self.deadline(ns as u64)?,
            };
        } else {
            return Err(SchedulerError::UnsupportedSleep(ns));
        }
        Ok(())
    }
    pub fn exit_current(&mut self) -> bool {
        self.dirty = true;
        self.threads[self.current].state = State::Exited;
        self.threads[self.current].object.terminated.set(true);
        !self.threads.iter().any(|t| {
            matches!(
                t.state,
                State::Runnable | State::Waiting { .. } | State::Sleeping { .. }
            )
        })
    }
    pub fn terminate_all(&mut self) {
        self.dirty = true;
        for thread in &mut self.threads {
            thread.object.terminated.set(true);
            thread.state = State::Exited;
        }
    }
    pub fn tick(&mut self) -> Result<(), SchedulerError> {
        self.stats.virtual_time_ns = self.deadline(1)?;
        self.quantum_used = self.quantum_used.saturating_add(1);
        Ok(())
    }
    pub fn stats(&self) -> SchedulerStats {
        self.stats
    }
    pub fn thread_summaries(&self) -> Vec<ThreadSummary> {
        self.threads
            .iter()
            .enumerate()
            .map(|(id, t)| ThreadSummary {
                id,
                state: match &t.state {
                    State::Created => "created".into(),
                    State::Runnable => "runnable".into(),
                    State::Waiting { targets, deadline } => format!(
                        "waiting(targets={}, deadline={})",
                        targets.len(),
                        deadline.map_or("none".into(), |d| d.to_string())
                    ),
                    State::Sleeping { deadline } => format!("sleeping(until={deadline})"),
                    State::Exited => "exited".into(),
                },
                priority: t.priority,
                ideal_core: t.ideal_core,
                core_mask: t.core_mask,
                pc: t.cpu.pc,
                lr: t.cpu.x[30],
                fp: t.cpu.x[29],
                args: [t.cpu.x[0], t.cpu.x[1], t.cpu.x[2], t.cpu.x[3]],
            })
            .collect()
    }
    pub fn current(&self) -> usize {
        self.current
    }
    pub fn current_core(&self) -> u8 {
        self.active_core
    }
    pub fn set_thread_core_mask(&mut self, id: usize, ideal_core: i32, mask: u64) -> bool {
        self.dirty = true;
        if id < self.threads.len() {
            self.threads[id].ideal_core = ideal_core;
            self.threads[id].core_mask = if mask == 0 { 7 } else { mask };
            true
        } else {
            false
        }
    }
    pub fn get_thread_core_mask(&self, id: usize) -> Option<(i32, u64)> {
        self.threads.get(id).map(|t| (t.ideal_core, t.core_mask))
    }
    pub fn set_thread_priority(&mut self, id: usize, priority: u8) -> bool {
        self.dirty = true;
        if id < self.threads.len() && priority < 64 {
            self.threads[id].priority = priority;
            true
        } else {
            false
        }
    }
    pub fn get_thread_priority(&self, id: usize) -> Option<u8> {
        self.threads.get(id).map(|t| t.priority)
    }
    fn wake(&mut self) {
        for id in 0..self.threads.len() {
            let now = self.stats.virtual_time_ns;
            let (ready, result) = match &self.threads[id].state {
                State::Waiting { targets, deadline } => {
                    if let Some(index) = targets.iter().position(WaitTarget::signaled) {
                        (true, Some((0, Some(index as u64))))
                    } else if deadline.is_some_and(|d| d <= now) {
                        (true, Some((crate::RESULT_TIMEOUT, None)))
                    } else {
                        (false, None)
                    }
                }
                State::Sleeping { deadline } => (*deadline <= now, None),
                _ => (false, None),
            };
            if ready {
                let order = self.next_order();
                let thread = &mut self.threads[id];
                thread.state = State::Runnable;
                thread.ready_order = order;
                if let Some((result, index)) = result {
                    thread.cpu.x[0] = result;
                    if let Some(index) = index {
                        thread.cpu.x[1] = index;
                    } else {
                        self.stats.timeouts += 1;
                    }
                }
            }
        }
    }
    fn best(&mut self) -> Option<usize> {
        // Multi-core simulation: assign each of cores 0..2 a distinct runnable
        // thread (its own ideal-core queue first, then a migrated thread that
        // no other core holds), and run the active core's assignment. Quantum
        // expiry rotates the active core, so concurrently runnable threads all
        // progress as they would on hardware.
        let runnable = |t: &Thread| matches!(t.state, State::Runnable);
        let mut assigned: [Option<usize>; 3] = [None; 3];
        for (core, slot) in assigned.iter_mut().enumerate() {
            *slot = self
                .threads
                .iter()
                .enumerate()
                .filter(|(_, t)| runnable(t))
                .filter(|(_, t)| t.ideal_core == core as i32 && (t.core_mask & (1 << core)) != 0)
                .min_by_key(|(_, t)| (t.priority, t.ready_order))
                .map(|(id, _)| id);
        }
        for core in 0..3 {
            if assigned[core].is_some() {
                continue;
            }
            assigned[core] = self
                .threads
                .iter()
                .enumerate()
                .filter(|(id, t)| runnable(t) && !assigned.contains(&Some(*id)))
                .filter(|(_, t)| (t.core_mask & (1 << core)) != 0)
                .min_by_key(|(_, t)| (t.priority, t.ready_order))
                .map(|(id, _)| id);
        }
        for offset in 0..3 {
            let core = (self.active_core + offset) % 3;
            if let Some(id) = assigned[core as usize] {
                self.active_core = core;
                return Some(id);
            }
        }
        // Threads whose mask names no simulated core.
        self.threads
            .iter()
            .enumerate()
            .filter(|(_, t)| runnable(t))
            .min_by_key(|(_, t)| (t.priority, t.ready_order))
            .map(|(id, _)| id)
    }
    /// Forces the next `prepare` to run a full reschedule (wake checks and
    /// thread selection). The kernel calls this on every supervisor call.
    pub fn request_reschedule(&mut self) {
        self.dirty = true;
    }
    pub fn prepare(&mut self, cpu: &mut Cpu) -> ScheduleOutcome {
        // Fast path: nothing outside the current thread's instruction stream
        // changed and its quantum is not used up, so it keeps running with its
        // live registers. Timed wake-ups are checked at the next quantum
        // boundary, which bounds their latency to THREAD_QUANTUM instructions.
        if !self.dirty
            && !self.yield_requested
            && self.quantum_used < THREAD_QUANTUM
            && self.stats.virtual_time_ns < self.next_deadline
            && matches!(self.threads[self.current].state, State::Runnable)
        {
            return ScheduleOutcome::Ready;
        }
        self.dirty = false;
        self.threads[self.current].cpu = cpu.clone();
        self.wake();
        let quantum_expired = self.quantum_used >= THREAD_QUANTUM;
        if self.yield_requested || quantum_expired {
            if matches!(self.threads[self.current].state, State::Runnable) {
                let order = self.next_order();
                self.threads[self.current].ready_order = order;
            }
            self.quantum_used = 0;
            self.yield_requested = false;
            // Time-slice the simulated cores: each of the three cores gets a
            // quantum in turn (a yield stays on its core). Without this, a busy thread whose ideal core is
            // the active core starves every thread pinned to the other cores,
            // which on hardware would run concurrently.
            if quantum_expired {
                self.active_core = (self.active_core + 1) % 3;
            }
        }
        let mut best = self.best();
        if best.is_none() {
            let deadline = self
                .threads
                .iter()
                .filter_map(|t| match &t.state {
                    State::Waiting { deadline, .. } => *deadline,
                    State::Sleeping { deadline } => Some(*deadline),
                    _ => None,
                })
                .min();
            if let Some(deadline) = deadline {
                self.stats.virtual_time_ns = deadline;
                self.wake();
                best = self.best();
            }
        }
        self.next_deadline = self
            .threads
            .iter()
            .filter_map(|t| match &t.state {
                State::Waiting { deadline, .. } => *deadline,
                State::Sleeping { deadline } => Some(*deadline),
                _ => None,
            })
            .min()
            .unwrap_or(u64::MAX);
        let Some(next) = best else {
            self.dirty = true;
            return ScheduleOutcome::Blocked;
        };
        let previous = self.current;
        if next != self.current {
            self.stats.context_switches += 1;
            self.quantum_used = 0;
            self.current = next;
        }
        *cpu = self.threads[next].cpu.clone();
        // A thread switch is an exception return on hardware, which clears the
        // local exclusive monitor; a preempted LDXR/STXR pair must retry.
        if next != previous {
            cpu.clear_exclusive();
        }
        ScheduleOutcome::Ready
    }
}
