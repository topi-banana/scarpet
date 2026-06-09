use std::cmp::Ordering;

use scarpet_syntax::ast::{Additive, Compare, Equality, Get, GetOp, Land, Lor, Mult, Power, Unary};

use super::Evalute;
use crate::{error::VmError, value::ValueContainer, vm::ScarpetVm};

impl<'src, 'state> Evalute<Lor<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Lor<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Lor::Or { lhs, rhs } => Ok(ValueContainer::bool(
                self.push(*lhs)?.lock()?.is_true() || self.push(rhs)?.lock()?.is_true(),
            )),
            Lor::Land(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Land<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Land<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Land::And { lhs, rhs } => Ok(ValueContainer::bool(
                self.push(*lhs)?.lock()?.is_true() && self.push(rhs)?.lock()?.is_true(),
            )),
            Land::Equality(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Equality<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Equality<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Equality::Eq { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(lhs.scarpet_eq(&rhs)?))
            }
            Equality::Ne { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(!lhs.scarpet_eq(&rhs)?))
            }
            Equality::Compare(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Compare<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Compare<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Compare::Lt { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(
                    lhs.scarpet_compare(&rhs)? == Ordering::Less,
                ))
            }
            Compare::Le { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(
                    lhs.scarpet_compare(&rhs)? != Ordering::Greater,
                ))
            }
            Compare::Gt { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(
                    lhs.scarpet_compare(&rhs)? == Ordering::Greater,
                ))
            }
            Compare::Ge { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                Ok(ValueContainer::bool(
                    lhs.scarpet_compare(&rhs)? != Ordering::Less,
                ))
            }
            Compare::Additive(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Additive<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Additive<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Additive::Add { lhs, rhs } => self.push(*lhs)? + self.push(rhs)?,
            Additive::Sub { lhs, rhs } => self.push(*lhs)? - self.push(rhs)?,
            Additive::Mult(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Mult<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Mult<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Mult::Mul { lhs, rhs } => self.push(*lhs)? * self.push(rhs)?,
            Mult::Div { lhs, rhs } => self.push(*lhs)? / self.push(rhs)?,
            Mult::Rem { lhs, rhs } => {
                let (lhs, rhs) = (self.push(*lhs)?, self.push(rhs)?);
                lhs.scarpet_rem(&rhs)
            }
            Mult::Power(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Power<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Power<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Power::Pow { base, exp } => {
                let (base, exp) = (self.push(base)?, self.push(*exp)?);
                base.scarpet_pow(&exp)
            }
            Power::Unary(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Unary<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Unary<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Unary::Neg(v) => self.push(*v)?.scarpet_neg(),
            Unary::Pos(v) => self.push(*v)?.scarpet_pos(),
            Unary::Not(v) => self.push(*v)?.scarpet_not(),
            Unary::Unpack(v) => Ok(ValueContainer::Expand(match self.push(*v)? {
                ValueContainer::Single(v) => v,
                ValueContainer::Expand(v) => v,
            })),
            Unary::Get(ost) => self.push(ost),
        }
    }
}

impl<'src, 'state> Evalute<Get<'src>> for ScarpetVm<'state, 'src> {
    fn push(&mut self, st: Get<'src>) -> Result<ValueContainer, VmError> {
        match st {
            Get::Index { base, op, key } => {
                let base = self.push(*base)?;
                let key = self.push(key)?;
                match op {
                    GetOp::Get => base.scarpet_get(&key),
                    GetOp::Match => base.scarpet_match(&key),
                }
            }
            Get::Primary(ost) => self.push(ost),
        }
    }
}
