-- Build lifecycle for the Rust host: view, list, new, open, import, save.
-- Runs after bootstrap.lua, which exposes `pob.main` and `pob.build`.
-- Errors use level 0 so the Rust side shows the message without a position prefix.

glue = glue or {}

-- `pob.main` is the launcher; PoB's own `main` module hangs off it.
local function main_module()
	return pob.main.main
end

local function frame()
	runCallback("OnFrame")
	-- Launch.lua catches errors from a frame and shows them as a prompt
	local message = pob.main.promptMsg
	if message then
		pob.main.promptMsg = nil
		error((tostring(message):gsub("%^%x", ""):match("[^\n]+") or "PoB reported an error"), 0)
	end
end

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
	MakeDir(main.buildPath .. subPath)
	local old = { build.dbFileName, build.buildName, build.dbFileSubPath }
	build.dbFileName = fullPath
	build.buildName = name
	build.dbFileSubPath = subPath
	local ok, err = pcall(glue.save)
	if not ok then
		build.dbFileName, build.buildName, build.dbFileSubPath = old[1], old[2], old[3]
		error(err, 0)
	end
end
