//! Shared Arc allocation layout; dynamic JSON reserve comes from JsonReceipt.
use std::{mem::size_of, sync::atomic::AtomicUsize};
// std ArcInner is repr(C), with these two atomic counts before its aligned data.
#[repr(C)]
struct Shared<T> {
    _strong: AtomicUsize,
    _weak: AtomicUsize,
    _value: T,
}
pub fn shared_inline<T>() -> Option<usize> {
    Some(shared_inline_bytes::<T>())
}
pub(crate) const fn shared_inline_bytes<T>() -> usize {
    size_of::<Shared<T>>()
}
