pub mod app;
pub mod build;
pub mod build_list;
pub mod calcs;
pub mod compare;
pub mod config;
pub mod first_run;
pub mod folder_picker;
pub mod help;
pub(crate) mod hover;
pub mod import;
pub mod items;
pub mod name_box;
pub mod notes;
pub mod options;
pub mod popup;
#[cfg(test)]
mod popup_tests;
pub mod requests;
pub mod shortcuts;
pub mod sidebar;
pub mod skills;
pub mod startup;
pub mod tree;
pub mod unsaved;

pub use app::AutoPobApp;
