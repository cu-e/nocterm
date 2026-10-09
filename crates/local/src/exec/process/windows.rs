//! Attach a suspended child to a job before it can spawn descendants. The
//! inner guard also kills/reaps the child if job setup fails after spawn.
use process_wrap::std::{ChildWrapper, CommandWrap, CommandWrapper, CreationFlags, JobObject};
use std::{io, process::Command};

pub(super) fn spawn(command: Command) -> io::Result<Box<dyn ChildWrapper>> {
    let mut flags = CreationFlags(Default::default());
    flags.0.0 = 0x0800_0000; // CREATE_NO_WINDOW
    CommandWrap::from(command)
        .wrap(flags)
        .wrap(Guard)
        .wrap(JobObject)
        .spawn()
}

#[derive(Debug)]
struct Guard;

impl CommandWrapper for Guard {
    fn wrap_child(
        &mut self,
        child: Box<dyn ChildWrapper>,
        _: &CommandWrap,
    ) -> io::Result<Box<dyn ChildWrapper>> {
        Ok(Box::new(OwnedChild(Some(child))))
    }
}

#[derive(Debug)]
struct OwnedChild(Option<Box<dyn ChildWrapper>>);

impl ChildWrapper for OwnedChild {
    fn inner(&self) -> &dyn ChildWrapper {
        self.0.as_deref().unwrap()
    }
    fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
        self.0.as_deref_mut().unwrap()
    }
    fn into_inner(mut self: Box<Self>) -> Box<dyn ChildWrapper> {
        self.0.take().unwrap()
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.start_kill();
            let _ = child.wait();
        }
    }
}
