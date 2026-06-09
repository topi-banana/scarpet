use crate::{error::VmError, value::ValueContainer};

mod assign;
mod destructure;
mod operator;
mod primary;
mod stmt;

pub trait Evalute<T> {
    fn push(&mut self, st: T) -> Result<ValueContainer, VmError>;
}
