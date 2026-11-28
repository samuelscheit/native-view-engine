//! Unified IME composition (`CompositionStart` / `CompositionUpdate` / `CompositionEnd`)
//! lifecycle dispatch for [`DesktopSession`].

use nve_abi::{EventBatchBuf, NodeId, TextInputEvent, TimestampUs};
use winit::event::Ime;

use super::session::{now_us, DesktopSession};
use super::text_edit::insert_text_at;
use super::{DesktopApp, DesktopHostError};

impl<A: DesktopApp> DesktopSession<A> {
    pub(super) fn dispatch_and_redraw(
        &mut self,
        events: &EventBatchBuf,
    ) -> Result<(), DesktopHostError> {
        if self.app.dispatch_events(events.as_validated()) {
            self.render_and_rasterize()?;
        } else {
            self.rasterize_current();
        }
        Ok(())
    }

    pub(super) fn push_composition_event(
        &self,
        events: &mut EventBatchBuf,
        node_id: NodeId,
        ts: TimestampUs,
        event: TextInputEvent,
        text: &str,
    ) {
        events.push_composition(self.surface_id, node_id, ts, event, text);
    }

    pub(super) fn emit_text_change(
        &mut self,
        node_id: NodeId,
        new_value: String,
        new_caret: usize,
        composition_end: Option<String>,
    ) -> Result<bool, DesktopHostError> {
        {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.set_ime_preedit(store, None, None);
            backend.set_uncontrolled_text_input(store, Some(new_value.clone()), new_caret);
        }
        self.mark_a11y_dirty(node_id);
        let ts = TimestampUs(now_us());
        let mut events = EventBatchBuf::with_capacity(2, new_value.len());
        if let Some(committed) = composition_end {
            self.push_composition_event(
                &mut events,
                node_id,
                ts,
                TextInputEvent::CompositionEnd,
                &committed,
            );
        }
        events.push_text_change(self.surface_id, node_id, ts, &new_value);
        self.dispatch_and_redraw(&events)?;
        Ok(true)
    }

    /// Handles `winit::event::Ime` (`Preedit` / `Commit` / `Disabled`) for the focused `TextInput`,
    /// emitting unified `CompositionStart` / `CompositionUpdate` / `CompositionEnd` events.
    pub fn handle_ime(&mut self, ime: &Ime) -> Result<bool, DesktopHostError> {
        self.reset_multi_click();
        let Some(focused_id) = self
            .replica
            .backend()
            .focused_node()
            .filter(|&id| {
                self.replica
                    .backend()
                    .is_text_input(self.replica.store(), id)
            })
        else {
            return Ok(false);
        };

        match ime {
            Ime::Enabled => Ok(false),
            Ime::Preedit(text, cursor) => {
                let was_composing = self.replica.backend().focus_state().ime_preedit.is_some();
                if text.is_empty() {
                    if !was_composing {
                        return Ok(false);
                    }
                    {
                        let (store, backend) = self.replica.store_and_backend_mut();
                        backend.set_ime_preedit(store, None, None);
                    }
                    self.mark_a11y_dirty(focused_id);
                    let mut events = EventBatchBuf::with_capacity(1, 0);
                    self.push_composition_event(
                        &mut events,
                        focused_id,
                        TimestampUs(now_us()),
                        TextInputEvent::CompositionEnd,
                        "",
                    );
                    self.dispatch_and_redraw(&events)?;
                    Ok(true)
                } else {
                    {
                        let (store, backend) = self.replica.store_and_backend_mut();
                        backend.set_ime_preedit(store, Some(text.clone()), *cursor);
                    }
                    self.mark_a11y_dirty(focused_id);
                    let kind = if was_composing {
                        TextInputEvent::CompositionUpdate
                    } else {
                        TextInputEvent::CompositionStart
                    };
                    let mut events = EventBatchBuf::with_capacity(1, text.len());
                    self.push_composition_event(
                        &mut events,
                        focused_id,
                        TimestampUs(now_us()),
                        kind,
                        text,
                    );
                    self.dispatch_and_redraw(&events)?;
                    Ok(true)
                }
            }
            Ime::Commit(committed) => {
                let had_preedit = self.replica.backend().focus_state().ime_preedit.is_some();
                if committed.is_empty() {
                    if !had_preedit {
                        return Ok(false);
                    }
                    {
                        let (store, backend) = self.replica.store_and_backend_mut();
                        backend.set_ime_preedit(store, None, None);
                    }
                    self.mark_a11y_dirty(focused_id);
                    let mut events = EventBatchBuf::with_capacity(1, 0);
                    self.push_composition_event(
                        &mut events,
                        focused_id,
                        TimestampUs(now_us()),
                        TextInputEvent::CompositionEnd,
                        "",
                    );
                    self.dispatch_and_redraw(&events)?;
                    return Ok(true);
                }
                let current = self
                    .replica
                    .backend()
                    .text_input_value(self.replica.store(), focused_id)
                    .unwrap_or_default();
                let caret = self.replica.backend().focus_state().caret_byte_index;
                let selection = self.replica.backend().focus_state().selection_range();
                let (next_val, next_caret) =
                    insert_text_at(&current, caret, selection, committed);
                self.emit_text_change(
                    focused_id,
                    next_val,
                    next_caret,
                    had_preedit.then(|| committed.clone()),
                )
            }
            Ime::Disabled => {
                let had = self.replica.backend().focus_state().ime_preedit.is_some();
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_ime_preedit(store, None, None);
                }
                if !had {
                    return Ok(false);
                }
                self.mark_a11y_dirty(focused_id);
                let mut events = EventBatchBuf::with_capacity(1, 0);
                self.push_composition_event(
                    &mut events,
                    focused_id,
                    TimestampUs(now_us()),
                    TextInputEvent::CompositionEnd,
                    "",
                );
                self.dispatch_and_redraw(&events)?;
                Ok(true)
            }
        }
    }
}
