// Prototype evaluator: most match arms are still `todo!()`, so the operands
// they bind (`lhs`, `rhs`, ...) are intentionally unused until those arms get
// implemented. Drop this allow once the evaluator is filled in.
#![allow(unused_variables)]

use scarpet_syntax::ast::{
    Additive, Assign, Code, Compare, Equality, Expr, Get, Land, Lor, Mult, Power, Primary, Unary,
};

use crate::{
    error::VmError,
    value::{Value, ValueContainer},
    vm::ScarpetVm,
};

pub trait Evalute<T> {
    fn push(&mut self, st: T) -> Result<ValueContainer, VmError>;
}

impl<'src, 'state> Evalute<Code<'src>> for ScarpetVm<'state> {
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

impl<'src, 'state> Evalute<Expr<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Expr<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Expr::Assign(ost) => self.push(ost),
            _ => todo!(),
        }
    }
}

impl<'src, 'state> Evalute<Assign<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Assign<'src>) -> Result<ValueContainer, VmError> {
        use scarpet_syntax::ast::{AssignOp, Assignable};
        match st {
            Assign::Set { target, op, value } => {
                let var = match target {
                    Assignable::Var(name) => self.var.get(name).cloned().unwrap_or_else(|| {
                        let v = ValueContainer::null();
                        self.var.insert(name.to_owned(), v.clone());
                        v
                    }),
                    _ => todo!(),
                };
                let val = self.push(*value)?;
                match op {
                    AssignOp::Assign => *var.lock()? = val.lock()?.clone(),
                    AssignOp::Add => *var.lock()? += val.lock()?.clone(),
                    AssignOp::Swap => std::mem::swap(&mut *var.lock()?, &mut *val.lock()?),
                }
                Ok(var.clone())
            }
            Assign::Lor(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Lor<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Lor<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Lor::Or { lhs, rhs } => Ok(ValueContainer::bool(
                self.push(*lhs)?.lock()?.is_true() || self.push(rhs)?.lock()?.is_true(),
            )),
            Lor::Land(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Land<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Land<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Land::And { lhs, rhs } => Ok(ValueContainer::bool(
                self.push(*lhs)?.lock()?.is_true() && self.push(rhs)?.lock()?.is_true(),
            )),
            Land::Equality(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Equality<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Equality<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Equality::Eq { lhs, rhs } => todo!(),
            Equality::Ne { lhs, rhs } => todo!(),
            Equality::Compare(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Compare<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Compare<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Compare::Lt { lhs, rhs } => todo!(),
            Compare::Le { lhs, rhs } => todo!(),
            Compare::Gt { lhs, rhs } => todo!(),
            Compare::Ge { lhs, rhs } => todo!(),
            Compare::Additive(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Additive<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Additive<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Additive::Add { lhs, rhs } => self.push(*lhs)? + self.push(rhs)?,
            Additive::Sub { lhs, rhs } => self.push(*lhs)? - self.push(rhs)?,
            Additive::Mult(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Mult<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Mult<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Mult::Mul { lhs, rhs } => self.push(*lhs)? * self.push(rhs)?,
            Mult::Div { lhs, rhs } => self.push(*lhs)? / self.push(rhs)?,
            Mult::Rem { lhs, rhs } => todo!(),
            Mult::Power(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Power<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Power<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Power::Pow { base, exp } => todo!(),
            Power::Unary(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Unary<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Unary<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Unary::Neg(v) => todo!(),
            Unary::Pos(v) => todo!(),
            Unary::Not(v) => todo!(),
            Unary::Unpack(v) => todo!(),
            Unary::Get(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Get<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Get<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Get::Index { base, op, key } => todo!(),
            Get::Primary(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Primary<'src>> for ScarpetVm<'state> {
    fn push(&mut self, st: Primary<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Primary::Number(v) => Ok(ValueContainer::new(Value::from_number_literal(v))),
            Primary::Str(v) => todo!(),
            Primary::Ident(v) => todo!(),
            Primary::Call { name, args } => todo!(),
            Primary::List(v) => todo!(),
            Primary::Map(v) => todo!(),
            Primary::Paren(v) => todo!(),
        }
    }
}
