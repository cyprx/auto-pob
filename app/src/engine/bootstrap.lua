-- Our startup for Path of Building, based on its HeadlessWrapper.lua.
-- Differences: native Inflate/Deflate/NewFileSearch/paths, ConPrintf logs, startup
-- errors raise, LaunchSubScript returns nil, and the result is exposed as `pob`.
-- It runs inside our embedded LuaJIT (mlua) with native Rust helpers for the host functions.



-- Callbacks
local callbackTable = { }
local mainObject
function runCallback(name, ...)
	if callbackTable[name] then
		return callbackTable[name](...)
	elseif mainObject and mainObject[name] then
		return mainObject[name](mainObject, ...)
	end
end
function SetCallback(name, func)
	callbackTable[name] = func
end
function GetCallback(name)
	return callbackTable[name]
end
function SetMainObject(obj)
	mainObject = obj
end

-- Image Handles
local imageHandleClass = { }
imageHandleClass.__index = imageHandleClass
function NewImageHandle()
	return setmetatable({ }, imageHandleClass)
end
function imageHandleClass:Load(fileName, ...)
	self.valid = true
end
function imageHandleClass:Unload()
	self.valid = false
end
function imageHandleClass:IsValid()
	return self.valid
end
function imageHandleClass:SetLoadingPriority(pri) end
function imageHandleClass:ImageSize()
	return 1, 1
end

-- Rendering
function RenderInit(flag, ...) end
function GetScreenSize()
	return 1920, 1080
end
function GetScreenScale()
	return 1
end
function GetVirtualScreenSize()
	return GetScreenSize()
end
function GetDPIScaleOverridePercent()
	return 1
end
function SetDPIScaleOverridePercent(scale) end
function SetClearColor(r, g, b, a) end
function SetDrawLayer(layer, subLayer) end
function SetViewport(x, y, width, height) end
function SetDrawColor(r, g, b, a) end
function DrawImage(imgHandle, left, top, width, height, tcLeft, tcTop, tcRight, tcBottom) end
function DrawImageQuad(imageHandle, x1, y1, x2, y2, x3, y3, x4, y4, s1, t1, s2, t2, s3, t3, s4, t4) end
function DrawString(left, top, align, height, font, text) end
function DrawStringWidth(height, font, text)
	return 1
end
function DrawStringCursorIndex(height, font, text, cursorX, cursorY)
	return 0
end
function StripEscapes(text)
	return text:gsub("%^%d",""):gsub("%^x%x%x%x%x%x%x","")
end
function GetAsyncCount()
	return 0
end

-- Search Handles
-- NewFileSearch is native (Rust)

-- General Functions
function SetWindowTitle(title) end
function GetCursorPos()
	return 0, 0
end
function SetCursorPos(x, y) end
function ShowCursor(doShow) end
function IsKeyDown(keyName) end
function Copy(text) end
function Paste() end
-- Inflate/Deflate are native (Rust)
function GetTime()
	return 0
end
function GetRuntimePath()
	return ""
end
-- RemoveDir is native (Rust)
function SetWorkDir(path) end
function LaunchSubScript(scriptText, funcList, subList, ...) return nil end
function AbortSubScript(ssID) end
function IsSubScriptRunning(ssID) end
function LoadModule(fileName, ...)
	if not fileName:match("%.lua") then
		fileName = fileName .. ".lua"
	end
	local func, err = loadfile(fileName)
	if func then
		return func(...)
	else
		error("LoadModule() error loading '"..fileName.."': "..err)
	end
end
function PLoadModule(fileName, ...)
	if not fileName:match("%.lua") then
		fileName = fileName .. ".lua"
	end
	local func, err = loadfile(fileName)
	if func then
		return PCall(func, ...)
	else
		error("PLoadModule() error loading '"..fileName.."': "..err)
	end
end
function PCall(func, ...)
	local ret = { pcall(func, ...) }
	if ret[1] then
		table.remove(ret, 1)
		return nil, unpack(ret)
	else
		return ret[2]
	end
end
function ConPrintf(fmt, ...)
	__log_info(string.format(fmt, ...))
end
function ConPrintTable(tbl, noRecurse) end
function ConExecute(cmd) end
function ConClear() end
function SpawnProcess(cmdName, args) end
function OpenURL(url) end
function SetProfiling(isEnabled) end
function Restart() end
function Exit() end
function TakeScreenshot() end

---@return string? provider
---@return string? version
---@return number? status
function GetCloudProvider(fullPath)
	return nil, nil, nil
end


local l_require = require
function require(name)
	-- Hack to stop it looking for lcurl, which we don't really need
	if name == "lcurl.safe" then
		return
	end
	return l_require(name)
end


dofile("Launch.lua")

-- The pinned PoB ships a repository-style manifest, so Launch.lua turns on devMode, and
-- Main:Init then keeps the user's data (builds, settings) inside the PoB folder. We want
-- GetUserPath() instead, so devMode is turned off before Main initialises and stays off.
-- Classic release installs run with devMode false too. Leaving it on would enable PoB's
-- "dev autosave" in the build Shutdown, which rewrites build files on every mode switch,
-- and it would disable Launch.lua's crash recovery for a build that fails to load.
do
	local plain_plo = PLoadModule
	function PLoadModule(fileName, ...)
		local err, module = plain_plo(fileName, ...)
		if not err and fileName:match("^Modules/Main") and type(module) == "table" and module.Init then
			local init = module.Init
			module.Init = function(self, ...)
				mainObject.devMode = false
				return init(self, ...)
			end
		end
		return err, module
	end
end

-- never skip ModCache
mainObject.continuousIntegrationMode = nil

runCallback("OnInit")
runCallback("OnFrame") -- Need at least one frame for everything to initialise

if mainObject.promptMsg then
	error(mainObject.promptMsg)
end

pob = {
	main = mainObject,
	build = mainObject.main.modes["BUILD"],
}
