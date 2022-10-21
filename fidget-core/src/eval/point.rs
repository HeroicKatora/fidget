use crate::{eval::Choice, tape::Tape};

/// Function handle for `f32` evaluation
pub trait PointEvalT: From<Tape> {
    fn eval_p(&mut self, x: f32, y: f32, z: f32, c: &mut [Choice]) -> f32;
}

/// Function handle for point evaluation
///
/// This trait represents a `struct` that _owns_ a function, but does not have
/// the equipment to evaluate it (e.g. scratch memory).  It is used to produce
/// one or more `PointEval` objects, which actually do evaluation.
pub struct PointEval<E> {
    pub(crate) tape: Tape,
    pub(crate) choices: Vec<Choice>,
    pub(crate) eval: E,
}

impl<E: PointEvalT> From<Tape> for PointEval<E> {
    fn from(tape: Tape) -> Self {
        Self {
            tape: tape.clone(),
            choices: vec![Choice::Unknown; tape.choice_count()],
            eval: E::from(tape),
        }
    }
}

impl<E: PointEvalT> PointEval<E> {
    /// Calculates a simplified [`Tape`](crate::tape::Tape) based on the last
    /// evaluation.
    pub fn simplify(&self, reg_limit: u8) -> Tape {
        self.tape.simplify_with_reg_limit(&self.choices, reg_limit)
    }

    /// Resets the internal choice array to `Choice::Unknown`
    fn reset_choices(&mut self) {
        self.choices.fill(Choice::Unknown);
    }

    /// Performs point evaluation
    pub fn eval_p(&mut self, x: f32, y: f32, z: f32) -> f32 {
        self.reset_choices();
        let out = self.eval.eval_p(x, y, z, self.choices.as_mut_slice());
        out
    }
}
