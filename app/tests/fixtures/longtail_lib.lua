-- Helpers for the long-tail gate tests. Runs in unmodified upstream PoB (HeadlessWrapper) with
-- our file natives installed over the wrapper's stubs. Written independently of our glue: every
-- act presses a button of PoB's build list (`main.modes.LIST`) or types into its search box as
-- a user would, and every read is of PoB's own list. Nothing here may call `glue`.

-- ---- Build list ----

local list

-- Starts PoB's build list on the Builds folder `builds`
function listStart(builds)
	main.buildPath = builds
	main.filterBuildList = ""
	main.buildSortMode = "NAME"
	list = main.modes.LIST
	list:Init(nil, "")
end

-- A click on the path bar's segment, then a rescan as the list does after any change
local function go(subPath)
	list.controls.buildList.controls.path:SetSubPath(subPath)
	list:BuildList()
end

local function folderOf(path)
	return path:sub(#main.buildPath + 1):match("^(.*/)[^/]*$") or ""
end

-- A click on the entry's row
local function pick(path)
	go(folderOf(path))
	local control = list.controls.buildList
	control:SelByFullFileName(path)
	assert(control.selValue, "no row for " .. path)
end

local function closePopups()
	while #main.popups > 0 do main:ClosePopup() end
end

-- The text of the newest popup's message lines
local function popupText()
	local lines = {}
	for _, control in ipairs(main.popups[1].controls) do
		if control._className == "LabelControl" then
			lines[#lines + 1] = StripEscapes(control.label)
		end
	end
	return table.concat(lines, "\n")
end

-- What an error popup says (and closes everything), or nil
local function takeError()
	local message
	if #main.popups > 0 and main.popups[1].controls.close then
		message = popupText()
	end
	closePopups()
	return message
end

-- The entries PoB lists in `subPath`, as the list holds them
function listDump(subPath)
	go(subPath)
	local out = {}
	for _, item in ipairs(list.list) do
		out[#out + 1] = {
			name = item.folderName or item.buildName,
			is_folder = item.folderName ~= nil,
			class = item.className,
			ascendancy = item.ascendClassName,
			level = item.level,
			rel = item.fullFileName:sub(#main.buildPath + 1),
		}
	end
	return out
end

-- The Rename or Copy button, a typed name, Save. Returns "disabled" when PoB leaves Save
-- disabled, or the text of the error popup PoB opens.
function listRename(path, name, copy)
	pick(path)
	local button = copy and list.controls.copy or list.controls.rename
	assert(button:IsEnabled(), "the button is disabled")
	button.onClick()
	local popup = main.popups[1]
	popup.controls.edit:SetText(name, true)
	if not popup.controls.save.enabled then
		closePopups()
		return "disabled"
	end
	popup.controls.save.onClick()
	return takeError()
end

-- New Folder, a typed name, Create
function listNewFolder(subPath, name)
	go(subPath)
	list.controls.newFolder.onClick()
	local popup = main.popups[1]
	popup.controls.edit:SetText(name, true)
	if not popup.controls.create.enabled then
		closePopups()
		return "disabled"
	end
	popup.controls.create.onClick()
	return takeError()
end

-- Ctrl+X on the row, the path bar to the folder, Ctrl+V. Returns the error popup's text.
function listMove(path, folder)
	pick(path)
	local real = IsKeyDown
	IsKeyDown = function(key) return key == "CTRL" end
	list.controls.buildList:OnKeyDown("x")
	go(folder)
	list:OnFrame({ { type = "KeyDown", key = "v" } })
	IsKeyDown = real
	return takeError()
end

-- Ctrl+C on the row, the path bar to the folder, Ctrl+V. Returns the error popup's text.
function listCopyTo(path, folder)
	pick(path)
	local real = IsKeyDown
	IsKeyDown = function(key) return key == "CTRL" end
	list.controls.buildList:OnKeyDown("c")
	go(folder)
	list:OnFrame({ { type = "KeyDown", key = "v" } })
	IsKeyDown = real
	return takeError()
end

-- The Delete button. Returns PoB's confirm text ("" when PoB doesn't ask) and presses the
-- confirm when `confirm` is set.
function listDelete(path, confirm)
	pick(path)
	list.controls.delete.onClick()
	local text = ""
	local message
	if #main.popups > 0 then
		if main.popups[1].controls.confirm then
			text = popupText()
			if confirm then main.popups[1].controls.confirm.onClick() end
		else
			message = takeError()
		end
	end
	closePopups()
	return text, message
end

-- The sort dropdown, at the entry with `sortMode`
function listSort(sortMode)
	local sort = list.controls.sort
	for index, entry in ipairs(sort.list) do
		if entry.sortMode == sortMode then sort:SetSel(index) end
	end
	return main.buildSortMode
end

-- Types into the search box
function listFilter(text)
	list.controls.searchText:SetText(text, true)
	return main.filterBuildList
end

-- ---- Top bar and Spectre Library (Task 12) ----
-- Every act presses the build's own top-bar controls or the Spectre Library's buttons as a
-- user would, then runs PoB's own frame; every read is of PoB's own state.

local function usable(control, what)
	assert(control:IsShown() and control:IsEnabled(), what .. " is not usable")
end

-- One of PoB's own frames: draws the build, and recalculates when a flag asks for it
function upFrame()
	runCallback("OnFrame")
end

-- Typed into the level box
function typeLevel(text)
	local box = build.controls.characterLevel
	usable(box, "the level box")
	box:SetText(text, true)
	upFrame()
end

-- The Auto/Manual button
function pressLevelMode()
	local button = build.controls.levelScalingButton
	usable(button, "the level button")
	button.onClick()
	upFrame()
end

-- The entry of a dropdown whose `key` (or the entry itself) is `value`, picked with a click
local function pickEntry(drop, value, key, what)
	usable(drop, what)
	for index, entry in ipairs(drop.list) do
		local v = key and type(entry) == "table" and entry[key] or entry
		if v == value then
			drop:SetSel(index)
			return
		end
	end
	error(what .. " has no entry " .. tostring(value))
end

function pickBloodline(id)
	pickEntry(build.controls.secondaryAscendDrop, id, "ascendClassId", "the bloodline dropdown")
	upFrame()
end

function pickLoadout(label)
	pickEntry(build.controls.buildLoadouts, label, nil, "the loadouts dropdown")
	upFrame()
end

-- "New Loadout", the name typed into its popup, Save
function newLoadout(name)
	pickEntry(build.controls.buildLoadouts, "^7^7New Loadout", nil, "the loadouts dropdown")
	local popup = main.popups[1]
	assert(popup and popup.title == "Set Name", "New Loadout opened no name popup")
	popup.controls.edit:SetText(name, true)
	assert(popup.controls.save.enabled, "Save stayed disabled")
	popup.controls.save.onClick()
	assert(#main.popups == 0, "the name popup stayed open")
	upFrame()
end

-- Clears the unsaved flags, as a save does, so the next act shows the ones it sets. `b` is
-- the build (the wrapper's `build` by default); the gate reads its own engine's the same way.
function clearFlags(b)
	b = b or build
	b.modFlag = false
	b.spec.modFlag = false
	b.treeTab.modFlag = false
	b.skillsTab.modFlag = false
	b.itemsTab.modFlag = false
	b.configTab.modFlag = false
	b.calcsTab.modFlag = false
	b.notesTab.modFlag = false
end

-- What the top bar shows and what the acts change, of `b` (the wrapper's `build` by default)
function topState(b)
	b = b or build
	local alloc = {}
	for id in pairs(b.spec.allocNodes) do alloc[#alloc + 1] = id end
	table.sort(alloc)
	local secondary = {}
	for i, entry in ipairs(b.controls.secondaryAscendDrop.list) do
		secondary[i] = { entry.ascendClassId, entry.label }
	end
	local loadouts = {}
	for i, entry in ipairs(b.controls.buildLoadouts.list) do loadouts[i] = entry end
	return {
		level = b.characterLevel,
		auto = b.characterLevelAutoMode and true or false,
		secondary_id = b.spec.curSecondaryAscendClassId or 0,
		secondary = secondary,
		secondary_enabled = b.controls.secondaryAscendDrop:IsEnabled() and true or false,
		alloc = alloc,
		loadouts = loadouts,
		loadout_sel = b.controls.buildLoadouts.selIndex,
		spec = b.treeTab.activeSpec,
		item_set = b.itemsTab.activeItemSetId,
		skill_set = b.skillsTab.activeSkillSetId,
		config_set = b.configTab.activeConfigSetId,
		specs = #b.treeTab.specList,
		item_sets = #b.itemsTab.itemSetOrderList,
		skill_sets = #b.skillsTab.skillSetOrderList,
		config_sets = #b.configTab.configSetOrderList,
		flags = {
			build = b.modFlag and true or false,
			spec = b.spec.modFlag and true or false,
			tree = b.treeTab.modFlag and true or false,
			skills = b.skillsTab.modFlag and true or false,
			items = b.itemsTab.modFlag and true or false,
			config = b.configTab.modFlag and true or false,
			calcs = b.calcsTab.modFlag and true or false,
		},
		undo = {
			spec = #b.spec.undo,
			skills = #b.skillsTab.undo,
			items = #b.itemsTab.undo,
			config = #b.configTab.undo,
			calcs = #b.calcsTab.undo,
		},
		unsaved = b.unsaved and true or false,
		progress = b.controls.pointDisplay.req or "",
		spectres = copyTable(b.spectreList),
		warnings = copyTable(b.controls.warnings.lines),
	}
end

-- The sidebar's and the Calcs selector's "Manage Spectres..." buttons: whether PoB shows them
function spectreButtons()
	local calcs = build.calcsTab
	local section
	for _, s in ipairs(calcs.sectionList) do
		if s.id == "SkillSelect" then section = s end
	end
	section:updateFunc()
	local calcsButton = section.controls.mainSkillMinionLibrary
	local row
	for _, sub in ipairs(section.subSection) do
		for _, r in ipairs(sub.data) do
			for _, col in ipairs(r) do
				if col.controlName == "mainSkillMinionLibrary" then row = r end
			end
		end
	end
	return build.controls.mainSkillMinionLibrary:IsShown() and true or false,
		(calcsButton.shown == true and calcs:CheckFlag(row)) and true or false
end

-- The sidebar's minion dropdown labels
function minionLabels()
	local out = {}
	for i, entry in ipairs(build.controls.mainSkillMinion.list) do
		out[i] = type(entry) == "table" and entry.label or entry
	end
	return out
end

local library

-- The sidebar's "Manage Spectres..."
function openLibrary()
	local button = build.controls.mainSkillMinionLibrary
	usable(button, "Manage Spectres")
	button.onClick()
	library = main.popups[1]
	assert(library and library.title == "Spectre Library", "no Spectre Library window")
end

-- The ids the available list shows, in its order
function librarySource()
	return copyTable(library.controls.source.list)
end

function libraryDest()
	return copyTable(library.controls.list.list)
end

-- The search mode dropdown (1 Names, 2 Skills, 3 Both), then the typed search
function librarySearch(text, mode)
	local source = library.controls.source
	source.controls.searchModeDropDown:SetSel(mode)
	source.controls.searchText:SetText(text, true)
end

-- A click on the row, then the list's button ("add" or "delete")
function libraryPress(listName, id, button)
	local list = library.controls[listName]
	for index, value in ipairs(list.list) do
		if value == id then
			list:SelectIndex(index)
			usable(list.controls[button], button)
			list.controls[button].onClick()
			return
		end
	end
	error("no row " .. tostring(id))
end

function libraryClose(button)
	library.controls[button].onClick()
	assert(#main.popups == 0, "the library stayed open")
	library = nil
	upFrame()
end
