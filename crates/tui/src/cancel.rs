use crate::{App, TuiError};

impl App {
    pub(crate) fn request_cancel(&mut self) {
        let Some(run) = self.runs.get(self.selected) else {
            return;
        };
        if !run.service.as_ref().is_some_and(is_cancelable) {
            self.notice = Some("this run cannot be canceled".to_owned());
            self.dirty = true;
            return;
        }
        self.confirm_cancel = Some(run.name.clone());
        self.notice = Some(format!("cancel {}? press Enter to confirm", run.name));
        self.dirty = true;
    }

    pub(crate) fn send_cancel(&mut self) -> Result<(), TuiError> {
        let Some(name) = self.confirm_cancel.take() else {
            return Ok(());
        };
        let endpoint = kairo_control::load_endpoint(std::path::Path::new(".kairo"))
            .map_err(|source| TuiError::Control { source })?;
        kairo_control::cancel(&endpoint, name.clone())
            .map_err(|source| TuiError::Control { source })?;
        self.notice = Some(format!("cancel requested · {name}"));
        self.refresh()
    }
}

fn is_cancelable(status: &kairo_control::RunStatus) -> bool {
    !matches!(
        status,
        kairo_control::RunStatus::Completed { .. }
            | kairo_control::RunStatus::Failed { .. }
            | kairo_control::RunStatus::Canceled
    )
}
