//! Kernel-owned timed signals. Services arm one-shot or periodic timers on
//! their event signals (vsync, audio buffer completion); the scheduler fires
//! them as guest virtual time passes, including while every thread is
//! blocked. A timer holds its signal weakly and disappears with the event.
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// Most live timers at once; arming beyond this fails.
pub const MAX_TIMERS: usize = 1024;
/// Shortest period accepted for a periodic timer.
pub const MIN_TIMER_PERIOD_NS: u64 = 100_000;

#[derive(Debug)]
struct Timer {
    signal: Weak<Cell<bool>>,
    due: u64,
    period: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerError {
    /// [`MAX_TIMERS`] live timers already exist.
    Capacity,
    /// Periodic timers need a period of at least [`MIN_TIMER_PERIOD_NS`].
    Period,
}

#[derive(Debug, Default)]
pub struct TimerQueue {
    timers: RefCell<Vec<Timer>>,
}

impl TimerQueue {
    /// Signal `signal` once guest time reaches `due_ns`.
    pub fn arm_once(&self, signal: &Rc<Cell<bool>>, due_ns: u64) -> Result<(), TimerError> {
        self.push(Timer {
            signal: Rc::downgrade(signal),
            due: due_ns,
            period: None,
        })
    }

    /// Signal `signal` at `first_ns` and then every `period_ns`.
    pub fn arm_periodic(
        &self,
        signal: &Rc<Cell<bool>>,
        first_ns: u64,
        period_ns: u64,
    ) -> Result<(), TimerError> {
        if period_ns < MIN_TIMER_PERIOD_NS {
            return Err(TimerError::Period);
        }
        self.push(Timer {
            signal: Rc::downgrade(signal),
            due: first_ns,
            period: Some(period_ns),
        })
    }

    /// Remove every timer armed on `signal`.
    pub fn cancel(&self, signal: &Rc<Cell<bool>>) {
        let target = Rc::downgrade(signal);
        self.timers
            .borrow_mut()
            .retain(|timer| !Weak::ptr_eq(&timer.signal, &target));
    }

    /// Whether a timer is armed on `signal`.
    pub fn is_armed(&self, signal: &Rc<Cell<bool>>) -> bool {
        let target = Rc::downgrade(signal);
        self.timers
            .borrow()
            .iter()
            .any(|timer| Weak::ptr_eq(&timer.signal, &target))
    }

    /// Live timers (those whose signal still exists).
    pub fn len(&self) -> usize {
        self.prune();
        self.timers.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn push(&self, timer: Timer) -> Result<(), TimerError> {
        self.prune();
        let mut timers = self.timers.borrow_mut();
        if timers.len() >= MAX_TIMERS {
            return Err(TimerError::Capacity);
        }
        timers.push(timer);
        Ok(())
    }

    fn prune(&self) {
        self.timers
            .borrow_mut()
            .retain(|timer| timer.signal.strong_count() != 0);
    }

    /// Earliest due time of a live timer.
    pub(crate) fn next_due(&self) -> Option<u64> {
        self.prune();
        self.timers.borrow().iter().map(|timer| timer.due).min()
    }

    /// Signal every timer due at `now`. One-shot timers are removed; periodic
    /// ones move to their next period after `now` (missed periods collapse
    /// into this one signal, like an event that was already set). Returns how
    /// many timers fired.
    pub(crate) fn fire(&self, now: u64) -> usize {
        let mut fired = 0;
        self.timers.borrow_mut().retain_mut(|timer| {
            let Some(signal) = timer.signal.upgrade() else {
                return false;
            };
            if timer.due > now {
                return true;
            }
            signal.set(true);
            fired += 1;
            match timer.period {
                Some(period) => {
                    let missed = (now - timer.due) / period + 1;
                    timer.due = timer.due.saturating_add(missed.saturating_mul(period));
                    true
                }
                None => false,
            }
        });
        fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_and_periodic_timers_fire_and_reschedule() {
        let queue = TimerQueue::default();
        let once = Rc::new(Cell::new(false));
        let tick = Rc::new(Cell::new(false));
        queue.arm_once(&once, 500).unwrap();
        queue.arm_periodic(&tick, 1_000_000, 1_000_000).unwrap();
        assert_eq!(queue.next_due(), Some(500));
        assert_eq!(queue.fire(499), 0);
        assert!(!once.get());
        assert_eq!(queue.fire(500), 1);
        assert!(once.get());
        assert_eq!(queue.len(), 1, "one-shot removed");
        assert_eq!(queue.fire(1_000_000), 1);
        assert!(tick.get());
        assert_eq!(queue.next_due(), Some(2_000_000));
        // Missed periods collapse into one firing.
        tick.set(false);
        assert_eq!(queue.fire(5_500_000), 1);
        assert!(tick.get());
        assert_eq!(queue.next_due(), Some(6_000_000));
    }

    #[test]
    fn timers_die_with_their_signal_and_can_be_cancelled() {
        let queue = TimerQueue::default();
        let kept = Rc::new(Cell::new(false));
        {
            let dropped = Rc::new(Cell::new(false));
            queue.arm_periodic(&dropped, 10, 1_000_000).unwrap();
        }
        queue.arm_periodic(&kept, 20, 1_000_000).unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.next_due(), Some(20));
        assert!(queue.is_armed(&kept));
        queue.cancel(&kept);
        assert!(queue.is_empty());
        assert_eq!(queue.next_due(), None);
        assert_eq!(queue.fire(u64::MAX), 0);
        assert!(!kept.get());
    }

    #[test]
    fn capacity_and_period_are_bounded() {
        let queue = TimerQueue::default();
        let signal = Rc::new(Cell::new(false));
        assert_eq!(
            queue.arm_periodic(&signal, 0, MIN_TIMER_PERIOD_NS - 1),
            Err(TimerError::Period)
        );
        for _ in 0..MAX_TIMERS {
            queue.arm_once(&signal, 1).unwrap();
        }
        assert_eq!(queue.arm_once(&signal, 1), Err(TimerError::Capacity));
        assert_eq!(queue.fire(1), MAX_TIMERS);
        queue.arm_once(&signal, 2).unwrap();
    }
}
