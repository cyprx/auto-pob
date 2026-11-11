//! auto-pob: an egui front-end driving Path of Building's calc engine headlessly.

pub mod engine;
pub mod game;
pub mod pob_files;
pub mod theme;
pub mod ui;
pub mod update_notice;
pub mod widgets;

/// The first command-line argument that is a `pob:` link (the first item is the program name).
pub fn pob_link_from_args(args: impl Iterator<Item = String>) -> Option<String> {
    args.skip(1).find(|arg| {
        arg.get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("pob:"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(args: &[&str]) -> Option<String> {
        pob_link_from_args(args.iter().map(|a| (*a).to_owned()))
    }

    #[test]
    fn pob_link_is_taken_from_the_first_pob_argument() {
        assert_eq!(
            link(&["auto-pob", "pob://pobbin/x"]).as_deref(),
            Some("pob://pobbin/x")
        );
        assert_eq!(link(&["auto-pob"]), None);
        assert_eq!(
            link(&["auto-pob", "--x", "POB://a/b", "pob://c/d"]).as_deref(),
            Some("POB://a/b")
        );
        assert_eq!(link(&["auto-pob", "https://pobb.in/x"]), None);
    }
}
