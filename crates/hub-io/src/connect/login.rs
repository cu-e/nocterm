//! A successful sign-in hands its original native cleanup authority to the consumer.
use crate::{account::source::CapturedMembership, pool::Pool};
use nocterm_hubs_model::{HubError, LoginOwner, LoginReceipt};
use std::sync::{Arc, Weak};
struct Owner {
    pool: Weak<Pool>,
    member: Arc<CapturedMembership>,
}
impl LoginOwner for Owner {
    fn check_current(&self) -> Result<(), HubError> {
        self.member.check()
    }
    fn discard(&self) -> Result<(), HubError> {
        self.member.check()?;
        let pool = self.pool.upgrade().ok_or(HubError::Stale)?;
        let subject = self.member.subject()?;
        let (_, result) = pool
            .end_membership(self.member.hub(), &subject, false)?
            .into_parts();
        result
    }
}
pub(super) fn receipt(pool: &Arc<Pool>, member: Arc<CapturedMembership>) -> LoginReceipt {
    LoginReceipt::new(
        member.hub().clone(),
        Arc::new(Owner {
            pool: Arc::downgrade(pool),
            member,
        }),
    )
}
