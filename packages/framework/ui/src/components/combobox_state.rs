//! Pure headless [`ComboboxState`] state machine for searchable single-select & multi-select
//! comboboxes with custom filtering, IME composition protection, `aria-activedescendant`
//! tracking, and `Backspace` last-tag deletion.

/// Selectable option entry inside a [`crate::Combobox`] or [`ComboboxState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComboboxOption {
    pub value: String,
    pub label: String,
    pub disabled: bool,
}

impl ComboboxOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// Outcome produced by [`ComboboxState::handle_key`] or [`ComboboxState::select_value`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComboboxAction {
    None,
    Opened,
    Closed,
    Highlighted(Option<String>),
    Selected(String),
    Deselected(String),
    RemovedLastTag(String),
}

/// Default case-insensitive substring matcher over option label and value.
pub fn default_combobox_filter(option: &ComboboxOption, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let q_lower = q.to_ascii_lowercase();
    option.label.to_ascii_lowercase().contains(&q_lower)
        || option.value.to_ascii_lowercase().contains(&q_lower)
}

/// Headless searchable single-select & multi-select combobox state machine.
///
/// Focus remains in the text input while `ArrowUp`/`ArrowDown`/`Home`/`End`/`Enter`/
/// `Escape`/`Backspace` control selection and `aria-activedescendant`. When `composing` is
/// `true` (active IME composition), keyboard commit/dismiss/delete actions are suppressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComboboxState {
    options: Vec<ComboboxOption>,
    filtered_indices: Vec<usize>,
    query: String,
    open: bool,
    multi: bool,
    selected: Vec<String>,
    highlighted: Option<usize>,
    composing: bool,
    loading: bool,
}

impl ComboboxState {
    pub fn new(options: Vec<ComboboxOption>, multi: bool) -> Self {
        let filtered_indices: Vec<usize> = (0..options.len()).collect();
        let highlighted = filtered_indices
            .iter()
            .position(|&idx| !options[idx].disabled);
        Self {
            options,
            filtered_indices,
            query: String::new(),
            open: false,
            multi,
            selected: Vec::new(),
            highlighted,
            composing: false,
            loading: false,
        }
    }

    pub fn with_selected(mut self, selected: Vec<String>) -> Self {
        self.selected = if self.multi {
            let mut deduped = Vec::new();
            for val in selected {
                if !deduped.contains(&val) {
                    deduped.push(val);
                }
            }
            deduped
        } else {
            selected.into_iter().take(1).collect()
        };
        self
    }

    #[inline]
    pub fn options(&self) -> &[ComboboxOption] {
        &self.options
    }

    #[inline]
    pub fn filtered_indices(&self) -> &[usize] {
        &self.filtered_indices
    }

    pub fn filtered_options(&self) -> Vec<&ComboboxOption> {
        self.filtered_indices
            .iter()
            .filter_map(|&idx| self.options.get(idx))
            .collect()
    }

    #[inline]
    pub fn query(&self) -> &str {
        &self.query
    }

    #[inline]
    pub fn is_open(&self) -> bool {
        self.open
    }

    #[inline]
    pub fn is_multi(&self) -> bool {
        self.multi
    }

    #[inline]
    pub fn selected(&self) -> &[String] {
        &self.selected
    }

    #[inline]
    pub fn is_selected(&self, value: &str) -> bool {
        self.selected.iter().any(|v| v == value)
    }

    #[inline]
    pub fn is_composing(&self) -> bool {
        self.composing
    }

    #[inline]
    pub fn set_composing(&mut self, composing: bool) {
        self.composing = composing;
    }

    #[inline]
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    #[inline]
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        !self.loading && self.filtered_indices.is_empty()
    }

    #[inline]
    pub fn highlighted_filtered_index(&self) -> Option<usize> {
        self.highlighted
    }

    pub fn highlighted_option(&self) -> Option<&ComboboxOption> {
        let fidx = self.highlighted?;
        let opt_idx = *self.filtered_indices.get(fidx)?;
        self.options.get(opt_idx).filter(|o| !o.disabled)
    }

    /// Returns the DOM/node ID for `aria-activedescendant` when the listbox is open and an
    /// enabled option is highlighted.
    pub fn active_option_id(&self, id_prefix: &str) -> Option<String> {
        if !self.open {
            return None;
        }
        self.highlighted_option()
            .map(|opt| format!("{id_prefix}-option-{}", opt.value))
    }

    pub fn open(&mut self) {
        self.open = true;
        if self.highlighted_option().is_none() {
            self.highlighted = self.first_enabled_filtered();
        }
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn set_options(&mut self, options: Vec<ComboboxOption>) {
        self.set_options_with_filter(options, default_combobox_filter);
    }

    pub fn set_options_with_filter<F>(&mut self, options: Vec<ComboboxOption>, filter: F)
    where
        F: Fn(&ComboboxOption, &str) -> bool,
    {
        self.options = options;
        self.apply_filter(filter);
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.set_query_with_filter(query, default_combobox_filter);
    }

    pub fn set_query_with_filter<F>(&mut self, query: impl Into<String>, filter: F)
    where
        F: Fn(&ComboboxOption, &str) -> bool,
    {
        self.query = query.into();
        self.open = true;
        self.apply_filter(filter);
    }

    /// Re-runs filtering over current options using a custom predicate.
    pub fn filter_with<F>(&mut self, filter: F)
    where
        F: Fn(&ComboboxOption, &str) -> bool,
    {
        self.apply_filter(filter);
    }

    fn apply_filter<F>(&mut self, filter: F)
    where
        F: Fn(&ComboboxOption, &str) -> bool,
    {
        let prev_value = self.highlighted_option().map(|o| o.value.clone());
        self.filtered_indices = self
            .options
            .iter()
            .enumerate()
            .filter(|(_, opt)| filter(opt, &self.query))
            .map(|(idx, _)| idx)
            .collect();
        self.highlighted = prev_value
            .as_deref()
            .and_then(|val| {
                self.filtered_indices.iter().position(|&idx| {
                    !self.options[idx].disabled && self.options[idx].value == val
                })
            })
            .or_else(|| self.first_enabled_filtered());
    }

    fn first_enabled_filtered(&self) -> Option<usize> {
        self.filtered_indices
            .iter()
            .position(|&idx| !self.options[idx].disabled)
    }

    fn last_enabled_filtered(&self) -> Option<usize> {
        self.filtered_indices
            .iter()
            .rposition(|&idx| !self.options[idx].disabled)
    }

    fn step_enabled(&self, forward: bool) -> Option<usize> {
        let len = self.filtered_indices.len();
        if len == 0 {
            return None;
        }
        let Some(cur) = self.highlighted else {
            return if forward {
                self.first_enabled_filtered()
            } else {
                self.last_enabled_filtered()
            };
        };
        for delta in 1..=len {
            let cand = if forward {
                (cur + delta) % len
            } else {
                (cur + len - (delta % len)) % len
            };
            if !self.options[self.filtered_indices[cand]].disabled {
                return Some(cand);
            }
        }
        None
    }

    /// Selects or toggles `value` (ignoring missing or disabled options).
    pub fn select_value(&mut self, value: &str) -> ComboboxAction {
        let Some(opt) = self
            .options
            .iter()
            .find(|o| !o.disabled && o.value == value)
            .cloned()
        else {
            return ComboboxAction::None;
        };

        if self.multi {
            if let Some(pos) = self.selected.iter().position(|v| v == &opt.value) {
                self.selected.remove(pos);
                ComboboxAction::Deselected(opt.value)
            } else {
                self.selected.push(opt.value.clone());
                self.query.clear();
                self.apply_filter(default_combobox_filter);
                ComboboxAction::Selected(opt.value)
            }
        } else {
            self.selected.clear();
            self.selected.push(opt.value.clone());
            self.query = opt.label;
            self.open = false;
            self.apply_filter(default_combobox_filter);
            ComboboxAction::Selected(opt.value)
        }
    }

    pub fn remove_selected(&mut self, value: &str) -> bool {
        if let Some(pos) = self.selected.iter().position(|v| v == value) {
            self.selected.remove(pos);
            true
        } else {
            false
        }
    }

    /// Handles keyboard input while focus stays on the combobox text input.
    /// Returns [`ComboboxAction::None`] whenever `composing == true` (IME composition active).
    pub fn handle_key(&mut self, key: &str) -> ComboboxAction {
        if self.composing {
            return ComboboxAction::None;
        }
        match key {
            "ArrowDown" => {
                if !self.open {
                    self.open = true;
                    self.highlighted = self.first_enabled_filtered();
                    ComboboxAction::Opened
                } else {
                    self.highlighted = self.step_enabled(true);
                    ComboboxAction::Highlighted(
                        self.highlighted_option().map(|o| o.value.clone()),
                    )
                }
            }
            "ArrowUp" => {
                if !self.open {
                    self.open = true;
                    self.highlighted = self.last_enabled_filtered();
                    ComboboxAction::Opened
                } else {
                    self.highlighted = self.step_enabled(false);
                    ComboboxAction::Highlighted(
                        self.highlighted_option().map(|o| o.value.clone()),
                    )
                }
            }
            "Home" if self.open => {
                self.highlighted = self.first_enabled_filtered();
                ComboboxAction::Highlighted(self.highlighted_option().map(|o| o.value.clone()))
            }
            "End" if self.open => {
                self.highlighted = self.last_enabled_filtered();
                ComboboxAction::Highlighted(self.highlighted_option().map(|o| o.value.clone()))
            }
            "Enter" => {
                if !self.open {
                    self.open = true;
                    self.highlighted = self.first_enabled_filtered();
                    ComboboxAction::Opened
                } else if let Some(val) = self.highlighted_option().map(|o| o.value.clone()) {
                    self.select_value(&val)
                } else {
                    ComboboxAction::None
                }
            }
            "Escape" | "Esc" if self.open => {
                self.open = false;
                ComboboxAction::Closed
            }
            "Backspace" if self.multi && self.query.is_empty() => {
                if let Some(removed) = self.selected.pop() {
                    ComboboxAction::RemovedLastTag(removed)
                } else {
                    ComboboxAction::None
                }
            }
            _ => ComboboxAction::None,
        }
    }
}
