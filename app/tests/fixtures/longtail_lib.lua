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
