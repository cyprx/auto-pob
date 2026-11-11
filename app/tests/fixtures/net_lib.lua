-- Helpers for the network recipes. Runs in upstream PoB after tree_lib.lua, with the subscript
-- natives installed and their scope open. Every `act.*` presses PoB's own controls and leaves
-- the popup open for the pump to deliver into, as PoB does. Nothing here may call our glue.

local function frame()
	runCallback("OnFrame")
end

-- The Import button on a PoEURL link (TreeTab.lua:741-771)
function act.tree_import(url, title)
	build.treeTab:OpenImportPopup()
	local controls = main.popups[1].controls
	controls.name:SetText(title, true)
	controls.edit:SetText(url, true)
	controls.import.onClick()
	frame()
end

-- The Export popup's "Shrink with PoEURL" button (TreeTab.lua:813-831)
function act.tree_shrink()
	build.treeTab:OpenExportPopup()
	main.popups[1].controls.shrink.onClick()
	frame()
end

-- What the newest popup tells the user: the import popup's message, or a message popup's text
function popupMessage()
	local popup = main.popups[1]
	if not popup then return nil end
	local controls = popup.controls
	if controls.msg then return StripEscapes(controls.msg.label) end
	if controls[1] then return StripEscapes(controls[1].label) end
end

-- The Export popup's link box after the shrink
function shrunkLink()
	for _, popup in ipairs(main.popups) do
		if popup.controls.shrink then return popup.controls.edit.buf end
	end
end

-- Closes what the pump left open
function closePopups()
	while #main.popups > 0 do main:ClosePopup() end
end

-- The tree state, as `checkpoint()` records it; `tree_lib.lua`'s own
function treeDump()
	checkpoint()
	return actions[#actions].check
end

function treeCanUndo()
	return build.spec.undo[2] ~= nil
end
