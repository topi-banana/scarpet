mod error;
mod eval;
mod function;
mod value;
mod vm;

pub use error::VmError;
pub use eval::Evalute;
pub use function::{DefFunction, Function};
pub use value::{Value, ValueContainer};
pub use vm::{GlobalState, ScarpetVm};
