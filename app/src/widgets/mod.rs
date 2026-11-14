//! Widgets shared by every screen. They know only `engine::views` types, egui and `theme`.

pub mod art;
pub mod dropdown;
pub mod edit_box;
pub mod escape;
pub mod item_tooltip;
pub mod list;
pub mod pob_text;
pub mod scroll_step;
pub mod search_combo;
pub mod size_grip;
pub mod tooltip;
pub mod tooltip_art;
pub mod url_click;

pub use edit_box::{EditBox, EditBoxOptions, EditBoxes};
pub use item_tooltip::{item_tooltip_ui, item_tooltip_ui_with};
pub use list::{ListAction, ListColumn, ListSort, drop_index, list, list_sort, list_with};
pub use pob_text::{fit_with_dots, pob_colour, pob_text};
pub use search_combo::{search_combo, search_combo_ui};
pub use tooltip::tooltip_ui;
