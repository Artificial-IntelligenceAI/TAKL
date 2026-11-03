//! Values held in the heap, and how `show` prints every value.
//!
//! trace: decision.memory.share
//!
//! A heap value is counted: a register holds an `Arc`'s raw pointer, or 0 for nothing. A table has two levels. The
//! table itself is what `share` hands on, so two variables sharing one see each other's changes; its items are a
//! storage that copies share until one of them changes (copy-on-write), so `= @a` is a copy without copying.
//!
//! No cycle can be made yet: a table holds copies of values, never a shared table, and only variables share. So
//! counting frees everything, and the cycle collector the decision asks for has nothing to find until a value can
//! hold a shared one; `collect_cycles` is where it runs when that arrives.

use crate::numbers::{Decimal, Size, fast};
use num_bigint::BigInt;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

pub type HeapRef = u64;

pub enum HeapValue {
    Text(TextValue),
    Table(RefCell<Arc<Storage>>),
    Decimal(Decimal),
    Fraction(BigInt, BigInt),
    /// A float128's bits.
    Quad(u128),
}

// trace: decision.text.letters
pub struct TextValue {
    pub text: String,
    /// Letters as a kid sees them: grapheme clusters.
    pub letters: usize,
}

impl TextValue {
    pub fn new(text: String) -> TextValue {
        let letters = text.graphemes(true).count();
        TextValue { text, letters }
    }
}

/// A table's items. Heap items are counted holders, released when the storage goes.
pub struct Storage {
    pub items: Vec<u64>,
    pub item_heap: bool,
    /// For `table of text to ...`: each item's key, in the order they were put in, and where each key is.
    pub keys: Option<(Vec<String>, HashMap<String, usize>)>,
}

impl Clone for Storage {
    fn clone(&self) -> Storage {
        if self.item_heap {
            for item in &self.items {
                retain(*item);
            }
        }
        Storage { items: self.items.clone(), item_heap: self.item_heap, keys: self.keys.clone() }
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        if self.item_heap {
            for item in &self.items {
                release(*item);
            }
        }
    }
}

pub fn new_heap(value: HeapValue) -> HeapRef {
    Arc::into_raw(Arc::new(value)) as HeapRef
}

pub fn retain(held: HeapRef) {
    if held != 0 {
        // SAFETY: a nonzero heap register always holds a pointer from `new_heap` that is still counted.
        unsafe { Arc::increment_strong_count(held as *const HeapValue) }
    }
}

pub fn release(held: HeapRef) {
    if held != 0 {
        // SAFETY: as in `retain`; this gives up the holder's count.
        unsafe { Arc::decrement_strong_count(held as *const HeapValue) }
    }
}

/// The value a heap register holds. Every heap register is filled before it is read, as the bytecode is made.
pub fn heap<'a>(held: HeapRef) -> &'a HeapValue {
    // SAFETY: as in `retain`; the value lives while the register holding it does.
    unsafe { &*(held as *const HeapValue) }
}

pub fn new_text(text: String) -> HeapRef {
    new_heap(HeapValue::Text(TextValue::new(text)))
}

pub fn new_table(keyed: bool, item_heap: bool) -> HeapRef {
    let keys = if keyed { Some((Vec::new(), HashMap::new())) } else { None };
    new_heap(HeapValue::Table(RefCell::new(Arc::new(Storage { items: Vec::new(), item_heap, keys }))))
}

pub fn text_of<'a>(held: HeapRef) -> &'a TextValue {
    match heap(held) {
        HeapValue::Text(text) => text,
        _ => &EMPTY_TEXT,
    }
}

static EMPTY_TEXT: TextValue = TextValue { text: String::new(), letters: 0 };

pub fn decimal_of<'a>(held: HeapRef) -> &'a Decimal {
    match heap(held) {
        HeapValue::Decimal(decimal) => decimal,
        _ => &ZERO,
    }
}

static ZERO: std::sync::LazyLock<Decimal> = std::sync::LazyLock::new(|| Decimal::from_whole(BigInt::from(0)));

pub fn quad_of(held: HeapRef) -> u128 {
    match heap(held) {
        HeapValue::Quad(bits) => *bits,
        _ => 0,
    }
}

/// The storage of a table, to read.
pub fn storage(held: HeapRef) -> Arc<Storage> {
    match heap(held) {
        HeapValue::Table(cell) => cell.borrow().clone(),
        _ => Arc::new(Storage { items: Vec::new(), item_heap: false, keys: None }),
    }
}

pub fn table_length(held: HeapRef) -> usize {
    match heap(held) {
        HeapValue::Table(cell) => cell.borrow().items.len(),
        _ => 0,
    }
}

/// Changes a table's own items, copying them first if a copy of the table still shares them.
pub fn change_table<R>(held: HeapRef, change: impl FnOnce(&mut Storage) -> R) -> Option<R> {
    match heap(held) {
        HeapValue::Table(cell) => {
            let mut storage = cell.borrow_mut();
            Some(change(Arc::make_mut(&mut storage)))
        }
        _ => None,
    }
}

// trace: decision.memory.share
/// `= @a` for a table: a new table whose items are the same storage until one of them changes.
pub fn copy_table(held: HeapRef) -> HeapRef {
    match heap(held) {
        HeapValue::Table(cell) => new_heap(HeapValue::Table(RefCell::new(cell.borrow().clone()))),
        _ => {
            retain(held);
            held
        }
    }
}

/// Where the cycle collector would run; see the module's comment for why it finds nothing yet.
pub fn collect_cycles() -> usize {
    0
}

/// A letter of text, counting from 0.
pub fn letter_at(text: &str, offset: usize) -> Option<String> {
    text.graphemes(true).nth(offset).map(str::to_string)
}

// ---- show ----

use crate::bytecode::{Held, WHOLE_KINDS};
use crate::types::Type;

/// A whole number of a kind, from its register bits.
pub fn whole_value(kind: u8, bits: u64) -> i128 {
    if kind == crate::bytecode::BIG_NEVER_NEGATIVE { bits as i128 } else { bits as i64 as i128 }
}

// trace: decision.show.numbers
/// How `show` prints a value of a type, from what a register holds.
pub fn show(ty: &Type, bits: u64) -> String {
    match (crate::bytecode::held(ty), ty) {
        (Held::Whole(kind), _) => {
            let _ = WHOLE_KINDS[kind as usize];
            whole_value(kind, bits).to_string()
        }
        (Held::Fast(size), _) => fast::show(size, bits as u128),
        (Held::TrueOrFalse, _) => String::new(),
        (Held::Heap, Type::Decimal { size: Size::Big, fast: true }) => fast::show(Size::Big, quad_of(bits)),
        (Held::Heap, Type::Decimal { .. }) => decimal_of(bits).show(),
        (Held::Heap, Type::Fraction(_)) => match heap(bits) {
            HeapValue::Fraction(top, bottom) => format!("{} over {}", top, bottom),
            _ => String::new(),
        },
        (Held::Heap, Type::Text) => text_of(bits).text.clone(),
        _ => String::new(),
    }
}
