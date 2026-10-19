//! Validation obligations survive replacement without retaining ordinary values.
use super::{DecodeLedger, JsonDecodeError, ledger::Reservation, nodes::*};
use std::mem::size_of;
#[derive(Debug)]
struct Obligation {
    input: String,
    allocation: Reservation,
}
#[derive(Debug)]
pub(super) struct Pending {
    nodes: Vec<Obligation>,
    allocation: Reservation,
}
impl Pending {
    pub(super) fn new(ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        Ok(Self {
            nodes: Vec::new(),
            allocation: ledger.reserve(0)?,
        })
    }
    pub(super) fn push_raw(
        &mut self,
        input: String,
        allocation: Reservation,
        ledger: &DecodeLedger,
    ) -> Result<(), JsonDecodeError> {
        if self.nodes.len() == self.nodes.capacity() {
            let capacity = next_capacity(self.nodes.capacity())?;
            let next = ledger.reserve(product(capacity, size_of::<Obligation>())?)?;
            self.nodes.try_reserve_exact(capacity - self.nodes.len())?;
            if self.nodes.capacity() != capacity {
                return Err(JsonDecodeError::CapacityProfile);
            }
            self.allocation = next;
        }
        self.nodes.push(Obligation { input, allocation });
        Ok(())
    }
    pub(super) fn extract(
        &mut self,
        node: Node,
        ledger: &DecodeLedger,
    ) -> Result<(), JsonDecodeError> {
        let Node {
            kind,
            allocation,
            layout: _,
        } = node;
        match kind {
            Kind::Raw(text) => self.push_raw(text, allocation, ledger),
            Kind::Array(nodes) => {
                for child in nodes {
                    self.extract(child, ledger)?;
                }
                drop(allocation);
                Ok(())
            }
            Kind::Object(nodes) => {
                for (_, child) in nodes {
                    self.extract(child, ledger)?;
                }
                drop(allocation);
                Ok(())
            }
            Kind::Literal(_) => Ok(()),
        }
    }
    pub(super) fn validate(mut self, ledger: &DecodeLedger) -> Result<(), JsonDecodeError> {
        while let Some(Obligation {
            input: text,
            allocation,
        }) = self.nodes.pop()
        {
            let reader = super::chunks::OwnedChunks::from_reserved(text, allocation, ledger)?;
            let mut decoder = serde_json::Deserializer::from_reader(reader);
            let mut children = Self::new(ledger)?;
            let mut failure = None;
            let parsed = serde::de::DeserializeSeed::deserialize(
                super::validation::Validate {
                    ledger,
                    pending: &mut children,
                    failure: &mut failure,
                },
                &mut decoder,
            );
            if let Some(error) = failure {
                return Err(error);
            }
            parsed?;
            decoder.end()?;
            drop(decoder);
            // Drain only after parser and input have both retired.
            for child in children.nodes.drain(..) {
                self.push_raw(child.input, child.allocation, ledger)?;
            }
        }
        Ok(())
    }
}
