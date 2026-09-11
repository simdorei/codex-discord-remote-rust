//! File sessions do not require a native UI backend. Create it only for a UI request.
use super::{ComputerAccessMode, ComputerController, ComputerError, new_computer_controller};
use cdr_remote_protocol::{output::ComputerOutput, request::ComputerRequest};
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct SessionComputer {
    mode: ComputerAccessMode,
    controller: OnceLock<Result<ComputerController, ComputerError>>,
    stopped: AtomicBool,
    factory: fn(ComputerAccessMode) -> Result<ComputerController, ComputerError>,
}

impl SessionComputer {
    pub(crate) fn new(mode: ComputerAccessMode) -> Self {
        Self {
            mode,
            controller: OnceLock::new(),
            stopped: AtomicBool::new(false),
            factory: new_computer_controller,
        }
    }

    pub(crate) fn execute(
        &self,
        request: &ComputerRequest,
    ) -> Result<ComputerOutput, ComputerError> {
        if matches!(request, ComputerRequest::ComputerStop) {
            self.stopped.store(true, Ordering::Release);
            if let Some(Ok(controller)) = self.controller.get() {
                return controller.execute(request);
            }
            return Ok(ComputerOutput::ComputerStop {
                stopped: true,
                message: "Computer control stopped until this project is bound again.".into(),
            });
        }
        self.ensure_running()?;
        let controller = self.controller.get_or_init(|| (self.factory)(self.mode));
        // A stop racing with initialization must not permit the first UI action.
        self.ensure_running()?;
        controller.as_ref().map_err(Clone::clone)?.execute(request)
    }

    fn ensure_running(&self) -> Result<(), ComputerError> {
        if self.stopped.load(Ordering::Acquire) {
            Err(ComputerError::Stopped)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn unavailable(_: ComputerAccessMode) -> Result<ComputerController, ComputerError> {
        Err(ComputerError::Platform(
            "injected unsupported UI platform".into(),
        ))
    }
    fn fixture() -> SessionComputer {
        SessionComputer {
            factory: unavailable,
            ..SessionComputer::new(ComputerAccessMode::Device)
        }
    }
    #[test]
    fn creation_and_file_only_cleanup_do_not_initialize_unsupported_ui() {
        let session = fixture();
        assert!(session.controller.get().is_none());
        assert!(matches!(
            session.execute(&ComputerRequest::ComputerStop),
            Ok(ComputerOutput::ComputerStop { stopped: true, .. })
        ));
        assert!(session.controller.get().is_none());
    }
    #[test]
    fn requesting_ui_surfaces_exact_platform_failure_without_another_backend() {
        let session = fixture();
        for _ in 0..2 {
            assert_eq!(
                session
                    .execute(&ComputerRequest::ComputerListWindows)
                    .unwrap_err(),
                ComputerError::Platform("injected unsupported UI platform".into())
            );
        }
        assert!(session.controller.get().unwrap().is_err());
    }
    #[test]
    fn stop_before_first_ui_request_is_permanent_for_this_session() {
        let session = fixture();
        session.execute(&ComputerRequest::ComputerStop).unwrap();
        assert_eq!(
            session
                .execute(&ComputerRequest::ComputerListWindows)
                .unwrap_err(),
            ComputerError::Stopped
        );
        assert!(session.controller.get().is_none());
    }
}
