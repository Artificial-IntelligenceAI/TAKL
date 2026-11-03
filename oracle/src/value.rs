//! Values while a program runs.

use crate::numbers::{Exact, Fast, Size};
use num_bigint::BigInt;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum Value {
    /// `size` is `None` for a number written in the program and not yet given a type.
    Whole {
        size: Option<Size>,
        never_negative: bool,
        n: BigInt,
    },
    Exact {
        size: Option<Size>,
        value: Exact,
    },
    Fast(Fast),
    Fraction {
        size: Option<Size>,
        top: BigInt,
        bottom: BigInt,
    },
    Text(String),
    TrueOrFalse(bool),
    /// trace: decision.memory.share (a table is shared only when `share` says so; otherwise it is copied)
    Table(Rc<RefCell<Table>>),
}

#[derive(Clone, Debug)]
pub enum Table {
    /// Keyed 1, 2, 3, in order.
    Numbered(Vec<Value>),
    /// Keyed by name, in the order the keys were put in.
    Keyed(Vec<(Value, Value)>),
}

impl Value {
    /// A copy that shares nothing with the original, as `= a` makes.
    /// trace: decision.memory.share
    pub fn copy(&self) -> Value {
        match self {
            Value::Table(table) => {
                let copied = match &*table.borrow() {
                    Table::Numbered(items) => Table::Numbered(items.iter().map(Value::copy).collect()),
                    Table::Keyed(pairs) => Table::Keyed(pairs.iter().map(|(k, v)| (k.copy(), v.copy())).collect()),
                };
                Value::Table(Rc::new(RefCell::new(copied)))
            }
            other => other.clone(),
        }
    }

    pub fn new_table(table: Table) -> Value {
        Value::Table(Rc::new(RefCell::new(table)))
    }
}
