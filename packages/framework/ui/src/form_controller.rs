//! Reusable headless [`FormController`] combining [`FormState`] validation/dirty tracking with
//! automatic draft saving, restoration, and post-submit clearing via [`FormDraftStorage`].

use std::collections::BTreeMap;

use crate::form::FormState;
use crate::form_draft::{
    decode_form_draft, default_draft_storage, encode_form_draft, FormDraftStorage,
    DEFAULT_MAX_FORM_DRAFT_BYTES,
};

/// Reusable headless controller combining [`FormState`] validation/dirty tracking with automatic
/// draft saving and restoration via [`FormDraftStorage`].
pub struct FormController {
    state: FormState,
    persist_key: Option<String>,
    storage: Box<dyn FormDraftStorage>,
    restored_draft: bool,
    max_draft_bytes: usize,
}

impl Default for FormController {
    fn default() -> Self {
        Self::new()
    }
}

impl FormController {
    pub fn new() -> Self {
        Self {
            state: FormState::new(),
            persist_key: None,
            storage: default_draft_storage(),
            restored_draft: false,
            max_draft_bytes: DEFAULT_MAX_FORM_DRAFT_BYTES,
        }
    }

    pub fn with_persistence<S: FormDraftStorage + 'static>(
        persist_key: impl Into<String>,
        storage: S,
    ) -> Self {
        Self {
            state: FormState::new(),
            persist_key: Some(persist_key.into()),
            storage: Box::new(storage),
            restored_draft: false,
            max_draft_bytes: DEFAULT_MAX_FORM_DRAFT_BYTES,
        }
    }

    pub fn with_default_persistence(persist_key: impl Into<String>) -> Self {
        Self {
            state: FormState::new(),
            persist_key: Some(persist_key.into()),
            storage: default_draft_storage(),
            restored_draft: false,
            max_draft_bytes: DEFAULT_MAX_FORM_DRAFT_BYTES,
        }
    }

    #[inline]
    pub fn persist_key(&self) -> Option<&str> {
        self.persist_key.as_deref()
    }

    #[inline]
    pub fn state(&self) -> &FormState {
        &self.state
    }

    #[inline]
    pub fn state_mut(&mut self) -> &mut FormState {
        &mut self.state
    }

    /// Registers a text field with its clean baseline value.
    pub fn register_text_field(
        &mut self,
        name: impl Into<String>,
        initial: impl Into<String>,
    ) -> &mut Self {
        self.state.register_field(name, initial.into());
        self
    }

    /// Attaches a synchronous validator to a registered text field.
    pub fn add_text_validator<F>(&mut self, name: &str, validator: F) -> bool
    where
        F: Fn(&String) -> Result<(), String> + 'static,
    {
        self.state.add_field_validator::<String, F>(name, validator)
    }

    /// Returns the current text value of `name` (or empty string if unregistered).
    pub fn text_value(&self, name: &str) -> String {
        self.state
            .value::<String>(name)
            .cloned()
            .unwrap_or_default()
    }

    /// Updates a text field (auto-registering with empty baseline if absent), runs validators,
    /// and automatically saves the dirty draft when `persist_key` is configured.
    pub fn set_text(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        if self.state.field::<String>(name).is_none() {
            self.state.register_field(name, String::new());
        }
        let _ = self.state.set_value(name, value);
        self.sync_draft_persistence();
    }

    pub fn touch(&mut self, name: &str) -> bool {
        self.state.touch(name)
    }

    /// Restores any previously saved draft for `persist_key` into registered (or new) fields,
    /// preserving each field's clean baseline so [`Self::is_dirty`] reflects unsaved changes.
    pub fn restore_draft(&mut self) -> bool {
        let Some(key) = self.persist_key.clone() else {
            return false;
        };
        let Some(raw) = self.storage.load_draft(&key) else {
            return false;
        };
        let Some(fields) = decode_form_draft(&raw) else {
            return false;
        };
        let mut applied_any = false;
        for (name, val) in fields {
            if self.state.field::<String>(&name).is_none() {
                self.state.register_field(name.clone(), String::new());
            }
            if self.state.set_value(&name, val) {
                applied_any = true;
            }
        }
        if applied_any {
            self.restored_draft = true;
        }
        applied_any
    }

    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.state.is_dirty()
    }

    #[inline]
    pub fn can_pop(&self) -> bool {
        self.state.can_pop()
    }

    #[inline]
    pub fn has_restored_draft(&self) -> bool {
        self.restored_draft
    }

    /// Explicitly removes any saved draft and clears the restored-draft flag.
    pub fn clear_draft(&mut self) {
        if let Some(ref key) = self.persist_key {
            self.storage.clear_draft(key);
        }
        self.restored_draft = false;
    }

    pub fn error(&self, name: &str) -> Option<&str> {
        self.state.error(name)
    }

    pub fn visible_error(&self, name: &str) -> Option<&str> {
        self.state.visible_error(name)
    }

    pub fn is_valid(&self) -> bool {
        self.state.is_valid()
    }

    pub fn is_submitted(&self) -> bool {
        self.state.is_submitted()
    }

    /// Collects all registered text field values as a deterministic map.
    pub fn field_values(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for id in self.state.field_ids() {
            if let Some(val) = self.state.value::<String>(id) {
                map.insert(id.clone(), val.clone());
            }
        }
        map
    }

    /// Runs full validation; if valid, invokes `on_valid` and clears the saved draft + commits the
    /// clean baseline ONLY when `on_valid` returns `Ok(())`.
    pub fn submit_with<F>(&mut self, on_valid: F) -> Result<BTreeMap<String, String>, Option<String>>
    where
        F: FnOnce(&BTreeMap<String, String>) -> Result<(), String>,
    {
        if !self.state.submit() {
            return Err(None);
        }
        let values = self.field_values();
        match on_valid(&values) {
            Ok(()) => {
                self.state.commit_clean_baseline();
                self.clear_draft();
                Ok(values)
            }
            Err(err) => Err(Some(err)),
        }
    }

    /// Resets all fields back to their clean baseline and clears any persisted draft.
    pub fn reset(&mut self) {
        self.state.reset_to_clean();
        self.clear_draft();
    }

    fn sync_draft_persistence(&mut self) {
        let Some(key) = self.persist_key.clone() else {
            return;
        };
        if !self.state.is_dirty() {
            self.storage.clear_draft(&key);
            self.restored_draft = false;
            return;
        }
        let encoded = encode_form_draft(&self.field_values());
        if encoded.len() <= self.max_draft_bytes {
            let _ = self.storage.save_draft(&key, &encoded);
        }
    }
}
