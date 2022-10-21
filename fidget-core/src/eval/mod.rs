//! Evaluation, both generically and with a small local interpreter

pub mod asm;
mod choice;

pub mod float_slice;
pub mod grad;
pub mod interval;
pub mod point;

// Re-export a few things
pub use choice::Choice;

use float_slice::FloatSliceEvalT;
use grad::GradEvalT;
use interval::IntervalEvalT;
use point::PointEvalT;

/// Represents a "family" of evaluators (JIT, interpreter, etc)
pub trait EvalFamily {
    /// Register limit for this evaluator family.
    const REG_LIMIT: u8;

    type IntervalEval: IntervalEvalT;
    type FloatSliceEval: FloatSliceEvalT;
    type PointEval: PointEvalT;
    type GradEval: GradEvalT;
}
