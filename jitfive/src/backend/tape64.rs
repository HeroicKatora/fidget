use crate::{
    backend::{
        common::{Choice, NodeIndex, Op, VarIndex},
        dynasm::AsmOp,
    },
    op::{BinaryChoiceOpcode, BinaryOpcode, UnaryOpcode},
    scheduled::Scheduled,
    util::indexed::IndexMap,
};

use std::collections::BTreeMap;

#[derive(Copy, Clone, Debug)]
pub enum ClauseOp64 {
    /// Reads one of the inputs (X, Y, Z)
    Input,
    /// Copy an immediate to a register
    CopyImm,

    /// Negates a register
    NegReg,
    /// Takes the absolute value of a register
    AbsReg,
    /// Takes the reciprocal of a register
    RecipReg,
    /// Takes the square root of a register
    SqrtReg,
    /// Squares a register
    SquareReg,

    /// Copies the given register
    CopyReg,

    /// Add a register and an immediate
    AddRegImm,
    /// Multiply a register and an immediate
    MulRegImm,
    /// Subtract a register from an immediate
    SubImmReg,
    /// Subtract an immediate from a register
    SubRegImm,

    /// Adds two registers
    AddRegReg,
    /// Multiplies two registers
    MulRegReg,
    /// Subtracts two registers
    SubRegReg,

    /// Compute the minimum of a register and an immediate
    MinRegImm,
    /// Compute the maximum of a register and an immediate
    MaxRegImm,
    /// Compute the minimum of two registers
    MinRegReg,
    /// Compute the maximum of two registers
    MaxRegReg,
}

/// `Tape` stores a pair of flat expressions suitable for evaluation:
/// - `ssa` is suitable for use during tape simplification
/// - `asm` is ready to be fed into an assembler, e.g. `dynasm`
///
/// We keep both because SSA form makes tape shortening easier, while the `asm`
/// data already has registers assigned.
pub struct Tape {
    ssa: SsaTape,
    asm: Vec<AsmOp>,
    reg_limit: u8,
}

impl Tape {
    pub fn new(s: &Scheduled) -> Self {
        Self::new_with_reg_limit(s, u8::MAX)
    }

    pub fn new_with_reg_limit(s: &Scheduled, reg_limit: u8) -> Self {
        let ssa = SsaTape::new(s);
        let dummy = vec![Choice::Both; ssa.choice_count];
        let (ssa, asm) = ssa.simplify(&dummy, reg_limit);
        Self {
            ssa,
            asm,
            reg_limit,
        }
    }

    pub fn simplify(&self, choices: &[Choice]) -> Self {
        let (ssa, asm) = self.ssa.simplify(choices, self.reg_limit);
        Self {
            ssa,
            asm,
            reg_limit: self.reg_limit,
        }
    }
}

/// Tape storing... stuff
/// - 4-byte opcode
/// - 4-byte output register
/// - 4-byte LHS register
/// - 4-byte RHS register (or immediate `f32`)
///
/// Outputs, arguments, and immediates are packed into the `data` array
///
/// All slot addressing is absolute.
#[derive(Clone, Debug)]
pub struct SsaTape {
    /// The tape is stored in reverse order, such that the root of the tree is
    /// the first item in the tape.
    pub tape: Vec<ClauseOp64>,

    /// Variable-length data for tape clauses.
    ///
    /// Data is densely packed in the order
    /// - output slot
    /// - lhs slot (or input)
    /// - rhs slot (or immediate)
    ///
    /// i.e. a unary operation would only store two items in this array
    pub data: Vec<u32>,

    /// Number of choice operations in the tape
    pub choice_count: usize,
}

impl SsaTape {
    pub fn new(t: &Scheduled) -> Self {
        let mut builder = SsaTapeBuilder::new(t);
        builder.run();
        builder.tape.reverse();
        builder.data.reverse();
        Self {
            tape: builder.tape,
            data: builder.data,
            choice_count: builder.choice_count,
        }
    }

    /// Builds an evaluator which takes a (read-only) reference to this tape
    pub fn get_evaluator(&self) -> SsaTapeEval {
        SsaTapeEval {
            tape: self,
            slots: vec![0.0; self.tape.len()],
        }
    }

    pub fn pretty_print(&self) {
        let mut data = self.data.iter().rev();
        let mut next = || *data.next().unwrap();
        for &op in self.tape.iter().rev() {
            match op {
                ClauseOp64::Input => {
                    let i = next();
                    let out = next();
                    println!("${out} = %{i}");
                }
                ClauseOp64::NegReg
                | ClauseOp64::AbsReg
                | ClauseOp64::RecipReg
                | ClauseOp64::SqrtReg
                | ClauseOp64::CopyReg
                | ClauseOp64::SquareReg => {
                    let arg = next();
                    let out = next();
                    let op = match op {
                        ClauseOp64::NegReg => "NEG",
                        ClauseOp64::AbsReg => "ABS",
                        ClauseOp64::RecipReg => "RECIP",
                        ClauseOp64::SqrtReg => "SQRT",
                        ClauseOp64::SquareReg => "SQUARE",
                        ClauseOp64::CopyReg => "COPY",
                        _ => unreachable!(),
                    };
                    println!("${out} {op} ${arg}");
                }

                ClauseOp64::AddRegReg
                | ClauseOp64::MulRegReg
                | ClauseOp64::SubRegReg
                | ClauseOp64::MinRegReg
                | ClauseOp64::MaxRegReg => {
                    let rhs = next();
                    let lhs = next();
                    let out = next();
                    let op = match op {
                        ClauseOp64::AddRegReg => "ADD",
                        ClauseOp64::MulRegReg => "MUL",
                        ClauseOp64::SubRegReg => "SUB",
                        ClauseOp64::MinRegReg => "MIN",
                        ClauseOp64::MaxRegReg => "MAX",
                        _ => unreachable!(),
                    };
                    println!("${out} = {op} ${lhs} ${rhs}");
                }

                ClauseOp64::AddRegImm
                | ClauseOp64::MulRegImm
                | ClauseOp64::SubImmReg
                | ClauseOp64::SubRegImm
                | ClauseOp64::MinRegImm
                | ClauseOp64::MaxRegImm => {
                    let imm = f32::from_bits(next());
                    let arg = next();
                    let out = next();
                    let (op, swap) = match op {
                        ClauseOp64::AddRegImm => ("ADD", false),
                        ClauseOp64::MulRegImm => ("MUL", false),
                        ClauseOp64::SubImmReg => ("SUB", true),
                        ClauseOp64::SubRegImm => ("SUB", false),
                        ClauseOp64::MinRegImm => ("MIN", false),
                        ClauseOp64::MaxRegImm => ("MAX", false),
                        _ => unreachable!(),
                    };
                    if swap {
                        println!("${out} = {op} ${arg} {imm}");
                    } else {
                        println!("${out} = {op} {imm} ${arg}");
                    }
                }
                ClauseOp64::CopyImm => {
                    let imm = f32::from_bits(next());
                    let out = next();
                    println!("${out} = COPY {imm}");
                }
            }
        }
    }

    pub fn simplify(
        &self,
        choices: &[Choice],
        reg_limit: u8,
    ) -> (Self, Vec<AsmOp>) {
        // If a node is active (i.e. has been used as an input, as we walk the
        // tape in reverse order), then store its new slot assignment here.
        let mut active = vec![None; self.tape.len()];
        let mut count = 0..;
        let mut choice_count = 0;

        let mut alloc = SsaTapeAllocator::new(reg_limit);

        // The tape is constructed so that the output slot is first
        active[self.data[0] as usize] = Some(count.next().unwrap());

        // Other iterators to consume various arrays in order
        let mut data = self.data.iter();
        let mut choice_iter = choices.iter().rev();

        let mut ops_out = vec![];
        let mut data_out = vec![];

        for &op in self.tape.iter() {
            use ClauseOp64::*;
            let index = *data.next().unwrap();
            if active[index as usize].is_none() {
                match op {
                    Input | CopyImm | NegReg | AbsReg | RecipReg | SqrtReg
                    | SquareReg | CopyReg => {
                        data.next().unwrap();
                    }
                    AddRegImm | MulRegImm | SubRegImm | SubImmReg
                    | AddRegReg | MulRegReg | SubRegReg => {
                        data.next().unwrap();
                        data.next().unwrap();
                    }

                    MinRegImm | MaxRegImm | MinRegReg | MaxRegReg => {
                        data.next().unwrap();
                        data.next().unwrap();
                        choice_iter.next().unwrap();
                    }
                }
                continue;
            }

            // Because we reassign nodes when they're used as an *input*
            // (while walking the tape in reverse), this node must have been
            // assigned already.
            let new_index = active[index as usize].unwrap();

            match op {
                Input | CopyImm => {
                    let i = *data.next().unwrap();
                    data_out.push(new_index);
                    data_out.push(i);
                    ops_out.push(op);

                    match op {
                        Input => {
                            alloc.op_input(new_index, i.try_into().unwrap())
                        }
                        CopyImm => {
                            alloc.op_copy_imm(new_index, f32::from_bits(i))
                        }
                        _ => unreachable!(),
                    }
                }
                NegReg | AbsReg | RecipReg | SqrtReg | SquareReg => {
                    let arg = *active[*data.next().unwrap() as usize]
                        .get_or_insert_with(|| count.next().unwrap());
                    data_out.push(new_index);
                    data_out.push(arg);
                    ops_out.push(op);

                    alloc.op_reg(new_index, arg, op);
                }
                CopyReg => {
                    // CopyReg effectively does
                    //      dst <= src
                    // If src has not yet been used (as we iterate backwards
                    // through the tape), then we can replace it with dst
                    // everywhere!
                    let src = *data.next().unwrap();
                    match active[src as usize] {
                        Some(new_src) => {
                            data_out.push(new_index);
                            data_out.push(new_src);
                            ops_out.push(op);

                            alloc.op_reg(new_index, new_src, CopyReg);
                        }
                        None => {
                            active[src as usize] = Some(new_index);
                        }
                    }
                }
                MinRegImm | MaxRegImm => {
                    let arg = *data.next().unwrap();
                    let imm = *data.next().unwrap();
                    match choice_iter.next().unwrap() {
                        Choice::Left => match active[arg as usize] {
                            Some(new_arg) => {
                                data_out.push(new_index);
                                data_out.push(new_arg);
                                ops_out.push(CopyReg);

                                alloc.op_reg(new_index, new_arg, CopyReg);
                            }
                            None => {
                                active[arg as usize] = Some(new_index);
                            }
                        },
                        Choice::Right => {
                            data_out.push(new_index);
                            data_out.push(imm);
                            ops_out.push(CopyImm);

                            alloc.op_copy_imm(new_index, f32::from_bits(imm));
                        }
                        Choice::Both => {
                            choice_count += 1;
                            let arg = *active[arg as usize]
                                .get_or_insert_with(|| count.next().unwrap());

                            data_out.push(new_index);
                            data_out.push(arg);
                            data_out.push(imm);
                            ops_out.push(op);

                            alloc.op_reg_imm(
                                new_index,
                                arg,
                                f32::from_bits(imm),
                                op,
                            );
                        }
                        Choice::Unknown => panic!("oh no"),
                    }
                }
                MinRegReg | MaxRegReg => {
                    let lhs = *data.next().unwrap();
                    let rhs = *data.next().unwrap();
                    match choice_iter.next().unwrap() {
                        Choice::Left => match active[lhs as usize] {
                            Some(new_lhs) => {
                                data_out.push(new_index);
                                data_out.push(new_lhs);
                                ops_out.push(CopyReg);

                                alloc.op_reg(new_index, new_lhs, CopyReg);
                            }
                            None => {
                                active[lhs as usize] = Some(new_index);
                            }
                        },
                        Choice::Right => match active[rhs as usize] {
                            Some(new_rhs) => {
                                data_out.push(new_index);
                                data_out.push(new_rhs);
                                ops_out.push(CopyReg);

                                alloc.op_reg(new_index, new_rhs, CopyReg);
                            }
                            None => {
                                active[rhs as usize] = Some(new_index);
                            }
                        },
                        Choice::Both => {
                            choice_count += 1;
                            let lhs = *active[lhs as usize]
                                .get_or_insert_with(|| count.next().unwrap());
                            let rhs = *active[rhs as usize]
                                .get_or_insert_with(|| count.next().unwrap());
                            data_out.push(new_index);
                            data_out.push(lhs);
                            data_out.push(rhs);
                            ops_out.push(op);

                            alloc.op_reg_reg(new_index, lhs, rhs, op);
                        }
                        Choice::Unknown => panic!("oh no"),
                    }
                }
                AddRegReg | MulRegReg | SubRegReg => {
                    let lhs = *active[*data.next().unwrap() as usize]
                        .get_or_insert_with(|| count.next().unwrap());
                    let rhs = *active[*data.next().unwrap() as usize]
                        .get_or_insert_with(|| count.next().unwrap());
                    data_out.push(new_index);
                    data_out.push(lhs);
                    data_out.push(rhs);
                    ops_out.push(op);

                    alloc.op_reg_reg(new_index, lhs, rhs, op);
                }
                AddRegImm | MulRegImm | SubRegImm | SubImmReg => {
                    let arg = *active[*data.next().unwrap() as usize]
                        .get_or_insert_with(|| count.next().unwrap());
                    let imm = *data.next().unwrap();
                    data_out.push(new_index);
                    data_out.push(arg);
                    data_out.push(imm);
                    ops_out.push(op);

                    alloc.op_reg_imm(new_index, arg, f32::from_bits(imm), op);
                }
            }
        }

        assert_eq!(count.next().unwrap() as usize, ops_out.len());
        assert_eq!(ops_out.len(), alloc.out.len());

        (
            SsaTape {
                tape: ops_out,
                data: data_out,
                choice_count,
            },
            alloc.out,
        )
    }
}

struct SsaTapeAllocator {
    /// Map from the index in the original (globally allocated) tape to a
    /// specific register or memory slot.
    allocations: Vec<u32>,

    /// Map from a particular register to the index in the original tape that's
    /// using that register, or `usize::MAX` if the register is currently unused.
    ///
    /// The inner `u32` here is an index into the original (SSA) tape
    registers: Vec<u32>,

    /// For each register, this `Vec` stores its last access time
    register_lru: Vec<usize>,
    time: usize,

    /// User-defined register limit; beyond this point we use load/store
    /// operations to move values to and from memory.
    reg_limit: u8,

    /// Available short registers (index < 256)
    ///
    /// The most recently available is at the back of the `Vec`
    spare_registers: Vec<u8>,

    /// Available extended registers (index >= 256)
    ///
    /// The most recently available is at the back of the `Vec`
    spare_memory: Vec<u32>,

    /// Total allocated slots
    ///
    /// This will be <= the number of clauses in the tape, because we can often
    /// reuse slots.
    total_slots: u32,

    /// Output slots, assembled in reverse order
    out: Vec<AsmOp>,
}

impl SsaTapeAllocator {
    fn new(reg_limit: u8) -> Self {
        Self {
            allocations: vec![],

            registers: vec![u32::MAX; reg_limit as usize],
            register_lru: vec![0; reg_limit as usize],
            time: 0,

            reg_limit,
            spare_registers: vec![],
            spare_memory: vec![],

            total_slots: 0,
            out: vec![],
        }
    }

    /// Returns an available memory slot.
    ///
    /// We can *always* call this, because we treat memory as unlimited.
    ///
    /// > If there's one thing I love
    /// > It's an infinite resource
    /// > If there's one thing worth loving
    /// > It's a surplus of supplies
    fn get_memory(&mut self) -> u32 {
        if let Some(p) = self.spare_memory.pop() {
            p
        } else {
            let out = self.total_slots;
            self.total_slots += 1;
            out
        }
    }

    /// Finds the oldest register
    ///
    /// This is useful when deciding which register to evict to make room
    fn oldest_reg(&self) -> u8 {
        self.register_lru
            .iter()
            .enumerate()
            .min_by_key(|i| i.1)
            .unwrap()
            .0
            .try_into()
            .unwrap()
    }

    /// Returns the slot allocated to the given node in the globally indexed
    /// tape, or `u32::MAX` if unassigned.
    fn get_allocation(&mut self, n: u32) -> u32 {
        if n as usize >= self.allocations.len() {
            self.allocations.resize(n as usize + 1, u32::MAX);
        }
        self.allocations
            .get(n as usize)
            .cloned()
            .unwrap_or(u32::MAX)
    }

    /// Attempt to get a register for the given node (which is an index into the
    /// globally-allocated tape).
    ///
    /// This happens to work if a node is unassigned, because at that point, it
    /// will be allocated to `usize::MAX`, which looks like a (very far away)
    /// slot in memory.
    ///
    /// If there are no free registers, then this function will move a register
    /// into memory, adjusting the allocations table accordingly.
    fn get_register(&mut self, n: u32) -> u8 {
        // Find the current allocation of this node, if present
        let slot = self.get_allocation(n);
        let reg = if slot >= self.reg_limit as u32 {
            // Pick a register, prioritizing picking a spare register (if
            // possible); if not, then introducing a new register (if we haven't
            // allocated past our limit)
            let reg = self.spare_registers.pop().or_else(|| {
                if self.total_slots < self.reg_limit as u32 {
                    let reg = self.total_slots;
                    self.total_slots += 1;
                    Some(reg.try_into().unwrap())
                } else {
                    None
                }
            });
            let reg = if let Some(reg) = reg {
                // If we've got a spare register, then we can use it by adding a
                // `Load` instruction to the stream.
                assert_eq!(self.registers[reg as usize], u32::MAX);
                reg
            } else {
                // Otherwise, we need to free up a register by pushing the
                // oldest value to a slot in memory.
                let reg = self.oldest_reg();

                // Here's where it will go:
                let mem = self.get_memory();

                // Whoever was previously using you is in for a surprise
                let prev_node = self.registers[reg as usize];
                self.allocations[prev_node as usize] = mem;

                // Because we're constructing the AsmOp tape in reverse,
                // this looks like a Load (instead of a Store)
                self.out.push(AsmOp::Load(reg, mem));
                reg
            };
            // Release the memory slot that we were previously using, if it's
            // not the dummy slot (indicating no assignment has been made)
            if slot != u32::MAX {
                self.spare_memory.push(slot);
                self.out.push(AsmOp::Store(reg, slot));
            }
            reg
        } else {
            slot as u8
        };
        // Bind the register and update its use time
        self.registers[reg as usize] = n;
        self.allocations[n as usize] = reg as u32;
        self.register_lru[reg as usize] = self.time;
        self.time += 1;
        reg
    }

    /// Release a register back to the pool of spares
    fn release(&mut self, reg: u8) {
        // Release the output register, so it could be used for inputs
        let node = self.registers[reg as usize];
        self.registers[reg as usize] = u32::MAX;
        self.spare_registers.push(reg);
        // Modifying self.allocations isn't strictly necessary, but could help
        // us detect logical errors (since it should never be used after this)
        self.allocations[node as usize] = u32::MAX;
    }

    fn op_reg(&mut self, out: u32, arg: u32, op: ClauseOp64) {
        // The output must be allocated already, since we're walking the tape in
        // reverse.  However, it may be in a memory slot, so we might need to
        // free up a register for it.
        assert!(self.get_allocation(out) != u32::MAX);

        let out = self.get_register(out);
        self.release(out);

        let arg = self.get_register(arg);

        let op: fn(u8, u8) -> AsmOp = match op {
            ClauseOp64::NegReg => AsmOp::NegReg,
            ClauseOp64::AbsReg => AsmOp::AbsReg,
            ClauseOp64::RecipReg => AsmOp::RecipReg,
            ClauseOp64::SqrtReg => AsmOp::SqrtReg,
            ClauseOp64::SquareReg => AsmOp::SquareReg,
            _ => panic!(),
        };
        self.out.push(op(out, arg));
    }

    fn op_reg_reg(&mut self, out: u32, lhs: u32, rhs: u32, op: ClauseOp64) {
        assert!(self.get_allocation(out) != u32::MAX);

        let out = self.get_register(out);
        self.release(out);

        let lhs = self.get_register(lhs);
        let rhs = self.get_register(rhs);
        let op: fn(u8, u8, u8) -> AsmOp = match op {
            ClauseOp64::AddRegReg => AsmOp::AddRegReg,
            ClauseOp64::SubRegReg => AsmOp::SubRegReg,
            ClauseOp64::MulRegReg => AsmOp::MulRegReg,
            ClauseOp64::MinRegReg => AsmOp::MinRegReg,
            ClauseOp64::MaxRegReg => AsmOp::MaxRegReg,
            _ => panic!(),
        };
        self.out.push(op(out, lhs, rhs));
    }

    fn op_reg_imm(&mut self, out: u32, arg: u32, imm: f32, op: ClauseOp64) {
        assert!(self.get_allocation(out) != u32::MAX);

        let out = self.get_register(out);
        self.release(out);

        let arg = self.get_register(arg);
        let op: fn(u8, u8, f32) -> AsmOp = match op {
            ClauseOp64::AddRegImm => AsmOp::AddRegImm,
            ClauseOp64::SubRegImm => AsmOp::SubRegImm,
            ClauseOp64::SubImmReg => AsmOp::SubImmReg,
            ClauseOp64::MulRegImm => AsmOp::MulRegImm,
            ClauseOp64::MinRegImm => AsmOp::MinRegImm,
            ClauseOp64::MaxRegImm => AsmOp::MaxRegImm,
            _ => panic!(),
        };
        self.out.push(op(out, arg, imm));
    }

    fn op_copy_imm(&mut self, out: u32, imm: f32) {
        assert!(self.get_allocation(out) != u32::MAX);

        let out = self.get_register(out);
        self.release(out);

        self.out.push(AsmOp::CopyImm(out, imm));
    }

    fn op_input(&mut self, out: u32, i: u8) {
        assert!(self.get_allocation(out) != u32::MAX);

        let out = self.get_register(out);
        self.release(out);

        self.out.push(AsmOp::Input(out, i));
    }
}

////////////////////////////////////////////////////////////////////////////////

struct SsaTapeBuilder<'a> {
    iter: std::slice::Iter<'a, (NodeIndex, Op)>,

    tape: Vec<ClauseOp64>,
    data: Vec<u32>,

    vars: &'a IndexMap<String, VarIndex>,
    mapping: BTreeMap<NodeIndex, u32>,
    constants: BTreeMap<NodeIndex, f32>,
    choice_count: usize,
}

#[derive(Debug)]
enum Location {
    Slot(u32),
    Immediate(f32),
}

impl<'a> SsaTapeBuilder<'a> {
    fn new(t: &'a Scheduled) -> Self {
        Self {
            iter: t.tape.iter(),
            tape: vec![],
            data: vec![],
            vars: &t.vars,
            mapping: BTreeMap::new(),
            constants: BTreeMap::new(),
            choice_count: 0,
        }
    }

    fn get_allocated_value(&mut self, node: NodeIndex) -> Location {
        if let Some(r) = self.mapping.get(&node).cloned() {
            Location::Slot(r)
        } else {
            let c = self.constants.get(&node).unwrap();
            Location::Immediate(*c)
        }
    }

    fn run(&mut self) {
        while let Some(&(n, op)) = self.iter.next() {
            self.step(n, op);
        }
    }

    fn step(&mut self, node: NodeIndex, op: Op) {
        let index: u32 = self.mapping.len().try_into().unwrap();
        let op = match op {
            Op::Var(v) => {
                let arg = match self.vars.get_by_index(v).unwrap().as_str() {
                    "X" => 0,
                    "Y" => 1,
                    "Z" => 2,
                    _ => panic!(),
                };
                self.data.push(arg);
                self.data.push(index);
                Some(ClauseOp64::Input)
            }
            Op::Const(c) => {
                // Skip this (because it's not inserted into the tape),
                // recording its value for use as an immediate later.
                self.constants.insert(node, c as f32);
                None
            }
            Op::Binary(op, lhs, rhs) => {
                let lhs = self.get_allocated_value(lhs);
                let rhs = self.get_allocated_value(rhs);

                let f = match op {
                    BinaryOpcode::Add => (
                        ClauseOp64::AddRegReg,
                        ClauseOp64::AddRegImm,
                        ClauseOp64::AddRegImm,
                    ),
                    BinaryOpcode::Mul => (
                        ClauseOp64::MulRegReg,
                        ClauseOp64::MulRegImm,
                        ClauseOp64::MulRegImm,
                    ),
                    BinaryOpcode::Sub => (
                        ClauseOp64::SubRegReg,
                        ClauseOp64::SubRegImm,
                        ClauseOp64::SubImmReg,
                    ),
                };

                let op = match (lhs, rhs) {
                    (Location::Slot(lhs), Location::Slot(rhs)) => {
                        self.data.push(rhs);
                        self.data.push(lhs);
                        self.data.push(index);
                        f.0
                    }
                    (Location::Slot(arg), Location::Immediate(imm)) => {
                        self.data.push(imm.to_bits());
                        self.data.push(arg);
                        self.data.push(index);
                        f.1
                    }
                    (Location::Immediate(imm), Location::Slot(arg)) => {
                        self.data.push(imm.to_bits());
                        self.data.push(arg);
                        self.data.push(index);
                        f.2
                    }
                    (Location::Immediate(..), Location::Immediate(..)) => {
                        panic!("Cannot handle f(imm, imm)")
                    }
                };
                Some(op)
            }
            Op::BinaryChoice(op, lhs, rhs, ..) => {
                self.choice_count += 1;
                let lhs = self.get_allocated_value(lhs);
                let rhs = self.get_allocated_value(rhs);

                let f = match op {
                    BinaryChoiceOpcode::Min => {
                        (ClauseOp64::MinRegReg, ClauseOp64::MinRegImm)
                    }
                    BinaryChoiceOpcode::Max => {
                        (ClauseOp64::MaxRegReg, ClauseOp64::MaxRegImm)
                    }
                };

                let op = match (lhs, rhs) {
                    (Location::Slot(lhs), Location::Slot(rhs)) => {
                        self.data.push(rhs);
                        self.data.push(lhs);
                        self.data.push(index);
                        f.0
                    }
                    (Location::Slot(arg), Location::Immediate(imm)) => {
                        self.data.push(imm.to_bits());
                        self.data.push(arg);
                        self.data.push(index);
                        f.1
                    }
                    (Location::Immediate(imm), Location::Slot(arg)) => {
                        self.data.push(imm.to_bits());
                        self.data.push(arg);
                        self.data.push(index);
                        f.1
                    }
                    (Location::Immediate(..), Location::Immediate(..)) => {
                        panic!("Cannot handle f(imm, imm)")
                    }
                };
                Some(op)
            }
            Op::Unary(op, lhs) => {
                let lhs = match self.get_allocated_value(lhs) {
                    Location::Slot(r) => r,
                    Location::Immediate(..) => {
                        panic!("Cannot handle f(imm)")
                    }
                };
                let op = match op {
                    UnaryOpcode::Neg => ClauseOp64::NegReg,
                    UnaryOpcode::Abs => ClauseOp64::AbsReg,
                    UnaryOpcode::Recip => ClauseOp64::RecipReg,
                    UnaryOpcode::Sqrt => ClauseOp64::SqrtReg,
                    UnaryOpcode::Square => ClauseOp64::SquareReg,
                };
                self.data.push(lhs);
                self.data.push(index);
                Some(op)
            }
        };

        if let Some(op) = op {
            self.tape.push(op);
            let r = self.mapping.insert(node, index);
            assert!(r.is_none());
        }
    }
}

////////////////////////////////////////////////////////////////////////////////

/// Workspace to evaluate a tape
pub struct SsaTapeEval<'a> {
    tape: &'a SsaTape,
    slots: Vec<f32>,
}

impl<'a> SsaTapeEval<'a> {
    fn v(&self, i: u32) -> f32 {
        self.slots[i as usize]
    }
    pub fn f(&mut self, x: f32, y: f32, z: f32) -> f32 {
        let mut data = self.tape.data.iter().rev();
        let mut next = || *data.next().unwrap();
        for &op in self.tape.tape.iter().rev() {
            let out = match op {
                ClauseOp64::Input => match next() {
                    0 => x,
                    1 => y,
                    2 => z,
                    _ => panic!(),
                },
                ClauseOp64::NegReg
                | ClauseOp64::AbsReg
                | ClauseOp64::RecipReg
                | ClauseOp64::SqrtReg
                | ClauseOp64::CopyReg
                | ClauseOp64::SquareReg => {
                    let arg = self.v(next());
                    match op {
                        ClauseOp64::NegReg => -arg,
                        ClauseOp64::AbsReg => arg.abs(),
                        ClauseOp64::RecipReg => 1.0 / arg,
                        ClauseOp64::SqrtReg => arg.sqrt(),
                        ClauseOp64::SquareReg => arg * arg,
                        ClauseOp64::CopyReg => arg,
                        _ => unreachable!(),
                    }
                }

                ClauseOp64::AddRegReg
                | ClauseOp64::MulRegReg
                | ClauseOp64::SubRegReg
                | ClauseOp64::MinRegReg
                | ClauseOp64::MaxRegReg => {
                    let rhs = self.v(next());
                    let lhs = self.v(next());
                    match op {
                        ClauseOp64::AddRegReg => lhs + rhs,
                        ClauseOp64::MulRegReg => lhs * rhs,
                        ClauseOp64::SubRegReg => lhs - rhs,
                        ClauseOp64::MinRegReg => lhs.min(rhs),
                        ClauseOp64::MaxRegReg => lhs.max(rhs),
                        _ => unreachable!(),
                    }
                }

                ClauseOp64::AddRegImm
                | ClauseOp64::MulRegImm
                | ClauseOp64::SubImmReg
                | ClauseOp64::SubRegImm
                | ClauseOp64::MinRegImm
                | ClauseOp64::MaxRegImm => {
                    let imm = f32::from_bits(next());
                    let arg = self.v(next());
                    match op {
                        ClauseOp64::AddRegImm => arg + imm,
                        ClauseOp64::MulRegImm => arg * imm,
                        ClauseOp64::SubImmReg => imm - arg,
                        ClauseOp64::SubRegImm => arg - imm,
                        ClauseOp64::MinRegImm => arg.min(imm),
                        ClauseOp64::MaxRegImm => arg.max(imm),
                        _ => unreachable!(),
                    }
                }
                ClauseOp64::CopyImm => f32::from_bits(next()),
            };
            self.slots[next() as usize] = out;
        }
        self.slots[self.tape.data[0] as usize]
    }
}

////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::common::Choice;

    #[test]
    fn basic_interpreter() {
        let mut ctx = crate::context::Context::new();
        let x = ctx.x();
        let y = ctx.y();
        let one = ctx.constant(1.0);
        let sum = ctx.add(x, one).unwrap();
        let min = ctx.min(sum, y).unwrap();
        let scheduled = crate::scheduled::schedule(&ctx, min);
        let tape = Tape::new(&scheduled);
        let mut eval = tape.ssa.get_evaluator();
        assert_eq!(eval.f(1.0, 2.0, 0.0), 2.0);
        assert_eq!(eval.f(1.0, 3.0, 0.0), 2.0);
        assert_eq!(eval.f(3.0, 3.5, 0.0), 3.5);
    }

    #[test]
    fn test_push() {
        let mut ctx = crate::context::Context::new();
        let x = ctx.x();
        let y = ctx.y();
        let min = ctx.min(x, y).unwrap();

        let scheduled = crate::scheduled::schedule(&ctx, min);
        let tape = Tape::new(&scheduled);
        let mut eval = tape.ssa.get_evaluator();
        assert_eq!(eval.f(1.0, 2.0, 0.0), 1.0);
        assert_eq!(eval.f(3.0, 2.0, 0.0), 2.0);

        let t = tape.simplify(&[Choice::Left]);
        let mut eval = t.ssa.get_evaluator();
        assert_eq!(eval.f(1.0, 2.0, 0.0), 1.0);
        assert_eq!(eval.f(3.0, 2.0, 0.0), 3.0);

        let t = tape.simplify(&[Choice::Right]);
        let mut eval = t.ssa.get_evaluator();
        assert_eq!(eval.f(1.0, 2.0, 0.0), 2.0);
        assert_eq!(eval.f(3.0, 2.0, 0.0), 2.0);

        let one = ctx.constant(1.0);
        let min = ctx.min(x, one).unwrap();
        let scheduled = crate::scheduled::schedule(&ctx, min);
        let tape = Tape::new(&scheduled);
        let mut eval = tape.ssa.get_evaluator();
        assert_eq!(eval.f(0.5, 0.0, 0.0), 0.5);
        assert_eq!(eval.f(3.0, 0.0, 0.0), 1.0);

        let t = tape.simplify(&[Choice::Left]);
        let mut eval = t.ssa.get_evaluator();
        assert_eq!(eval.f(0.5, 0.0, 0.0), 0.5);
        assert_eq!(eval.f(3.0, 0.0, 0.0), 3.0);

        let t = tape.simplify(&[Choice::Right]);
        let mut eval = t.ssa.get_evaluator();
        assert_eq!(eval.f(0.5, 0.0, 0.0), 1.0);
        assert_eq!(eval.f(3.0, 0.0, 0.0), 1.0);
    }
}
