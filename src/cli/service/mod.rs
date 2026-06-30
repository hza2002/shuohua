use anyhow::Result;
use clap::Subcommand;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(target_os = "macos"))]
use unsupported as platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum ServiceCommand {
    Install,
    Uninstall,
    Start,
    Stop,
    Restart,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchdStatus {
    Installed(std::path::PathBuf),
    NotInstalled(std::path::PathBuf),
    #[cfg(not(target_os = "macos"))]
    Unsupported,
}

pub fn launchd_status() -> LaunchdStatus {
    platform::launchd_status()
}

/// 已安装 launchd plist 中 `ProgramArguments` 的第一个值（daemon 实际拉起的
/// binary 绝对路径）。用于安装路径漂移诊断；未安装则为 `None`。
pub fn plist_program() -> Option<std::path::PathBuf> {
    platform::plist_program()
}

pub async fn run(command: ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::Install => platform::install().await,
        ServiceCommand::Uninstall => platform::uninstall(),
        ServiceCommand::Start => platform::start().await,
        ServiceCommand::Stop => platform::stop().await,
        ServiceCommand::Restart => platform::restart().await,
        ServiceCommand::Status => platform::status().await,
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
