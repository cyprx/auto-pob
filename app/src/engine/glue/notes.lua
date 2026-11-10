-- The Notes tab, through PoB's own `notesTab` and its edit control.

glue = glue or {}

-- The buttons of NotesTab.lua:25-37, in order
local COLOURS = {
	"NORMAL", "MAGIC", "RARE", "UNIQUE", "FIRE", "COLD", "LIGHTNING", "CHAOS",
	"STRENGTH", "DEXTERITY", "INTELLIGENCE",
}

function glue.notes_view()
	local colours = {}
	for _, label in ipairs(COLOURS) do
		colours[#colours + 1] = { label = label, code = colorCodes[label] }
	end
	colours[#colours + 1] = { label = "DEFAULT", code = "^7" }
	return { text = pob.build.notesTab.controls.edit.buf, colours = colours }
end

-- What typing the text into the box would leave: `EditControl:Insert` drops "\r" and then
-- the characters its filter rejects (EditControl.lua:164-165).
function glue.set_notes(text)
	local tab = pob.build.notesTab
	local edit = tab.controls.edit
	text = text:gsub("\r", "")
	text = text:gsub(edit.filterPattern, "")
	edit:SetText(text)
	-- NotesTab only sets modFlag in Draw; OnFrame reads it for `unsaved` (Build.lua:1303)
	tab.modFlag = tab.lastContent ~= edit.buf
	glue.frame()
end
