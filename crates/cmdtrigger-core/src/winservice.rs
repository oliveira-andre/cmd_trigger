//! The Windows service entry point (`--service <folder>`): report to the service manager, run
//! the background agent, and stop when asked.

use std::ffi::OsString;
use std::sync::OnceLock;
use std::time::Duration;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

static FOLDER: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

/// Hand the process to the service manager. Blocks until the service stops.
pub fn run(folder: Option<std::path::PathBuf>) -> windows_service::Result<()> {
    let _ = FOLDER.set(folder);
    service_dispatcher::start("CmdTriggerAgent", ffi_service_main)
}

fn service_main(_args: Vec<OsString>) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let stop = tokio_util::sync::CancellationToken::new();
    let stop_signal = stop.clone();
    let handler = move |event| match event {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            stop_signal.cancel();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let Ok(status) = service_control_handler::register("CmdTriggerAgent", handler) else { return };
    let report = |state, accept| {
        let _ = status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        });
    };
    report(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN);
    runtime.block_on(async {
        crate::logging::init(None);
        let folder = FOLDER.get().cloned().flatten();
        let Ok(data) = crate::DataDir::resolve(folder.as_deref()) else { return };
        let agent = crate::Agent::start(data, crate::Ground::Background).await;
        stop.cancelled().await;
        agent.shutdown().await;
    });
    report(ServiceState::Stopped, ServiceControlAccept::empty());
}
