use base64::Engine as _;
use cdr_remote_protocol::identifiers::{
    TerminalWindowId, TerminalWindowObservationId, TerminalWindowReceiptId,
};
use cdr_remote_protocol::output::{
    TerminalCaptureMediaType, TerminalOutput, TerminalWindowAction, TerminalWindowActionReceipt,
    TerminalWindowEntry,
};
use chrono::Utc;
use sha2::{Digest, Sha256};

use super::types::{OwnedTerminalWindow, TerminalWindowObservation};
use super::{TerminalWindowManager, WindowState, generated_id};
use crate::terminal::TerminalError;

impl TerminalWindowManager {
    pub(super) fn capture(&self, id: &TerminalWindowId) -> Result<TerminalOutput, TerminalError> {
        let mut state = self.lock_open()?;
        let (owned, _) = self.owned(&mut state, id)?;
        let captured = self.interaction.capture(&owned)?;
        let (_, current) = self.owned(&mut state, id)?;
        let process_id = owned.window_process_id.ok_or_else(|| {
            TerminalError::Window("terminal window identity is unavailable".into())
        })?;
        let observation_id = TerminalWindowObservationId(generated_id("twobs_"));
        let identity_digest = identity_digest(&owned);
        let observation = TerminalWindowObservation {
            observation_id: observation_id.clone(),
            terminal_window_id: id.clone(),
            identity_digest: identity_digest.clone(),
            window_process_id: process_id,
            rect: captured.rect.clone(),
        };
        if !self.interaction.matches_observation(&owned, &observation) {
            return Err(fresh_capture_error());
        }
        state.observations.insert(id.0.clone(), observation);
        Ok(TerminalOutput::TerminalWindowCapture {
            window: current,
            observation_id,
            identity_digest,
            rect: captured.rect,
            media_type: TerminalCaptureMediaType::Png,
            data_base64: base64::engine::general_purpose::STANDARD.encode(captured.png),
            captured_at: Utc::now(),
        })
    }

    pub(super) fn activate(&self, id: &TerminalWindowId) -> Result<TerminalOutput, TerminalError> {
        let mut state = self.lock_open()?;
        let (owned, _) = self.owned(&mut state, id)?;
        let activated = self.interaction.activate(&owned)?;
        let (_, current) = self.owned(&mut state, id)?;
        state.observations.remove(&id.0);
        Ok(action_output(
            &owned,
            current,
            TerminalWindowAction::Activate,
            None,
            activated,
            0,
            Vec::new(),
        ))
    }

    pub(super) fn type_text(
        &self,
        id: &TerminalWindowId,
        observation_id: &TerminalWindowObservationId,
        text: &str,
    ) -> Result<TerminalOutput, TerminalError> {
        self.observed_action(
            id,
            observation_id,
            TerminalWindowAction::Type,
            u32::try_from(text.chars().count()).unwrap_or(u32::MAX),
            |owned| {
                self.interaction
                    .type_text(owned, text)
                    .map(|value| (value, Vec::new()))
            },
        )
    }

    pub(super) fn press_keys(
        &self,
        id: &TerminalWindowId,
        observation_id: &TerminalWindowObservationId,
        keys: &[String],
    ) -> Result<TerminalOutput, TerminalError> {
        self.observed_action(id, observation_id, TerminalWindowAction::Keys, 0, |owned| {
            self.interaction.press_keys(owned, keys)
        })
    }

    pub(super) fn interrupt(
        &self,
        id: &TerminalWindowId,
        observation_id: &TerminalWindowObservationId,
    ) -> Result<TerminalOutput, TerminalError> {
        self.observed_action(
            id,
            observation_id,
            TerminalWindowAction::Interrupt,
            0,
            |owned| {
                self.interaction.interrupt(owned)?;
                Ok((false, vec!["CTRL".into(), "C".into()]))
            },
        )
    }

    fn observed_action(
        &self,
        id: &TerminalWindowId,
        observation_id: &TerminalWindowObservationId,
        action: TerminalWindowAction,
        unicode_chars: u32,
        perform: impl FnOnce(&OwnedTerminalWindow) -> Result<(bool, Vec<String>), TerminalError>,
    ) -> Result<TerminalOutput, TerminalError> {
        let mut state = self.lock_open()?;
        let (owned, _) = self.owned(&mut state, id)?;
        let observation = state
            .observations
            .get(&id.0)
            .filter(|value| {
                value.observation_id == *observation_id
                    && self.interaction.matches_observation(&owned, value)
            })
            .cloned()
            .ok_or_else(fresh_capture_error)?;
        state.observations.remove(&id.0);
        let (activated, keys) = perform(&owned)?;
        let (_, current) = self.owned(&mut state, id)?;
        Ok(action_output(
            &owned,
            current,
            action,
            Some(observation.observation_id),
            activated,
            unicode_chars,
            keys,
        ))
    }

    fn owned(
        &self,
        state: &mut WindowState,
        id: &TerminalWindowId,
    ) -> Result<(OwnedTerminalWindow, TerminalWindowEntry), TerminalError> {
        let owned = state.windows.get(&id.0).cloned().ok_or_else(|| {
            TerminalError::Window("terminal window does not belong to this session".into())
        })?;
        if let Some(entry) = self.lifecycle.inspect(&owned)? {
            return Ok((owned, entry));
        }
        state.observations.remove(&id.0);
        self.lifecycle.close(&owned)?;
        state.windows.remove(&id.0);
        Err(TerminalError::Window(
            "terminal window is no longer available".into(),
        ))
    }
}

fn action_output(
    owned: &OwnedTerminalWindow,
    window: TerminalWindowEntry,
    action: TerminalWindowAction,
    observation_id: Option<TerminalWindowObservationId>,
    activated: bool,
    unicode_chars: u32,
    keys: Vec<String>,
) -> TerminalOutput {
    TerminalOutput::TerminalWindowAction {
        receipt: TerminalWindowActionReceipt {
            receipt_id: TerminalWindowReceiptId(generated_id("twrcpt_")),
            terminal_window_id: window.terminal_window_id.clone(),
            observation_id,
            identity_digest: identity_digest(owned),
            action,
            unicode_chars,
            keys,
            activated,
            completed_at: Utc::now(),
        },
        window,
    }
}

fn identity_digest(window: &OwnedTerminalWindow) -> String {
    let material = format!(
        "{}\0{}\0{}",
        window.entry.terminal_window_id.0,
        window.entry.window_id,
        window
            .window_process_id
            .map_or_else(|| "None".into(), |value| value.to_string())
    );
    format!("{:x}", Sha256::digest(material.as_bytes()))
}

fn fresh_capture_error() -> TerminalError {
    TerminalError::Window("terminal window changed after capture; take a fresh capture".into())
}
