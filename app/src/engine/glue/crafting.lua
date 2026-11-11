-- The display panel's crafting sections (R3). Later tasks add the affix editor, the cluster
-- jewel controls and the custom-mod list; the windows come first.

glue = glue or {}

-- the windows PoB's panel offers for this item
table.insert(glue.display_sections, function(tab, item, out)
	out.popups = glue.panel_popups(tab)
end)
