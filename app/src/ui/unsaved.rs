//! When to ask about unsaved changes, and what each answer means.

use std::path::PathBuf;

/// What the user is about to do that could lose unsaved changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    Close,
    BackToList,
    Open(PathBuf),
    New,
    /// Import a link or code as a new build, replacing the one open
    ImportNew(String),
}

/// The user's answer to the Save / Don't save / Cancel dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptChoice {
    Save,
    Discard,
    Cancel,
}

/// What to do after the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Save, then carry on with the pending action.
    SaveThen,
    /// The build has no file yet: ask for a name, save, then carry on.
    SaveAsThen,
    /// Carry on without saving.
    Proceed,
    /// Do nothing.
    Stay,
}

pub fn needs_prompt(unsaved: bool, _action: &Pending) -> bool {
    unsaved
}

pub fn resolve(choice: PromptChoice, has_path: bool) -> Resolution {
    match choice {
        PromptChoice::Save if has_path => Resolution::SaveThen,
        PromptChoice::Save => Resolution::SaveAsThen,
        PromptChoice::Discard => Resolution::Proceed,
        PromptChoice::Cancel => Resolution::Stay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Pending> {
        vec![
            Pending::Close,
            Pending::BackToList,
            Pending::Open("a.xml".into()),
            Pending::New,
        ]
    }

    #[test]
    fn no_prompt_when_saved() {
        for p in all() {
            assert!(!needs_prompt(false, &p), "{p:?}");
        }
    }

    #[test]
    fn prompt_when_unsaved() {
        for p in all() {
            assert!(needs_prompt(true, &p), "{p:?}");
        }
    }

    #[test]
    fn save_without_path_goes_to_save_as() {
        assert_eq!(resolve(PromptChoice::Save, false), Resolution::SaveAsThen);
        assert_eq!(resolve(PromptChoice::Save, true), Resolution::SaveThen);
    }

    #[test]
    fn discard_proceeds_cancel_stays() {
        assert_eq!(resolve(PromptChoice::Discard, true), Resolution::Proceed);
        assert_eq!(resolve(PromptChoice::Discard, false), Resolution::Proceed);
        assert_eq!(resolve(PromptChoice::Cancel, true), Resolution::Stay);
        assert_eq!(resolve(PromptChoice::Cancel, false), Resolution::Stay);
    }
}
