//! The Skills tab's wiki key (F1): the gem under the pointer (GemSelectControl.lua:822-839,
//! SkillListControl.lua:193-203).

use super::SkillsUi;

/// PoB's page for a gem: its name, and " Support" for a support (ItemTools.lua:382-392)
fn gem_page(name: &str, support: bool) -> String {
    if support {
        format!("{name} Support")
    } else {
        name.to_owned()
    }
}

impl SkillsUi {
    /// The wiki page of what the pointer is over: a row of the open gem list, a gem of the
    /// group shown, or a socket group of the list (its first gem)
    pub fn hovered_wiki(&self) -> Option<String> {
        if let Some(picker) = &self.picker {
            return picker.hovered_gem().map(|g| gem_page(&g.name, g.support));
        }
        let view = self.view.as_ref()?;
        if let Some(((group, slot), _)) = self.gem_hover {
            // only a gem with gem data has a page there (GemSelectControl.lua:834-838)
            let gem = view.groups.get(group)?.gems.get(slot)?;
            return gem
                .gem_id
                .as_ref()
                .map(|_| gem_page(&gem.name_spec, gem.support));
        }
        let (group, _) = self.hover?;
        let gem = view.groups.get(group)?.gems.first()?;
        Some(gem_page(&gem.name_spec, gem.support))
    }
}
