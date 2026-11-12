-- Build lifecycle for the Rust host: view, list, new, open, import, save.
-- Runs after bootstrap.lua, which exposes `pob.main` and `pob.build`.
-- Errors use level 0 so the Rust side shows the message without a position prefix.

glue = glue or {}

-- `pob.main` is the launcher; PoB's own `main` module hangs off it.
local function main_module()
	return pob.main.main
end

-- launch:ShowErrMsg builds "^1Error:\n\n^0<reason>\n^8v<version>\n^0Press Enter..."; keep
-- just the reason, on one line, without colour codes.
local function error_reason(message)
	local lines = {}
	for line in (tostring(message) .. "\n"):gmatch("(.-)\n") do
		if line:find("^%^8v") or line:find("^%^0Press ") then break end
		line = line:gsub("%^x%x%x%x%x%x%x", ""):gsub("%^%d", "")
		if line ~= "" and line ~= "Error:" then lines[#lines + 1] = line end
	end
	if #lines == 0 then return "PoB reported an error" end
	return table.concat(lines, " ")
end

-- For the other glue files: a prompt's reason (R11)
glue.error_reason = error_reason

local function frame()
	-- The tree view calculates node power in a frame when the heat map is on
	-- (PassiveTreeView.lua:773-776); here `glue.power_step` does that, a slice at a time, so a
	-- frame must leave it alone.
	--
	-- PoB draws the tab the build was saved on (`viewMode`, Build.lua:960) in every frame
	-- (Build.lua:1276-1294). The Skills tab's draw resumes the gem DPS sort against whatever
	-- group is shown (GemSelectControl.lua:492-509), and the Calcs tab's sets breakdown data, so
	-- a frame always draws the tree tab instead: it is PoB's default (Build.lua:78), every
	-- fixture and gate test so far ran on it, and its one calculation, node power, is held off
	-- above. Its draw only lays out and reads the tree. The build's own `viewMode` is put back
	-- afterwards, so Save still writes the tab it was saved on. Opening a build sets `viewMode`
	-- in the same frame (Init, then OnFrame), so the swap happens inside OnFrame.
	local build = pob.build
	local viewer = build.treeTab and build.treeTab.viewer
	local heatmap = viewer and viewer.showHeatMap
	if viewer then viewer.showHeatMap = false end
	local onFrame = rawget(build, "OnFrame")
	build.OnFrame = function(self, ...)
		local saved = self.viewMode
		self.viewMode = "TREE"
		local done, failure = pcall(onFrame, self, ...)
		self.viewMode = saved
		if not done then error(failure, 0) end
	end
	local ok, err = pcall(runCallback, "OnFrame")
	build.OnFrame = onFrame
	if viewer then viewer.showHeatMap = heatmap end
	if not ok then error(err, 0) end
	-- Launch.lua catches errors from a frame and shows them as a prompt
	local message = pob.main.promptMsg
	if message then
		pob.main.promptMsg = nil
		error(error_reason(message), 0)
	end
end

-- For the other glue files: a frame that doesn't calculate (unless a flag asks for it) but
-- lets PoB refresh `unsaved` (Build.lua:1303).
glue.frame = frame

-- Switches mode and runs a frame so PoB initialises the build and calculates it.
-- A build file PoB can't load sends it back to the list; fall back to a new build then.
local function enter_build(...)
	main_module():SetMode("BUILD", ...)
	frame()
	if main_module().mode ~= "BUILD" then
		main_module():SetMode("BUILD", false, "Unnamed build")
		frame()
		error("couldn't open that build", 0)
	end
end

-- Whether PoB has a build initialised. It starts on the build list when Settings.xml says so
-- (Main.lua), and then `pob.build` is the uninitialised BUILD mode: no spec, no tabs.
function glue.build_loaded()
	return main_module().mode == "BUILD" and pob.build.spec ~= nil
end

-- Recalculates the open build, raising any error PoB hit while doing it.
function glue.recalculate()
	pob.build.buildFlag = true
	frame()
end

-- ---- The top bar (R21) ----

-- The top bar's dropdowns as PoB's frame leaves them before it draws (Build.lua:1232-1236):
-- the class and ascendancy selections and the bloodline list. The loadouts list is not
-- rebuilt in a frame; PoB rebuilds it only in `SyncLoadouts`, which every set change calls.
local function top_bar_sync()
	local build = pob.build
	local controls = build.controls
	controls.classDrop:SelByValue(build.spec.curClassId, "classId")
	controls.ascendDrop.list = controls.classDrop:GetSelValueByKey("ascendancies")
	controls.ascendDrop:SelByValue(build.spec.curAscendClassId, "ascendClassId")
	build:UpdateSecondaryAscendancyDropdown()
	return controls
end

-- PoB marks the loadouts dropdown's headers and actions with a doubled colour code
-- (Build.lua:851), so no set title can match them
local function loadout_marker(entry)
	return type(entry) ~= "string" or entry:sub(1, 4) == "^7^7"
end

local function require_usable(control)
	if not (control:IsShown() and control:IsEnabled()) then
		error("PoB doesn't offer that here", 0)
	end
end

-- What hovering the level box shows (Build.lua:227-253)
local function level_tooltip(control)
	if not control.tooltipFunc then
		return nil
	end
	local tooltip = new("Tooltip")
	control.tooltipFunc(tooltip)
	return glue.read_tooltip(tooltip)
end

function glue.build_view()
	local build = pob.build
	local spec = build.spec
	local ascendancy = spec.curAscendClassName
	if ascendancy == "None" or ascendancy == "" then ascendancy = nil end
	local controls = top_bar_sync()
	local secondary = {}
	for i, entry in ipairs(controls.secondaryAscendDrop.list) do
		secondary[i] = { entry.ascendClassId, entry.label }
	end
	local loadouts = {}
	for i, entry in ipairs(controls.buildLoadouts.list) do
		loadouts[i] = tostring(entry)
	end
	local sel = controls.buildLoadouts.selIndex
	local loadout_selected
	if sel and loadouts[sel] and not loadout_marker(loadouts[sel]) then
		loadout_selected = sel - 1
	end
	return {
		name = build.buildName or "",
		path = build.dbFileName or nil,
		class = spec.curClassName or "",
		ascendancy = ascendancy,
		level = build.characterLevel or 1,
		unsaved = build.unsaved and true or false,
		can_undo = build.configTab.undo[2] ~= nil,
		can_redo = build.configTab.redo[1] ~= nil,
		level_auto = build.characterLevelAutoMode and true or false,
		secondary = secondary,
		secondary_selected = spec.curSecondaryAscendClassId or 0,
		loadouts = loadouts,
		loadout_selected = loadout_selected,
		progress = controls.pointDisplay.req or "",
		level_tooltip = level_tooltip(controls.characterLevel),
	}
end

-- Typed into the level box (Build.lua:218): PoB's `%D` filter and 3-character limit, then
-- its callback, which clamps the level and switches to Manual
function glue.set_character_level(level)
	local control = pob.build.controls.characterLevel
	require_usable(control)
	if not glue.type_into(control, tostring(level)) then
		error("that isn't a level", 0)
	end
end

-- The Auto/Manual button (Build.lua:211), pressed when the mode differs
function glue.set_level_mode(auto)
	local build = pob.build
	local button = build.controls.levelScalingButton
	require_usable(button)
	if (build.characterLevelAutoMode and true or false) ~= auto then
		button.onClick()
	end
end

-- A pick in the bloodline dropdown (Build.lua:285), from the list PoB's frame builds
function glue.select_secondary_ascendancy(id)
	local drop = top_bar_sync().secondaryAscendDrop
	require_usable(drop)
	for index, entry in ipairs(drop.list) do
		if entry.ascendClassId == id then
			drop:SetSel(index)
			return
		end
	end
	error("PoB doesn't offer that bloodline here", 0)
end

-- The loadouts dropdown's entry with this text, by its 1-based index
local function loadout_index(label)
	local drop = pob.build.controls.buildLoadouts
	require_usable(drop)
	for index, entry in ipairs(drop.list) do
		if entry == label then
			return drop, index
		end
	end
	error("no such loadout", 0)
end

-- A pick of a loadout (Build.lua:301-415); PoB's headers and actions are not loadouts
function glue.select_loadout(label)
	local drop, index = loadout_index(label)
	if loadout_marker(label) then
		error("PoB doesn't offer that here", 0)
	end
	drop:SetSel(index)
end

-- "New Loadout", its name popup and Save (Build.lua:316-352)
function glue.new_loadout(name)
	local drop, index = loadout_index("^7^7New Loadout")
	glue.in_popups(function()
		local main = main_module()
		local depth = #main.popups
		drop:SetSel(index)
		if #main.popups <= depth then
			error("PoB didn't open that window", 0)
		end
		glue.save_name("loadout", name)
	end)
end

-- "Sync" (Build.lua:306-309)
function glue.sync_loadouts()
	local drop, index = loadout_index("^7^7Sync")
	drop:SetSel(index)
end

-- A folder is "" or ends with "/", so it can be joined to a file name.
local function normalise_folder(folder)
	if folder ~= "" and folder:sub(-1) ~= "/" then folder = folder .. "/" end
	return folder
end
glue.normalise_folder = normalise_folder

-- For the import glue: switches mode and runs a frame, as opening a build does.
glue.enter_build = enter_build

function glue.new_build()
	enter_build(false, "Unnamed build")
end

function glue.open_build(fullPath)
	local name = fullPath:match("([^/\\]+)%.xml$") or fullPath
	enter_build(fullPath, name)
end

function glue.import_code(code)
	code = code:gsub("^%s+", ""):gsub("%s+$", "")
	if code:sub(1, 4):lower() == "http" then
		error("that's a link; use Import from URL", 0)
	end
	local ok, xml = pcall(function()
		return Inflate(common.base64.decode((code:gsub("-", "+"):gsub("_", "/"))))
	end)
	if not ok or not xml or xml == "" then
		error("that isn't a valid build code", 0)
	end
	enter_build(false, "Imported build", xml)
end

function glue.save()
	local build = pob.build
	if not build.dbFileName then
		error("build has no file yet", 0)
	end
	if build:SaveDBFile() then
		error("couldn't save the build file", 0)
	end
	-- refresh `unsaved`
	frame()
end

function glue.save_as(subPath, name, overwrite)
	if name == "" or name:find("[/\\]") then
		error("that isn't a valid build name", 0)
	end
	if subPath:find("%.%.") or subPath:sub(1, 1) == "/" then
		error("that isn't a valid folder", 0)
	end
	subPath = normalise_folder(subPath)
	local build = pob.build
	local main = main_module()
	local fullPath = main.buildPath .. subPath .. name .. ".xml"
	if not overwrite then
		local file = io.open(fullPath, "r")
		if file then
			file:close()
			error("a build named '" .. name .. "' exists", 0)
		end
	end
	local folder = main.buildPath .. subPath
	local createdFolder = MakeDir(folder)
	local old = { build.dbFileName, build.buildName, build.dbFileSubPath }
	build.dbFileName = fullPath
	build.buildName = name
	build.dbFileSubPath = subPath
	local ok, err = pcall(glue.save)
	if not ok then
		build.dbFileName, build.buildName, build.dbFileSubPath = old[1], old[2], old[3]
		-- don't leave behind a folder this call made; RemoveDir only removes empty ones
		if createdFolder then RemoveDir(folder) end
		error(err, 0)
	end
end

-- ---- Help ----

-- The title rows' height in OpenAboutPopup (Main.lua:1337, `titleSize`)
local HELP_TITLE_SIZE = 24

-- The rows of an About window list as sections: a title row ("^7<title>" at titleSize) starts
-- one; the rows under it are kept with a tab between two cells; spacer rows have no cells.
local function about_sections(list, strip)
	local sections = {}
	for _, row in ipairs(list) do
		if row.height == HELP_TITLE_SIZE and row[1] then
			sections[#sections + 1] = { title = strip(row[1]), lines = {} }
		elseif row[1] and #sections > 0 then
			local lines = sections[#sections].lines
			lines[#lines + 1] = row[2] and (row[1] .. "\t" .. row[2]) or row[1]
		end
	end
	return sections
end

-- PoB's help as its About window lists it: main:OpenAboutPopup(1) reads help.txt and builds
-- the list (Main.lua:1360-1407); the rows are read off its TextListControl, then its "Version
-- history:" button swaps in the changelog's (Main.lua:1425-1428), and the window is closed.
function glue.help_view()
	-- without the file OpenAboutPopup(1) indexes a missing section (Main.lua:1407-1416)
	local file = io.open("help.txt", "r")
	if not file then
		error("PoB's help file (help.txt) isn't installed", 0)
	end
	file:close()
	local main = main_module()
	local view
	glue.in_popups(function()
		main:OpenAboutPopup(1)
		local controls = main.popups[1].controls
		local sections = about_sections(controls.changelog.list, function(title)
			-- "^7<title> (<n>)" (Main.lua:1371)
			return (title:gsub("^%^7", ""):gsub(" %(%d+%)$", ""))
		end)
		controls.verLabel.onClick()
		local changelog = about_sections(controls.changelog.list, function(title)
			return (title:gsub("^%^7", ""))
		end)
		view = {
			sections = sections,
			header = { controls.version.label, controls.forum.label, controls.github.label },
			changelog = changelog,
		}
	end)
	return view
end
