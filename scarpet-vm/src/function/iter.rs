use scarpet_syntax::ast::Args;

use super::Function;
use crate::{
    Value,
    error::VmError,
    eval::Evaluate,
    value::{RangeList, ValueContainer},
    vm::ScarpetVm,
};

/// `range(to)` / `range(from, to)` / `range(from, to, step)` — a lazy arithmetic
/// progression (the original `range`, whose `type()` is "iterator"). `from`
/// defaults to `0` and `step` to `1`, and `to` is exclusive. Each bound coerces
/// to a number; the range stays integral unless a bound is fractional. A
/// negative step counts down; a zero or wrong-way step is an empty range. The
/// list is generated lazily — `range(1000000)` is a handful of numbers until
/// something walks it.
pub(super) struct Range;
impl<'src> Function<'src> for Range {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        Args(codes): Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        if codes.is_empty() || codes.len() > 3 {
            return Err(VmError::WrongArgCount);
        }
        let nums = codes
            .into_iter()
            .map(|code| Ok(vm.push(code)?.lock()?.clone()))
            .collect::<Result<Vec<Value>, VmError>>()?;
        let (zero, one) = (Value::Int(0), Value::Int(1));
        let range = match nums.as_slice() {
            [to] => RangeList::new(&zero, to, &one),
            [from, to] => RangeList::new(from, to, &one),
            [from, to, step] => RangeList::new(from, to, step),
            // The argument count is bounded to 1..=3 above.
            _ => unreachable!(),
        }?;
        Ok(ValueContainer::new(Value::List(Box::new(range))))
    }
}

/// `for(list, expr(_, _i))` — evaluate `expr` once per element of `list`, with
/// `_` bound to the element and `_i` to its index, returning how many times `expr`
/// was truthy (the original `for`). The body runs in the *current* scope, so a
/// `sum += _` accumulates outside the loop; `_` / `_i` are ordinary slots set each
/// iteration. A lazy `range` is walked element by element, so a huge range is not
/// realised up front. (`break` / `continue` are not modelled yet.)
pub(super) struct For;
impl<'src> Function<'src> for For {
    fn call(
        &self,
        vm: &mut ScarpetVm<'_, 'src>,
        Args(mut args): Args<'src>,
    ) -> Result<ValueContainer, VmError> {
        if args.len() != 2 {
            return Err(VmError::WrongArgCount);
        }
        let list = args.pop_front().expect("checked len == 2");
        let expr = args.pop_front().expect("checked len == 2");
        // Evaluate the list once; a lazy backing (a `range`) clones cheaply and is
        // still walked one element at a time below.
        let Value::List(items) = vm.push(list)?.lock()?.clone() else {
            return Err(VmError::ExpectedList);
        };
        let mut count: i64 = 0;
        for i in 0..items.len() {
            let Some(element) = items.get(i) else { break };
            // Bind `_` / `_i` in the current scope, then evaluate the body there.
            *vm.get_var("_").lock()? = element;
            *vm.get_var("_i").lock()? = Value::Int(i as i64);
            if vm.push(expr.clone())?.lock()?.is_true() {
                count += 1;
            }
        }
        Ok(ValueContainer::int(count))
    }
}

#[cfg(test)]
mod tests {
    use crate::error::VmError;
    use crate::test_util::{eval, eval_capturing_stdout, eval_err};
    use crate::value::Value;

    /// `range` reports its `type()` as "iterator", not "list" — a lazy list.
    #[test]
    fn range_is_an_iterator() {
        assert_eq!(eval("type(range(5))"), Value::String("iterator".to_owned()));
    }

    /// `range(to)` counts from 0; the three forms match Python's `range`. It is
    /// an iterator, but compares element-wise equal to the realised list.
    #[test]
    fn range_forms_yield_the_expected_elements() {
        assert_eq!(
            eval("range(5)"),
            Value::list(vec![
                Value::Int(0),
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
            ])
        );
        assert_eq!(
            eval("range(2, 6)"),
            Value::list(vec![
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
                Value::Int(5),
            ])
        );
        assert_eq!(
            eval("range(0, 10, 2)"),
            Value::list(vec![
                Value::Int(0),
                Value::Int(2),
                Value::Int(4),
                Value::Int(6),
                Value::Int(8),
            ])
        );
    }

    /// A negative step counts down; a wrong-way or empty span yields no elements.
    #[test]
    fn range_negative_step_and_empty() {
        assert_eq!(
            eval("range(5, 0, -1)"),
            Value::list(vec![
                Value::Int(5),
                Value::Int(4),
                Value::Int(3),
                Value::Int(2),
                Value::Int(1),
            ])
        );
        assert_eq!(eval("range(0)"), Value::list(vec![]));
        assert_eq!(eval("range(5, 0)"), Value::list(vec![]));
    }

    /// A fractional bound promotes the whole range to doubles.
    #[test]
    fn range_with_fractional_step_is_double() {
        assert_eq!(
            eval("range(0, 1, 0.5)"),
            Value::list(vec![Value::Double(0.0), Value::Double(0.5)])
        );
    }

    /// A range behaves as a list everywhere it flows: `str`, `:` indexing
    /// (wrapping like any list), `==`, and element-wise arithmetic.
    #[test]
    fn range_behaves_like_a_list() {
        assert_eq!(eval("str(range(3))"), Value::String("[0, 1, 2]".to_owned()));
        assert_eq!(eval("range(3):1"), Value::Int(1));
        assert_eq!(eval("range(5):(-1)"), Value::Int(4));
        assert_eq!(eval("range(3) == [0, 1, 2]"), Value::Bool(true));
        assert_eq!(
            eval("range(3) + 1"),
            Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );
    }

    /// The list is lazy: a billion-element range is created and randomly indexed
    /// (including from the end) without ever realising its elements.
    #[test]
    fn range_is_lazy() {
        assert_eq!(eval("range(1000000000):5"), Value::Int(5));
        assert_eq!(eval("range(1000000000):(-1)"), Value::Int(999999999));
    }

    /// `range` takes one to three arguments, each numeric.
    #[test]
    fn range_argument_errors() {
        assert!(matches!(eval_err("range()"), VmError::WrongArgCount));
        assert!(matches!(
            eval_err("range(1, 2, 3, 4)"),
            VmError::WrongArgCount
        ));
        assert!(matches!(eval_err("range('a')"), VmError::ExpectedNumber));
    }

    /// `for(list, expr)` returns how many times `expr` was truthy. Here `_i` is
    /// `0, 1, 2`, so the two non-zero indices count.
    #[test]
    fn for_returns_the_truthy_count() {
        assert_eq!(eval("for([1, 2, 3], _i)"), Value::Int(2));
    }

    /// The body runs in the current scope, so `_` is the element and a `+=`
    /// accumulates into an outer variable.
    #[test]
    fn for_binds_element_and_accumulates_in_scope() {
        assert_eq!(eval("s = 0; for([10, 20, 30], s += _); s"), Value::Int(60));
    }

    /// `for` walks a lazy `range` element by element.
    #[test]
    fn for_iterates_a_range() {
        // `_i` is 0..4, so the four non-zero indices are truthy.
        assert_eq!(eval("for(range(5), _i)"), Value::Int(4));
        assert_eq!(eval("s = 0; for(range(1, 5), s += _); s"), Value::Int(10));
    }

    /// `for` needs a list and exactly two arguments.
    #[test]
    fn for_argument_errors() {
        assert!(matches!(eval_err("for(5, _)"), VmError::ExpectedList));
        assert!(matches!(eval_err("for([1, 2])"), VmError::WrongArgCount));
        assert!(matches!(eval_err("for([1], _, _)"), VmError::WrongArgCount));
    }

    /// End to end: `for` + multi-branch `if` + `range` + `%` + `print` drive a
    /// FizzBuzz, the playground's sample program. Asserts the captured output.
    #[test]
    fn for_drives_fizzbuzz() {
        let src = "fizzbuzz(n) -> for(range(1, n + 1), if(_ % 15 == 0, print('FizzBuzz'), \
                   _ % 3 == 0, print('Fizz'), _ % 5 == 0, print('Buzz'), print(_))); \
                   fizzbuzz(5)";
        let text = eval_capturing_stdout(src);
        assert_eq!(text, "1\n2\nFizz\n4\nBuzz\n");
    }
}
