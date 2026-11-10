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

-- Recalculates the open build, raising any error PoB hit while doing it.
function glue.recalculate()
	pob.build.buildFlag = true
	frame()
end

function glue.build_view()
	local build = pob.build
	local spec = build.spec
	local ascendancy = spec.curAscendClassName
	if ascendancy == "None" or ascendancy == "" then ascendancy = nil end
	return {
		name = build.buildName or "",
		path = build.dbFileName or nil,
		class = spec.curClassName or "",
		ascendancy = ascendancy,
		level = build.characterLevel or 1,
		unsaved = build.unsaved and true or false,
		can_undo = build.configTab.undo[2] ~= nil,
		can_redo = build.configTab.redo[1] ~= nil,
	}
end

-- A folder is "" or ends with "/", so it can be joined to a file name.
local function normalise_folder(folder)
	if folder ~= "" and folder:sub(-1) ~= "/" then folder = folder .. "/" end
	return folder
end

function glue.list_builds(subPath)
	subPath = normalise_folder(subPath)
	local helpers = LoadModule("Modules/BuildListHelpers")
	local index = helpers.ScanFolder(subPath)
	local list = helpers.FilterList(index, subPath, "")
	helpers.SortList(list, "NAME")
	local entries = {}
	for _, item in ipairs(list) do
		if item.folderName then
			entries[#entries + 1] = {
				name = item.folderName,
				is_folder = true,
				path = item.fullFileName,
			}
		else
			entries[#entries + 1] = {
				name = item.buildName,
				is_folder = false,
				path = item.fullFileName,
				class = item.className,
				ascendancy = item.ascendClassName,
				level = item.level,
			}
		end
	end
	local result = { folder = subPath }
	-- an empty Lua table is ambiguous to serde, so leave it out
	if #entries > 0 then result.entries = entries end
	return result
end

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
		error("links need the network features, which aren't available yet", 0)
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
