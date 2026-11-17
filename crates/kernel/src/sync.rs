//! Original monotonic ticket futex arbitration and condition variable synchronization.

use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct Waiter {
    pub thread_index: usize,
    pub tag: u32,
    pub ticket: u64,
    pub signal: Rc<Cell<bool>>,
    pub mutex_addr: u64,
}

#[derive(Debug, Default)]
pub struct FutexArbitrator {
    next_ticket: u64,
    condvar_queues: BTreeMap<u64, VecDeque<Waiter>>,
    mutex_queues: BTreeMap<u64, VecDeque<Waiter>>,
    address_queues: BTreeMap<u64, VecDeque<Waiter>>,
}

impl FutexArbitrator {
    pub fn wait_condition(
        &mut self,
        thread_index: usize,
        tag: u32,
        key_addr: u64,
        mutex_addr: u64,
    ) -> (Rc<Cell<bool>>, u64) {
        let ticket = self.next_ticket;
        self.next_ticket = self.next_ticket.wrapping_add(1);
        let signal = Rc::new(Cell::new(false));
        let waiter = Waiter {
            thread_index,
            tag,
            ticket,
            signal: Rc::clone(&signal),
            mutex_addr,
        };
        self.condvar_queues
            .entry(key_addr)
            .or_default()
            .push_back(waiter);
        (signal, ticket)
    }

    pub fn signal_condition(&mut self, key_addr: u64, count: i32) -> Vec<Waiter> {
        let mut woken = Vec::new();
        if let Some(queue) = self.condvar_queues.get_mut(&key_addr) {
            let to_wake = if count <= 0 {
                queue.len()
            } else {
                (count as usize).min(queue.len())
            };
            for _ in 0..to_wake {
                if let Some(waiter) = queue.pop_front() {
                    woken.push(waiter);
                }
            }
            if queue.is_empty() {
                self.condvar_queues.remove(&key_addr);
            }
        }
        woken
    }

    pub fn requeue_to_mutex(&mut self, waiter: Waiter) {
        self.mutex_queues
            .entry(waiter.mutex_addr)
            .or_default()
            .push_back(waiter);
    }

    pub fn lock_mutex(
        &mut self,
        thread_index: usize,
        tag: u32,
        mutex_addr: u64,
    ) -> (Rc<Cell<bool>>, u64) {
        let ticket = self.next_ticket;
        self.next_ticket = self.next_ticket.wrapping_add(1);
        let signal = Rc::new(Cell::new(false));
        let waiter = Waiter {
            thread_index,
            tag,
            ticket,
            signal: Rc::clone(&signal),
            mutex_addr,
        };
        self.mutex_queues
            .entry(mutex_addr)
            .or_default()
            .push_back(waiter);
        (signal, ticket)
    }

    pub fn unlock_mutex(&mut self, mutex_addr: u64) -> Option<Waiter> {
        if let Some(queue) = self.mutex_queues.get_mut(&mutex_addr) {
            let waiter = queue.pop_front();
            if let Some(ref w) = waiter {
                w.signal.set(true);
            }
            if queue.is_empty() {
                self.mutex_queues.remove(&mutex_addr);
            }
            waiter
        } else {
            None
        }
    }

    pub fn has_mutex_waiters(&self, mutex_addr: u64) -> bool {
        self.mutex_queues
            .get(&mutex_addr)
            .is_some_and(|q| !q.is_empty())
    }

    pub fn condvar_waiter_count(&self, key_addr: u64) -> usize {
        self.condvar_queues
            .get(&key_addr)
            .map_or(0, |q| q.len())
    }

    /// Queues the current thread on an address arbiter (svcWaitForAddress).
    pub fn wait_address(&mut self, thread_index: usize, addr: u64) -> Rc<Cell<bool>> {
        let ticket = self.next_ticket;
        self.next_ticket = self.next_ticket.wrapping_add(1);
        let signal = Rc::new(Cell::new(false));
        self.address_queues.entry(addr).or_default().push_back(Waiter {
            thread_index,
            tag: 0,
            ticket,
            signal: Rc::clone(&signal),
            mutex_addr: addr,
        });
        signal
    }

    /// Drops waiters whose wait already ended (timeout or cancellation): the
    /// scheduler released its clone of the signal, so only ours remains.
    fn prune_address(&mut self, addr: u64) {
        if let Some(queue) = self.address_queues.get_mut(&addr) {
            queue.retain(|w| Rc::strong_count(&w.signal) > 1 && !w.signal.get());
            if queue.is_empty() {
                self.address_queues.remove(&addr);
            }
        }
    }

    /// Number of threads still blocked on `addr`.
    pub fn address_waiter_count(&mut self, addr: u64) -> usize {
        self.prune_address(addr);
        self.address_queues.get(&addr).map_or(0, VecDeque::len)
    }

    /// Wakes up to `count` waiters in FIFO order (`count <= 0` wakes all).
    pub fn signal_address(&mut self, addr: u64, count: i32) -> usize {
        self.prune_address(addr);
        let Some(queue) = self.address_queues.get_mut(&addr) else {
            return 0;
        };
        let n = if count <= 0 {
            queue.len()
        } else {
            (count as usize).min(queue.len())
        };
        for waiter in queue.drain(..n) {
            waiter.signal.set(true);
        }
        if queue.is_empty() {
            self.address_queues.remove(&addr);
        }
        n
    }
}
