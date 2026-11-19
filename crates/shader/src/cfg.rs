//! Control-flow graph of a decoded Maxwell program.
//!
//! Splits a [`Listing`] into basic blocks and resolves every terminator,
//! including SYNC, whose target comes from the SSY reconvergence stack. The
//! stack depth and contents are tracked per block; a block reached with two
//! different stacks, a SYNC with an empty stack, or a branch outside the code
//! is an explicit error. Nothing here assigns semantics to ALU instructions.

use std::collections::{BTreeMap, BTreeSet};

use crate::maxwell::{Decoded, Instr, Listing, Pred};

/// Maximum SSY nesting accepted.
pub const MAX_SSY_DEPTH: usize = 16;
/// Condition code "always" in branch / exit / sync encodings.
const CC_ALWAYS: u8 = 0xf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terminator {
    /// Falls into the next block (the next block starts at a leader).
    Fallthrough(usize),
    Jump(usize),
    /// Guarded branch: taken when `guard` holds.
    Branch {
        guard: Pred,
        taken: usize,
        not_taken: usize,
    },
    Exit,
    /// Guarded EXIT: exits when `guard` holds, else continues.
    ExitIf {
        guard: Pred,
        not_taken: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// Byte offset of the first instruction.
    pub start: usize,
    /// Indices into `Listing::instructions` (terminator included).
    pub instructions: std::ops::Range<usize>,
    pub terminator: Terminator,
    /// SSY stack on entry (reconvergence offsets, innermost last).
    pub ssy_stack: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CfgError {
    /// The listing holds an undecodable word.
    Undecoded {
        offset: usize,
        word: u64,
    },
    EmptyProgram,
    /// Branch / SSY target not at a decoded instruction.
    BadTarget {
        at: usize,
        target: usize,
    },
    SyncWithoutSsy {
        at: usize,
    },
    SsyTooDeep {
        at: usize,
    },
    /// Block reached with two different SSY stacks.
    StackMismatch {
        block: usize,
    },
    /// Condition-code-tested control flow is not modelled.
    ConditionCode {
        at: usize,
        cc: u8,
    },
    /// Execution runs past the last decoded instruction.
    FallsOffEnd {
        at: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cfg {
    /// Reachable blocks keyed by start offset (entry is the first key).
    pub blocks: BTreeMap<usize, Block>,
}

impl Cfg {
    pub fn entry(&self) -> usize {
        *self.blocks.keys().next().expect("non-empty")
    }
    pub fn successors(&self, start: usize) -> Vec<usize> {
        match self.blocks[&start].terminator {
            Terminator::Fallthrough(n) | Terminator::Jump(n) => vec![n],
            Terminator::Branch {
                taken, not_taken, ..
            } => vec![taken, not_taken],
            Terminator::Exit => vec![],
            Terminator::ExitIf { not_taken, .. } => vec![not_taken],
        }
    }
}

fn target(at: usize, offset: i32) -> Option<usize> {
    (at as i64 + 8 + offset as i64).try_into().ok()
}

/// Build the graph of blocks reachable from the first instruction.
pub fn build(listing: &Listing) -> Result<Cfg, CfgError> {
    if let Some((offset, word)) = listing.unsupported {
        return Err(CfgError::Undecoded { offset, word });
    }
    let code = &listing.instructions;
    if code.is_empty() {
        return Err(CfgError::EmptyProgram);
    }
    let index: BTreeMap<usize, usize> =
        code.iter().enumerate().map(|(i, (o, _))| (*o, i)).collect();
    let resolve = |at: usize, offset: i32| -> Result<usize, CfgError> {
        let t = target(at, offset).ok_or(CfgError::BadTarget { at, target: 0 })?;
        index
            .get(&t)
            .copied()
            .ok_or(CfgError::BadTarget { at, target: t })
    };
    // Leaders: entry, targets, and the instruction after any control transfer.
    let mut leaders = BTreeSet::from([0usize]);
    for (i, (at, d)) in code.iter().enumerate() {
        let transfer = match d.instr {
            Instr::Bra { offset, .. } => {
                leaders.insert(resolve(*at, offset)?);
                true
            }
            Instr::Ssy { offset } => {
                leaders.insert(resolve(*at, offset)?);
                false
            }
            Instr::Exit { .. } | Instr::Sync { .. } => true,
            _ => false,
        };
        if transfer && i + 1 < code.len() {
            leaders.insert(i + 1);
        }
    }
    let leader_list: Vec<usize> = leaders.iter().copied().collect();
    let block_end = |first: usize| -> usize {
        leader_list
            .iter()
            .copied()
            .find(|&l| l > first)
            .unwrap_or(code.len())
    };

    let mut blocks = BTreeMap::new();
    let mut work: Vec<(usize, Vec<usize>)> = vec![(0, Vec::new())];
    while let Some((first, stack)) = work.pop() {
        let start = code[first].0;
        if let Some(existing) = blocks.get(&start) {
            let existing: &Block = existing;
            if existing.ssy_stack != stack {
                return Err(CfgError::StackMismatch { block: start });
            }
            continue;
        }
        let end = block_end(first);
        let mut inner = stack.clone();
        // SSY pushes take effect inside the block.
        for (at, d) in &code[first..end] {
            if let Instr::Ssy { offset } = d.instr {
                if inner.len() >= MAX_SSY_DEPTH {
                    return Err(CfgError::SsyTooDeep { at: *at });
                }
                inner.push(code[resolve(*at, offset)?].0);
            }
        }
        let (at, last): (usize, Decoded) = code[end - 1];
        let next = |inner: &Vec<usize>| -> Result<(usize, Vec<usize>), CfgError> {
            if end < code.len() {
                Ok((end, inner.clone()))
            } else {
                Err(CfgError::FallsOffEnd { at })
            }
        };
        let off = |i: usize| code[i].0;
        let mut successors: Vec<(usize, Vec<usize>)> = Vec::new();
        if let Instr::Bra { cc, .. } | Instr::Exit { cc } | Instr::Sync { cc } = last.instr {
            if cc != CC_ALWAYS {
                return Err(CfgError::ConditionCode { at, cc });
            }
        }
        let terminator = match last.instr {
            Instr::Bra { offset, .. } => {
                let t = resolve(at, offset)?;
                if last.guard.is_always() {
                    successors.push((t, inner.clone()));
                    Terminator::Jump(off(t))
                } else {
                    let n = next(&inner)?;
                    let not_taken = off(n.0);
                    successors.push((t, inner.clone()));
                    successors.push(n);
                    Terminator::Branch {
                        guard: last.guard,
                        taken: off(t),
                        not_taken,
                    }
                }
            }
            Instr::Exit { .. } => {
                if last.guard.is_always() {
                    Terminator::Exit
                } else {
                    let n = next(&inner)?;
                    let not_taken = off(n.0);
                    successors.push(n);
                    Terminator::ExitIf {
                        guard: last.guard,
                        not_taken,
                    }
                }
            }
            Instr::Sync { .. } => {
                let mut popped = inner.clone();
                let t = popped.pop().ok_or(CfgError::SyncWithoutSsy { at })?;
                let t_index = index[&t];
                if last.guard.is_always() {
                    successors.push((t_index, popped));
                    Terminator::Jump(t)
                } else {
                    let n = next(&inner)?;
                    let not_taken = off(n.0);
                    successors.push((t_index, popped));
                    successors.push(n);
                    Terminator::Branch {
                        guard: last.guard,
                        taken: t,
                        not_taken,
                    }
                }
            }
            _ => {
                let n = next(&inner)?;
                let t = off(n.0);
                successors.push(n);
                Terminator::Fallthrough(t)
            }
        };
        blocks.insert(
            start,
            Block {
                start,
                instructions: first..end,
                terminator,
                ssy_stack: stack,
            },
        );
        work.extend(successors);
    }
    Ok(Cfg { blocks })
}
