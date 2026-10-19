use crate::{
    budget::{AllocationBudget, AllocationRefused, BudgetVec},
    compress_to_vec, decompress_to_vec_bounded_with_budget, Decompressor,
    ManagedDecompressionError,
};
use std::{cell::Cell, rc::Rc};

#[derive(Clone)]
struct Account(Rc<State>);
struct State {
    live: Cell<usize>,
    peak: Cell<usize>,
    limit: usize,
    refused: Cell<bool>,
}
impl Account {
    fn new(limit: usize) -> Self {
        Self(Rc::new(State {
            live: Cell::new(0),
            peak: Cell::new(0),
            limit,
            refused: Cell::new(false),
        }))
    }
}
impl AllocationBudget for Account {
    fn admit(&self, bytes: usize) -> Result<(), AllocationRefused> {
        let next = self
            .0
            .live
            .get()
            .checked_add(bytes)
            .ok_or(AllocationRefused)?;
        if next > self.0.limit {
            self.0.refused.set(true);
            return Err(AllocationRefused);
        }
        self.0.live.set(next);
        self.0.peak.set(next.max(self.0.peak.get()));
        Ok(())
    }
    fn retire(&self, bytes: usize) {
        self.0
            .live
            .set(self.0.live.get().checked_sub(bytes).unwrap());
    }
}

#[test]
fn original_huffman_boxes_are_admitted_before_decoder_construction() {
    let bytes = std::mem::size_of::<[u32; 4096]>() + std::mem::size_of::<[u32; 512]>();
    let account = Account::new(bytes - 1);
    assert!(Decompressor::new_with_budget(account.clone()).is_err());
    assert!(account.0.refused.get());
    assert_eq!(account.0.live.get(), 0);
    let account = Account::new(bytes);
    let decoder = Decompressor::new_with_budget(account.clone()).unwrap();
    assert_eq!(account.0.live.get(), bytes);
    drop(decoder);
    assert_eq!(account.0.live.get(), 0);
}

#[test]
fn vector_growth_pays_both_backings_and_refusal_preserves_original() {
    let account = Account::new(12);
    let mut value = BudgetVec::new(account.clone()).unwrap();
    value.resize(4, 7u8).unwrap();
    value.resize(8, 9).unwrap();
    assert_eq!(&*value, &[7, 7, 7, 7, 9, 9, 9, 9]);
    assert_eq!(account.0.peak.get(), 12);
    assert_eq!(account.0.live.get(), 8);
    assert!(value.resize(9, 11).is_err());
    assert_eq!(&*value, &[7, 7, 7, 7, 9, 9, 9, 9]);
    assert_eq!(account.0.live.get(), 8);
    drop(value);
    assert_eq!(account.0.live.get(), 0);
}

#[test]
fn managed_original_parser_keeps_output_and_partial_output_paid() {
    let source = vec![15u8; 40000];
    let encoded = compress_to_vec(&source);
    let account = Account::new(200000);
    let output =
        match decompress_to_vec_bounded_with_budget(&encoded, source.len(), account.clone()) {
            Ok(value) => value,
            Err(_) => panic!("managed original parser rejected valid stream"),
        };
    assert_eq!(&*output, source);
    let (_, credit) = output.into_parts();
    // The moved vector is dropped at this statement, before its explicit receipt.
    assert!(account.0.live.get() >= source.len());
    drop(credit);
    assert_eq!(account.0.live.get(), 0);
    let partial = match decompress_to_vec_bounded_with_budget(&encoded, 1024, account.clone()) {
        Err(ManagedDecompressionError::OutputTooLarge(value)) => value,
        _ => panic!("original bounded output rule changed"),
    };
    assert_eq!(&*partial, &source[..1024]);
    assert_eq!(account.0.live.get(), 1024);
    drop(partial);
    assert_eq!(account.0.live.get(), 0);
}

#[test]
fn payload_destructor_panic_still_retires_its_actual_backing() {
    #[derive(Clone)]
    struct Payload(Account);
    impl Drop for Payload {
        fn drop(&mut self) {
            assert!(self.0 .0.live.get() >= std::mem::size_of::<Self>());
            panic!("payload destructor");
        }
    }
    let account = Account::new(1000);
    let mut owned = BudgetVec::new(account.clone()).unwrap();
    // Append one element without constructing an extra destructor-panic temporary.
    owned.allocation.reserve(&mut owned.value, 1).unwrap();
    owned.value.push(Payload(account.clone()));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(owned))).is_err());
    assert_eq!(account.0.live.get(), 0);
}
