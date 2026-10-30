//! Original deterministic single-core reference scheduler. Guest time is a
//! model: one virtual ns per interpreted instruction, not a hardware frequency.
use crate::handles::ThreadObject;
use emulator_a64::Cpu;
use std::cell::Cell;
use std::rc::Rc;

pub const MAX_THREADS: usize = 16;
pub const THREAD_TLS_ARENA: u64 = 0x7ffe_0000;
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
    ready_order: u128,
    state: State,
    cancel_pending: bool,
}
#[derive(Debug)]
pub struct Scheduler {
    threads: Vec<Thread>,
    current: usize,
    quantum_used: u32,
    yield_requested: bool,
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
                ready_order: 0,
                state: State::Runnable,
                cancel_pending: false,
            }],
            current: 0,
            quantum_used: 0,
            yield_requested: false,
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
    pub fn add(&mut self, object: ThreadObject, cpu: Cpu, priority: u8) {
        self.threads.push(Thread {
            object,
            cpu,
            priority,
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
        let pending = std::mem::take(&mut self.threads[self.current].cancel_pending);
        if pending {
            self.stats.cancelled_waits += 1;
        }
        pending
    }
    pub fn cancel_wait(&mut self, id: usize) {
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
        if ns == 0 {
            self.yield_requested = true;
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
    pub fn current(&self) -> usize {
        self.current
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
    fn best(&self) -> Option<usize> {
        self.threads
            .iter()
            .enumerate()
            .filter(|(_, t)| matches!(t.state, State::Runnable))
            .min_by_key(|(_, t)| (t.priority, t.ready_order))
            .map(|(id, _)| id)
    }
    pub fn prepare(&mut self, cpu: &mut Cpu) -> ScheduleOutcome {
        self.threads[self.current].cpu = cpu.clone();
        self.wake();
        if self.yield_requested || self.quantum_used >= THREAD_QUANTUM {
            if matches!(self.threads[self.current].state, State::Runnable) {
                let order = self.next_order();
                self.threads[self.current].ready_order = order;
            }
            self.quantum_used = 0;
            self.yield_requested = false;
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
        let Some(next) = best else {
            return ScheduleOutcome::Blocked;
        };
        if next != self.current {
            self.stats.context_switches += 1;
            self.quantum_used = 0;
            self.current = next;
        }
        *cpu = self.threads[next].cpu.clone();
        ScheduleOutcome::Ready
    }
}
