use anyhow::Result;
use clap::Subcommand;

pub use crate::platform::service::LaunchdStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum ServiceCommand {
    Install,
    Uninstall,
    Start,
    Stop,
    Restart,
    Status,
}

pub fn launchd_status() -> LaunchdStatus {
    crate::platform::service::launchd_status()
}

pub async fn run(command: ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::Install => crate::platform::service::install(),
        ServiceCommand::Uninstall => crate::platform::service::uninstall(),
        ServiceCommand::Start => crate::platform::service::start(),
        ServiceCommand::Stop => crate::platform::service::stop().await,
        ServiceCommand::Restart => crate::platform::service::restart().await,
        ServiceCommand::Status => crate::platform::service::status().await,
    }
}

#[cfg(test)]
fn run_sync_with(command: ServiceCommand, install: impl FnOnce() -> Result<()>) -> Result<()> {
    match command {
        ServiceCommand::Install => install(),
        ServiceCommand::Uninstall
        | ServiceCommand::Start
        | ServiceCommand::Stop
        | ServiceCommand::Restart
        | ServiceCommand::Status => Ok(()),
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Calls(RefCell<Vec<&'static str>>);

    impl Calls {
        fn push(&self, name: &'static str) {
            self.0.borrow_mut().push(name);
        }
    }

    #[test]
    fn service_run_routes_sync_commands() {
        let calls = Calls::default();

        run_sync_with(ServiceCommand::Install, || {
            calls.push("install");
            Ok(())
        })
        .unwrap();

        assert_eq!(&*calls.0.borrow(), &["install"]);
    }
}
