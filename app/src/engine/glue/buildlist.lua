-- The build list (R16): PoB's own list mode (`main.modes.LIST`) and its controls. The glue
-- sets the folder, selects the entry, presses the button and reads what PoB answers. It never
-- switches the mode, so a loaded build stays loaded. `list_builds` is the view and writes
-- nothing; only the commands below change a file, and only inside the Builds folder.
--
-- The checks that are ours (R16): paths stay inside `main.buildPath`, `.` and `..` are not
-- names, the open build is not renamed, moved or deleted, and what PoB ignores the result of
-- (`os.remove`, `os.rename` on a paste) is looked at afterwards.

glue = glue or {}

local function main()
	return pob.main.main
end

local OUTSIDE = "that path is outside the Builds folder"

-- PoB's list mode, initialised (a start in BUILD mode never ran its Init)
local function list_mode()
	local mode = main().modes.LIST
	if not mode.initialised then
		mode:Init(nil, "")
	end
	return mode
end

-- Puts the mode on `subPath` and rescans it, with the search box and sort as `main` holds them
local function prepare(subPath)
	local mode = list_mode()
	mode.controls.searchText:SetText(main().filterBuildList or "")
	mode.controls.sort:SelByValue(main().buildSortMode, "sortMode")
	mode.subPath = subPath
	mode:BuildList()
	return mode
end

-- Raises when `control` is hidden or disabled, as PoB would not take the press
local function press(control)
	if not (control and control:IsShown() and control:IsEnabled()) then
		error("PoB doesn't offer that here", 0)
	end
	control.onClick()
end

-- The text of a popup's message lines
local function popup_text(popup)
	local lines = {}
	for _, control in ipairs(popup.controls) do
		if control._className == "LabelControl" then
			lines[#lines + 1] = StripEscapes(control.label)
		end
	end
	return table.concat(lines, "\n")
end

-- Without the Builds folder's path, which is the user's
local function without_root(text)
	local root = main().buildPath
	local out, from = {}, 1
	while true do
		local at, to = text:find(root, from, true)
		if not at then
			out[#out + 1] = text:sub(from)
			break
		end
		out[#out + 1] = text:sub(from, at - 1)
		from = to + 1
	end
	return table.concat(out)
end

-- Raises PoB's message when the press left an error popup above the `depth` popups
local function check_error_popup(depth)
	local popups = main().popups
	if #popups > depth then
		local popup = popups[1]
		if popup.controls.close and not popup.controls.confirm then
			error(without_root(popup_text(popup):gsub("\n.*", "")), 0)
		end
	end
end

-- The path below the Builds folder; raises for one that isn't inside it
local function relative(path)
	local root = main().buildPath
	if type(path) ~= "string" or path:sub(1, #root) ~= root then
		error(OUTSIDE, 0)
	end
	local rel = path:sub(#root + 1)
	if rel == "" or rel:sub(-1) == "/" or rel:find("//", 1, true) then
		error(OUTSIDE, 0)
	end
	for part in rel:gmatch("[^/]+") do
		if part == ".." or part == "." then
			error(OUTSIDE, 0)
		end
	end
	return rel
end

-- Raises for a name the Builds folder can't take: `.` and `..`, a blank one, or one with a
-- character PoB's own name boxes keep out (EditControl's filter, BuildListControl.lua:90)
local function check_name(name)
	if type(name) ~= "string" or name == "." or name == ".." or not name:match("%S")
		or name:find('[\\/:%*%?"<>|%c]') then
		error("that name isn't allowed", 0)
	end
end

local function exists(path)
	local file = io.open(path, "r")
	if file then
		file:close()
		return true
	end
	return false
end

-- A path with its repeated slashes folded (the user path ends with one, and PoB adds another)
local function squash(path)
	return (path:gsub("/+", "/"))
end

-- Raises when `build` is the open build's file, or a folder holding it
local function check_not_open(build)
	if not glue.build_loaded() then
		return
	end
	local open = pob.build.dbFileName
	if not open then
		return
	end
	open = squash(open)
	local entry = squash(build.fullFileName)
	if open == entry or (build.folderName and open:sub(1, #entry + 1) == entry .. "/") then
		error("that build is open", 0)
	end
end

-- Runs `fn(mode, list, entry)` with the entry at `path` selected in its own folder, the search
-- box empty (PoB's list shows matches from every folder while it isn't) and every popup
-- `fn` leaves open closed.
local function with_entry(path, fn)
	local rel = relative(path)
	local mode = list_mode()
	local filter = main().filterBuildList
	main().filterBuildList = ""
	local ok, err = pcall(function()
		mode.subPath = rel:match("^(.*/)[^/]*$") or ""
		mode:BuildList()
		local list = mode.controls.buildList
		list:SelByFullFileName(path)
		if not list.selValue then
			error("no such build", 0)
		end
		glue.in_popups(function()
			fn(mode, list, list.selValue)
		end)
	end)
	main().filterBuildList = filter
	if not ok then
		error(err, 0)
	end
end

-- Types `name` into the newest popup's box and presses `button` ("save" or "create")
local function submit_name(name, button)
	check_name(name)
	local popup = main().popups[1]
	glue.type_into(popup.controls.edit, name)
	local depth = #main().popups
	press(popup.controls[button])
	-- a success closes the popup; an error popup opens above it
	check_error_popup(depth)
	if #main().popups >= depth then
		error("PoB doesn't offer that here", 0)
	end
end

local function rename_or_copy(path, name, button)
	check_name(name)
	with_entry(path, function(mode, list, build)
		if button == "rename" then
			check_not_open(build)
		end
		local depth = #main().popups
		press(mode.controls[button])
		if #main().popups ~= depth + 1 then
			error("PoB doesn't offer that here", 0)
		end
		submit_name(name, "save")
	end)
end

function glue.rename_build(path, name)
	rename_or_copy(path, name, "rename")
end

function glue.copy_build(path, name)
	rename_or_copy(path, name, "copy")
end

-- What PoB asks before each delete, by what the UI showed (`kind`): "build", "folder" (not
-- empty) or "empty" (PoB asks nothing)
local ASKS = {
	build = "Are you sure you want to delete build:",
	folder = "The folder is not empty.",
}

function glue.delete_build(path, kind)
	with_entry(path, function(mode, list, build)
		check_not_open(build)
		if (kind == "build") == (build.folderName ~= nil) or not (kind == "build" or ASKS[kind] or kind == "empty") then
			error("the folder changed; look again", 0)
		end
		local depth = #main().popups
		press(mode.controls.delete)
		check_error_popup(depth)
		local popups = main().popups
		if #popups > depth then
			-- only the question the user was shown is answered
			if not (ASKS[kind] and popup_text(popups[1]):sub(1, #ASKS[kind]) == ASKS[kind]) then
				error("the folder changed; look again", 0)
			end
			glue.confirm_popup("delete that build")
		elseif kind ~= "empty" then
			error("the folder changed; look again", 0)
		end
		-- PoB ignores what `os.remove` and the recursive RemoveDir answer
		if exists(path) then
			error("couldn't delete that build", 0)
		end
	end)
end

function glue.move_build(path, folder)
	folder = glue.normalise_folder(folder)
	relative(main().buildPath .. folder .. "x")
	with_entry(path, function(mode, list, build)
		check_not_open(build)
		-- Ctrl+X on the entry, then Ctrl+V in the folder (BuildList.lua:125-149)
		list:OnSelCut(list.selIndex, build)
		mode.subPath = folder
		local depth = #main().popups
		local real = IsKeyDown
		IsKeyDown = function(key)
			return key == "CTRL"
		end
		local ok, err = pcall(mode.OnFrame, mode, { { type = "KeyDown", key = "v" } })
		IsKeyDown = real
		list.cutBuild = nil
		if not ok then
			error(err, 0)
		end
		-- "A folder cannot be moved into itself." and the like
		check_error_popup(depth)
		if build.subPath ~= folder and exists(path) then
			error("couldn't move that build", 0)
		end
	end)
end

-- PoB's Ctrl+C on the entry, then Ctrl+V in `folder` (BuildList.lua:138-148). In the folder the
-- entry is in PoB opens its Copy box instead, which `CopyBuild` presses.
function glue.copy_build_to(path, folder)
	folder = glue.normalise_folder(folder)
	relative(main().buildPath .. folder .. "x")
	with_entry(path, function(mode, list, build)
		if build.subPath == folder then
			error("that build is already in that folder", 0)
		end
		list:OnSelCopy(list.selIndex, build)
		mode.subPath = folder
		local depth = #main().popups
		local real = IsKeyDown
		IsKeyDown = function(key)
			return key == "CTRL"
		end
		local ok, err = pcall(mode.OnFrame, mode, { { type = "KeyDown", key = "v" } })
		IsKeyDown = real
		list.copyBuild = nil
		if not ok then
			error(err, 0)
		end
		-- "A folder cannot be copied into itself." and the like
		check_error_popup(depth)
		if #main().popups > depth then
			error("PoB doesn't offer that here", 0)
		end
	end)
end

function glue.new_folder(subPath, name)
	check_name(name)
	local mode = prepare(subPath)
	glue.in_popups(function()
		local depth = #main().popups
		press(mode.controls.newFolder)
		if #main().popups ~= depth + 1 then
			error("PoB doesn't offer that here", 0)
		end
		submit_name(name, "create")
	end)
end

function glue.set_build_sort(sortMode)
	local mode = list_mode()
	mode.controls.sort:SelByValue(main().buildSortMode, "sortMode")
	local index
	for i, entry in ipairs(mode.controls.sort.list) do
		if entry.sortMode == sortMode then
			index = i
		end
	end
	if not index then
		error("that isn't a way to sort builds", 0)
	end
	if main().buildSortMode == sortMode then
		return
	end
	mode.controls.sort:SetSel(index)
	-- PoB saves it at exit, which this app never runs (R16)
	main():SaveSettings()
end

function glue.set_build_filter(text)
	local mode = list_mode()
	glue.type_into(mode.controls.searchText, text)
end

-- PoB's confirm text for deleting `item`, or "" when it deletes without asking. The press is
-- PoB's own; a delete that would go ahead without a question (an empty folder) runs against
-- a RemoveDir that does nothing.
local function delete_prompt(mode, item)
	local list = mode.controls.buildList
	list:SelByFullFileName(item.fullFileName)
	if list.selValue == nil then
		return ""
	end
	local popups = main().popups
	local depth = #popups
	-- nothing PoB's Delete reaches may change a file here
	local real = { RemoveDir, MakeDir, os.remove, os.rename }
	local function nothing()
		return true
	end
	RemoveDir, MakeDir, os.remove, os.rename = nothing, nothing, nothing, nothing
	local ok, err = pcall(mode.controls.delete.onClick)
	RemoveDir, MakeDir, os.remove, os.rename = real[1], real[2], real[3], real[4]
	local text = ""
	if #popups > depth and popups[1].controls.confirm then
		text = popup_text(popups[1])
	end
	while #popups > depth do
		main():ClosePopup()
	end
	if not ok then
		error(err, 0)
	end
	return text
end

local SORTS = { NAME = true, CLASS = true, EDITED = true, LEVEL = true }

function glue.build_root()
	return main().buildPath
end

-- The folder's entries, as PoB's list mode filters and sorts them
function glue.list_builds(subPath)
	subPath = glue.normalise_folder(subPath)
	local mode = prepare(subPath)
	local items = {}
	for _, item in ipairs(mode.list) do
		items[#items + 1] = item
	end
	local entries = {}
	for _, item in ipairs(items) do
		local prompt = delete_prompt(mode, item)
		if item.folderName then
			entries[#entries + 1] = {
				name = item.folderName,
				is_folder = true,
				folder = item.subPath,
				path = item.fullFileName,
				delete_prompt = prompt,
			}
		else
			entries[#entries + 1] = {
				name = item.buildName,
				is_folder = false,
				folder = item.subPath,
				path = item.fullFileName,
				class = item.className,
				ascendancy = item.ascendClassName,
				level = item.level,
				delete_prompt = prompt,
			}
		end
	end
	local sort = main().buildSortMode
	local result = {
		folder = subPath,
		sort = SORTS[sort] and sort or "NAME",
		filter = main().filterBuildList or "",
	}
	-- an empty Lua table is ambiguous to serde, so leave it out
	if #entries > 0 then result.entries = entries end
	return result
end
