//! Headless type-safe [`FormState`] and [`FieldState<T>`] validation, dirty-tracking,
//! Android Predictive Back (`can_pop()`), cross-field validation, and generation-guarded
//! async validation state machine.

use std::any::Any;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Monotonic token tying an in-flight asynchronous validation to a specific field and value
/// revision so stale out-of-order async results are dropped cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValidationToken {
    pub field_id: String,
    pub generation: u64,
}

/// Strongly typed single-field state tracking clean baseline vs current value, touched/submitted
/// visibility, sync/cross-field/async errors, and async validation generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldState<T> {
    id: String,
    initial: T,
    value: T,
    touched: bool,
    submitted: bool,
    sync_error: Option<String>,
    cross_error: Option<String>,
    async_error: Option<String>,
    async_generation: u64,
    validating: bool,
}

impl<T: Clone + PartialEq> FieldState<T> {
    /// Creates a clean, untouched field with `initial` as both its baseline and current value.
    pub fn new(id: impl Into<String>, initial: T) -> Self {
        Self {
            id: id.into(),
            value: initial.clone(),
            initial,
            touched: false,
            submitted: false,
            sync_error: None,
            cross_error: None,
            async_error: None,
            async_generation: 0,
            validating: false,
        }
    }

    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[inline]
    pub fn value(&self) -> &T {
        &self.value
    }

    #[inline]
    pub fn initial(&self) -> &T {
        &self.initial
    }

    /// Updates the current field value, bumps the async generation counter (invalidating any
    /// in-flight async checks), and clears any stale async error.
    pub fn set_value(&mut self, value: T) {
        self.value = value;
        self.async_generation = self.async_generation.wrapping_add(1);
        self.async_error = None;
        self.validating = false;
    }

    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.value != self.initial
    }

    #[inline]
    pub fn is_touched(&self) -> bool {
        self.touched
    }

    #[inline]
    pub fn is_submitted(&self) -> bool {
        self.submitted
    }

    /// Marks the field as touched (typically on blur), allowing errors to become visible.
    #[inline]
    pub fn mark_touched(&mut self) {
        self.touched = true;
    }

    /// Marks the field as submitted, allowing errors to become visible even if untouched.
    #[inline]
    pub fn mark_submitted(&mut self) {
        self.submitted = true;
    }

    /// Runs a synchronous validator against the current value and stores its result.
    pub fn validate_sync<F>(&mut self, validator: F) -> bool
    where
        F: Fn(&T) -> Result<(), String>,
    {
        match validator(&self.value) {
            Ok(()) => {
                self.sync_error = None;
                true
            }
            Err(msg) => {
                self.sync_error = Some(msg);
                false
            }
        }
    }

    /// Explicitly sets or clears the cross-field validation error attached to this field.
    #[inline]
    pub fn set_cross_error(&mut self, error: Option<String>) {
        self.cross_error = error;
    }

    /// Starts an asynchronous validation cycle and returns a [`ValidationToken`] stamped with the
    /// field's current generation. Any earlier tokens become stale immediately.
    pub fn begin_async_validation(&mut self) -> ValidationToken {
        self.async_generation = self.async_generation.wrapping_add(1);
        self.validating = true;
        ValidationToken {
            field_id: self.id.clone(),
            generation: self.async_generation,
        }
    }

    /// Applies an asynchronous validation outcome only if `token` matches both `field_id` and the
    /// active `generation`. Returns `true` if applied, or `false` if dropped as stale.
    pub fn apply_async_result(
        &mut self,
        token: &ValidationToken,
        result: Result<(), String>,
    ) -> bool {
        if token.field_id != self.id || token.generation != self.async_generation {
            return false;
        }
        self.validating = false;
        self.async_error = result.err();
        true
    }

    #[inline]
    pub fn is_validating(&self) -> bool {
        self.validating
    }

    #[inline]
    pub fn async_generation(&self) -> u64 {
        self.async_generation
    }

    /// Returns the highest-priority active error regardless of `touched`/`submitted` state.
    #[inline]
    pub fn error(&self) -> Option<&str> {
        self.sync_error
            .as_deref()
            .or(self.cross_error.as_deref())
            .or(self.async_error.as_deref())
    }

    /// Returns the active error only once the field has been blurred (`touched`) or the form has
    /// been submitted (`submitted`), preventing aggressive error banners on the first keystroke.
    #[inline]
    pub fn visible_error(&self) -> Option<&str> {
        if self.touched || self.submitted {
            self.error()
        } else {
            None
        }
    }

    #[inline]
    pub fn is_valid(&self) -> bool {
        self.error().is_none()
    }

    /// Restores the field to its clean baseline, clears errors and interaction flags, and
    /// increments the async generation to discard any in-flight async checks.
    pub fn reset_to_clean(&mut self) {
        self.value = self.initial.clone();
        self.touched = false;
        self.submitted = false;
        self.sync_error = None;
        self.cross_error = None;
        self.async_error = None;
        self.validating = false;
        self.async_generation = self.async_generation.wrapping_add(1);
    }

    /// Commits the current value as the new clean baseline (e.g. after a successful save) and
    /// clears `touched` and `submitted` flags.
    pub fn commit_clean_baseline(&mut self) {
        self.initial = self.value.clone();
        self.touched = false;
        self.submitted = false;
    }
}

type SyncValidator<T> = Box<dyn Fn(&T) -> Result<(), String>>;
type CrossValidatorFn = Rc<dyn Fn(&FormState) -> Result<(), String>>;

struct TypedFieldSlot<T> {
    state: FieldState<T>,
    validators: Vec<SyncValidator<T>>,
}

trait DynField {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn is_dirty(&self) -> bool;
    fn is_touched(&self) -> bool;
    fn mark_touched(&mut self);
    fn mark_submitted(&mut self);
    fn run_sync_validators(&mut self) -> bool;
    fn set_cross_error(&mut self, error: Option<String>);
    fn begin_async_validation(&mut self) -> ValidationToken;
    fn apply_async_result(&mut self, token: &ValidationToken, result: Result<(), String>) -> bool;
    fn is_validating(&self) -> bool;
    fn error(&self) -> Option<&str>;
    fn visible_error(&self) -> Option<&str>;
    fn reset_to_clean(&mut self);
    fn commit_clean_baseline(&mut self);
}

impl<T: Clone + PartialEq + 'static> DynField for TypedFieldSlot<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn is_dirty(&self) -> bool {
        self.state.is_dirty()
    }

    fn is_touched(&self) -> bool {
        self.state.is_touched()
    }

    fn mark_touched(&mut self) {
        self.state.mark_touched();
    }

    fn mark_submitted(&mut self) {
        self.state.mark_submitted();
    }

    fn run_sync_validators(&mut self) -> bool {
        for validator in &self.validators {
            if let Err(msg) = validator(self.state.value()) {
                self.state.sync_error = Some(msg);
                return false;
            }
        }
        self.state.sync_error = None;
        true
    }

    fn set_cross_error(&mut self, error: Option<String>) {
        self.state.set_cross_error(error);
    }

    fn begin_async_validation(&mut self) -> ValidationToken {
        self.state.begin_async_validation()
    }

    fn apply_async_result(&mut self, token: &ValidationToken, result: Result<(), String>) -> bool {
        self.state.apply_async_result(token, result)
    }

    fn is_validating(&self) -> bool {
        self.state.is_validating()
    }

    fn error(&self) -> Option<&str> {
        self.state.error()
    }

    fn visible_error(&self) -> Option<&str> {
        self.state.visible_error()
    }

    fn reset_to_clean(&mut self) {
        self.state.reset_to_clean();
    }

    fn commit_clean_baseline(&mut self) {
        self.state.commit_clean_baseline();
    }
}

struct CrossValidator {
    target_field: String,
    validator: CrossValidatorFn,
}

/// Fluent registration handle returned by [`FormState::register_field`] for attaching typed
/// synchronous validators.
pub struct FieldHandle<'a, T: Clone + PartialEq + 'static> {
    form: &'a mut FormState,
    id: String,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: Clone + PartialEq + 'static> FieldHandle<'a, T> {
    /// Appends a synchronous field validator executed in registration order.
    pub fn with_validator<F>(self, validator: F) -> Self
    where
        F: Fn(&T) -> Result<(), String> + 'static,
    {
        let _ = self.form.add_field_validator(&self.id, validator);
        self
    }
}

/// Heterogeneous, type-safe headless form controller with deterministic registration ordering,
/// dirty-tracking, Android Predictive Back compatibility (`can_pop()`), cross-field validation,
/// and generation-guarded async validation.
#[derive(Default)]
pub struct FormState {
    order: Vec<String>,
    fields: BTreeMap<String, Box<dyn DynField>>,
    cross_validators: Vec<CrossValidator>,
    submitted: bool,
}

impl FormState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers (or replaces) a typed field with its clean baseline value.
    pub fn register_field<T: Clone + PartialEq + 'static>(
        &mut self,
        id: impl Into<String>,
        initial: T,
    ) -> FieldHandle<'_, T> {
        let id = id.into();
        if !self.fields.contains_key(&id) {
            self.order.push(id.clone());
        }
        self.fields.insert(
            id.clone(),
            Box::new(TypedFieldSlot {
                state: FieldState::new(id.clone(), initial),
                validators: Vec::new(),
            }),
        );
        FieldHandle {
            form: self,
            id,
            _marker: std::marker::PhantomData,
        }
    }

    /// Attaches a synchronous validator to an existing typed field. Returns `false` if the field
    /// is missing or has a different type `T`.
    pub fn add_field_validator<T, F>(&mut self, id: &str, validator: F) -> bool
    where
        T: Clone + PartialEq + 'static,
        F: Fn(&T) -> Result<(), String> + 'static,
    {
        let Some(slot) = self
            .fields
            .get_mut(id)
            .and_then(|f| f.as_any_mut().downcast_mut::<TypedFieldSlot<T>>())
        else {
            return false;
        };
        slot.validators.push(Box::new(validator));
        true
    }

    /// Registers a cross-field validator whose error attaches to `target_field`
    /// (for example `password == confirm_password`).
    pub fn add_cross_validator<F>(&mut self, target_field: impl Into<String>, validator: F)
    where
        F: Fn(&FormState) -> Result<(), String> + 'static,
    {
        self.cross_validators.push(CrossValidator {
            target_field: target_field.into(),
            validator: Rc::new(validator),
        });
    }

    /// Returns a typed reference to the [`FieldState<T>`] for `id`.
    pub fn field<T: Clone + PartialEq + 'static>(&self, id: &str) -> Option<&FieldState<T>> {
        self.fields
            .get(id)
            .and_then(|f| f.as_any().downcast_ref::<TypedFieldSlot<T>>())
            .map(|slot| &slot.state)
    }

    /// Returns the current typed value for `id`.
    pub fn value<T: Clone + PartialEq + 'static>(&self, id: &str) -> Option<&T> {
        self.field::<T>(id).map(FieldState::value)
    }

    /// Updates a typed field's value, invalidates in-flight async checks for that field, and
    /// re-evaluates its sync validators and all cross-field validators.
    pub fn set_value<T: Clone + PartialEq + 'static>(&mut self, id: &str, value: T) -> bool {
        let Some(slot) = self
            .fields
            .get_mut(id)
            .and_then(|f| f.as_any_mut().downcast_mut::<TypedFieldSlot<T>>())
        else {
            return false;
        };
        slot.state.set_value(value);
        slot.run_sync_validators();
        self.run_cross_validators();
        true
    }

    /// Marks `id` as touched (on blur) and validates it plus cross-field rules.
    pub fn touch(&mut self, id: &str) -> bool {
        let Some(field) = self.fields.get_mut(id) else {
            return false;
        };
        field.mark_touched();
        field.run_sync_validators();
        self.run_cross_validators();
        true
    }

    pub fn is_touched(&self, id: &str) -> bool {
        self.fields.get(id).is_some_and(|f| f.is_touched())
    }

    pub fn is_field_dirty(&self, id: &str) -> bool {
        self.fields.get(id).is_some_and(|f| f.is_dirty())
    }

    /// Returns `true` if any registered field differs from its clean baseline.
    pub fn is_dirty(&self) -> bool {
        self.fields.values().any(|f| f.is_dirty())
    }

    /// Unsaved-changes navigation guard compatible with Android Predictive Back: returns `true`
    /// when the form has no dirty fields and can safely pop without data loss.
    #[inline]
    pub fn can_pop(&self) -> bool {
        !self.is_dirty()
    }

    /// Returns all dirty field IDs in registration order.
    pub fn dirty_fields(&self) -> Vec<&str> {
        self.order
            .iter()
            .filter(|id| self.fields.get(*id).is_some_and(|f| f.is_dirty()))
            .map(String::as_str)
            .collect()
    }

    /// Returns all registered field IDs in registration order.
    #[inline]
    pub fn field_ids(&self) -> &[String] {
        &self.order
    }

    /// Begins an async validation cycle on `id` and returns a generation-stamped token.
    pub fn begin_async_validation(&mut self, id: &str) -> Option<ValidationToken> {
        self.fields
            .get_mut(id)
            .map(|f| f.begin_async_validation())
    }

    /// Applies an async validation result iff `token` matches the field's current generation.
    /// Returns `false` if the token is stale or the field no longer exists.
    pub fn apply_async_result(
        &mut self,
        token: ValidationToken,
        result: Result<(), String>,
    ) -> bool {
        self.fields
            .get_mut(&token.field_id)
            .is_some_and(|f| f.apply_async_result(&token, result))
    }

    pub fn is_field_validating(&self, id: &str) -> bool {
        self.fields.get(id).is_some_and(|f| f.is_validating())
    }

    pub fn is_validating(&self) -> bool {
        self.fields.values().any(|f| f.is_validating())
    }

    /// Runs sync validators for one field plus all cross-field validators.
    pub fn validate_field(&mut self, id: &str) -> bool {
        let Some(field) = self.fields.get_mut(id) else {
            return false;
        };
        field.run_sync_validators();
        self.run_cross_validators();
        self.error(id).is_none()
    }

    /// Evaluates every registered sync field validator and every cross-field validator.
    pub fn validate_all(&mut self) -> bool {
        for field in self.fields.values_mut() {
            field.run_sync_validators();
        }
        self.run_cross_validators();
        self.is_valid()
    }

    /// Marks the form and all fields submitted (revealing all validation errors), runs full
    /// validation, and returns `true` iff every field is currently valid.
    pub fn submit(&mut self) -> bool {
        self.submitted = true;
        for field in self.fields.values_mut() {
            field.mark_submitted();
        }
        self.validate_all()
    }

    #[inline]
    pub fn is_submitted(&self) -> bool {
        self.submitted
    }

    pub fn error(&self, id: &str) -> Option<&str> {
        self.fields.get(id).and_then(|f| f.error())
    }

    pub fn visible_error(&self, id: &str) -> Option<&str> {
        self.fields.get(id).and_then(|f| f.visible_error())
    }

    pub fn is_valid(&self) -> bool {
        self.fields.values().all(|f| f.error().is_none())
    }

    /// Returns the first invalid field ID in registration order (for scroll-to-error and
    /// screen-reader announcement).
    pub fn first_error_field(&self) -> Option<&str> {
        self.order
            .iter()
            .find(|id| self.fields.get(*id).is_some_and(|f| f.error().is_some()))
            .map(String::as_str)
    }

    /// Returns the first field ID in registration order that currently has a visible error.
    pub fn first_visible_error_field(&self) -> Option<&str> {
        self.order
            .iter()
            .find(|id| {
                self.fields
                    .get(*id)
                    .is_some_and(|f| f.visible_error().is_some())
            })
            .map(String::as_str)
    }

    /// Resets every field back to its clean baseline, clears errors and interaction flags, and
    /// invalidates any in-flight async validations.
    pub fn reset_to_clean(&mut self) {
        self.submitted = false;
        for field in self.fields.values_mut() {
            field.reset_to_clean();
        }
    }

    /// Commits every field's current value as its new clean baseline so `is_dirty()` becomes
    /// `false` and `can_pop()` becomes `true`.
    pub fn commit_clean_baseline(&mut self) {
        self.submitted = false;
        for field in self.fields.values_mut() {
            field.commit_clean_baseline();
        }
    }

    fn run_cross_validators(&mut self) {
        let validators: Vec<(String, CrossValidatorFn)> = self
            .cross_validators
            .iter()
            .map(|cv| (cv.target_field.clone(), Rc::clone(&cv.validator)))
            .collect();

        let mut errors_by_field: BTreeMap<String, Option<String>> = BTreeMap::new();
        for (target, _) in &validators {
            errors_by_field.entry(target.clone()).or_insert(None);
        }

        for (target, validator) in &validators {
            if let Some(slot) = errors_by_field.get_mut(target)
                && slot.is_none()
                && let Err(msg) = validator(self)
            {
                *slot = Some(msg);
            }
        }

        for (target, err) in errors_by_field {
            if let Some(field) = self.fields.get_mut(&target) {
                field.set_cross_error(err);
            }
        }
    }
}
