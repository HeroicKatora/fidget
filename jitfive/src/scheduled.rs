use std::collections::{BTreeMap, BTreeSet};

use crate::{
    backend::common::{NodeIndex, Op, VarIndex},
    context::{Context, Node},
    util::indexed::IndexMap,
};

/// Represents a set of instructions that have been scheduled (somehow)
#[derive(Debug)]
pub struct Scheduled {
    /// Topologically sorted instruction list, i.e. all nodes are guaranteed to
    /// execute _after_ their inputs.
    pub tape: Vec<(NodeIndex, Op)>,
    pub vars: IndexMap<String, VarIndex>,
    pub root: NodeIndex,
}

impl Scheduled {
    pub fn new(
        tape: Vec<(NodeIndex, Op)>,
        vars: IndexMap<String, VarIndex>,
        root: NodeIndex,
    ) -> Self {
        Self { tape, vars, root }
    }
}

/// Schedules the given math graph using a depth-first-ish strategy
pub fn schedule(ctx: &Context, root: Node) -> Scheduled {
    // Mappings from the context into the scheduler
    let mut nodes: IndexMap<Node, NodeIndex> = IndexMap::default();
    let mut vars: IndexMap<String, VarIndex> = IndexMap::default();

    // Stores parents of a given node.  Parents are erased from this set as
    // they are descheduled, so the length of the set serves as a "score".
    //
    // Children are stored implicitly in the context, i.e.
    // ```
    // ctx.get_op(nodes.get_by_index(n)).iter_children()
    // ```
    let mut parents: BTreeMap<NodeIndex, BTreeSet<NodeIndex>> =
        BTreeMap::default();

    // Stores whether the given node has been scheduled yet
    let mut scheduled = BTreeSet::default();

    // The output tape, which is topologically sorted
    let mut out = vec![];

    // Accumulate all parents
    let mut todo = vec![root];
    let mut seen = BTreeSet::new();
    while let Some(node) = todo.pop() {
        if !seen.insert(node) {
            continue;
        }
        let index = nodes.insert(node);
        let op = ctx.get_op(node).unwrap();
        for child in op.iter_children() {
            let child_index = nodes.insert(child);
            parents.entry(child_index).or_default().insert(index);
            todo.push(child);
        }
    }

    // Flatten the graph
    let mut todo = vec![nodes.get_by_value(root).unwrap()];
    while let Some(index) = todo.pop() {
        if parents.get(&index).map(|b| b.len()).unwrap_or(0) > 0
            || !scheduled.insert(index)
        {
            continue;
        }

        let node = *nodes.get_by_index(index).unwrap();
        let op = ctx.get_op(node).unwrap();
        for child in op.iter_children() {
            let child_index = nodes.get_by_value(child).unwrap();
            todo.push(child_index);
            let r = parents.get_mut(&child_index).unwrap().remove(&index);
            assert!(r);
        }

        use crate::context::Op as CtxOp;
        let op = match op {
            CtxOp::Unary(op, lhs) => {
                Op::Unary(*op, nodes.get_by_value(*lhs).unwrap())
            }
            CtxOp::Binary(op, lhs, rhs) => Op::Binary(
                *op,
                nodes.get_by_value(*lhs).unwrap(),
                nodes.get_by_value(*rhs).unwrap(),
            ),
            CtxOp::Const(i) => Op::Const(i.0),
            CtxOp::Var(v) => Op::Var(
                vars.insert(ctx.get_var_by_index(*v).unwrap().to_string()),
            ),
        };
        out.push((index, op));
    }
    out.reverse();

    Scheduled::new(out, vars, nodes.get_by_value(root).unwrap())
}
