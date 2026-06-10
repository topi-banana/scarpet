use std::rc::Rc;

use scarpet_syntax::ast::{Assign, Code, Expr, ParamWord};

use super::Evaluate;
use crate::{error::VmError, function::DefFunction, value::ValueContainer, vm::ScarpetVm};

impl<'src, 'state> Evaluate<Code<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, Code(mut sts): Code<'src>) -> Result<ValueContainer, VmError> {
        let last = sts.pop();
        for st in sts {
            self.push(st)?;
        }
        if let Some(st) = last {
            self.push(st)
        } else {
            Ok(ValueContainer::null())
        }
    }
}

impl<'src, 'state> Evaluate<Expr<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Expr<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Expr::Def { name, params, body } => {
                // Resolve each `outer(x)` against the *defining* scope now, grabbing
                // its shared slot to inject when the function runs.
                let mut captures = Vec::with_capacity(params.captures.len());
                for cap in &params.captures {
                    let slot = match cap.word {
                        ParamWord::Outer => self.get_var(cap.name),
                    };
                    captures.push((cap.name, slot));
                }
                let func = DefFunction::new(&params, captures, body);
                self.define(name, Rc::new(func));
                Ok(ValueContainer::string(name.to_owned()))
            }
            Expr::Assign(ost) => self.push(ost),
            // A bare `->` outside a map (a lambda) is not modelled yet.
            Expr::Arrow { .. } => todo!(),
        }
    }
}

impl<'src, 'state> Evaluate<Assign<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Assign<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Assign::Set { target, op, value } => {
                // The right-hand side is evaluated once, in the current scope,
                // before binding; `assign_lvalue` then routes it to the target — a
                // single place for `op` to update, or a destructure to spread across.
                let value = self.push(*value)?;
                self.assign_lvalue(target, op, value)
            }
            Assign::Lor(ost) => self.push(ost),
        }
    }
}
