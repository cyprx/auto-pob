-- PoB's settings with per-setting overrides (R12, R13).
--
-- `Settings.xml` is PoB's and shared with classic PoB. An override is the app's own value for
-- one setting: the glue holds it in `main` (or `launch`) while PoB runs and puts the shared
-- value back around every `main:SaveSettings()`, so the file never learns of it. The overrides
-- themselves live in `auto-pob.toml`, which the UI owns: this file only receives them
-- (`set_overrides`) and reports the ones an Options Save changed (`saved_overrides`).

glue = glue or {}

local function main()
	return pob.main.main
end

local function launch()
	return pob.main
end

-- Every setting the Options popup and the build list sort offer, except the three that are
-- ruled out here (R13): `betaTest`, `showPublicBuilds` and `disableDevAutoSave`. `control` is
-- the popup control that edits it ("" for none). `kind` says how the string is read.
-- `effect` says who acts on a change: PoB's own text, the UI, or nobody.
local function clamp_gem_quality(n)
	return math.min(n, 23)
end
local function clamp_level(n)
	return math.min(math.max(n, 1), 100)
end
local function clamp_affix_quality(n)
	return math.min(n, 1)
end

local function colour(code)
	return function(value)
		updateColorCode(code, value)
	end
end

local SPEC = {
	{ key = "connectionProtocol", control = "connectionProtocol", label = "Connection Protocol", kind = "number", launch = true, default = 0, effect = "PobText" },
	{ key = "proxyURL", control = "proxyURL", label = "Proxy server", kind = "string", launch = true, effect = "PobText" },
	{ key = "dpiScaleOverridePercent", control = "dpiScaleOverride", label = "UI scaling override", kind = "number", default = 0, effect = "Ui",
		after = function(value) SetDPIScaleOverridePercent(value) end },
	{ key = "buildPath", control = "buildPath", label = "Build save path", kind = "string", effect = "PobText" },
	{ key = "nodePowerTheme", control = "nodePowerTheme", label = "Node Power colours", kind = "string", effect = "Ui" },
	{ key = "colorPositive", control = "colorPositive", label = "Hex colour for positive values", kind = "string", effect = "PobText", after = colour("POSITIVE") },
	{ key = "colorNegative", control = "colorNegative", label = "Hex colour for negative values", kind = "string", effect = "PobText", after = colour("NEGATIVE") },
	{ key = "colorHighlight", control = "colorHighlight", label = "Hex colour for highlight nodes", kind = "string", effect = "PobText", after = colour("HIGHLIGHT") },
	{ key = "edgeSearchHighlight", control = "edgeSearchHighlight", label = "Show search circles at viewport edge", kind = "bool", effect = "Ui" },
	{ key = "showFlavourText", control = "showFlavourText", label = "Styled Tooltips with Flavour Text", kind = "bool", effect = "PobText" },
	{ key = "showAnimations", control = "showAnimations", label = "Show Animations", kind = "bool", effect = "NoEffect" },
	{ key = "showAllItemAffixes", control = "showAllItemAffixes", label = "Show all item affixes sliders", kind = "bool", effect = "PobText" },
	{ key = "disableScrollControlInteraction", control = "disableScrollControlInteraction", label = "Disable control scroll interaction", kind = "bool", effect = "NoEffect" },
	{ key = "showThousandsSeparators", control = "showThousandsSeparators", label = "Show thousands separators", kind = "bool", effect = "PobText" },
	{ key = "useCompactValues", control = "useCompactValues", label = "Compact large numbers", kind = "bool", effect = "PobText" },
	{ key = "thousandsSeparator", control = "thousandsSeparator", label = "Thousands separator", kind = "string", effect = "PobText" },
	{ key = "decimalSeparator", control = "decimalSeparator", label = "Decimal separator", kind = "string", effect = "PobText" },
	{ key = "showTitlebarName", control = "titlebarName", label = "Show build name in window title", kind = "bool", effect = "Ui" },
	{ key = "defaultGemQuality", control = "defaultGemQuality", label = "Default gem quality", kind = "number", default = 0, clamp = clamp_gem_quality, effect = "PobText" },
	{ key = "defaultCharLevel", control = "defaultCharLevel", label = "Default character level", kind = "number", default = 1, clamp = clamp_level, effect = "PobText" },
	{ key = "defaultItemAffixQuality", control = "defaultItemAffixQualitySlider", label = "Default item affix quality", kind = "number", default = 0.5, clamp = clamp_affix_quality, effect = "PobText" },
	{ key = "showWarnings", control = "showWarnings", label = "Show build warnings", kind = "bool", effect = "PobText" },
	{ key = "slotOnlyTooltips", control = "slotOnlyTooltips", label = "Show tooltips only for affected slots", kind = "bool", effect = "PobText" },
	{ key = "migrateEldritchImplicits", control = "migrateEldritchImplicits", label = "Copy Eldritch Implicits onto Display Item", kind = "bool", effect = "PobText" },
	{ key = "notSupportedModTooltips", control = "notSupportedModTooltips", label = "Show tooltip for unsupported mods", kind = "bool", effect = "PobText" },
	{ key = "invertSliderScrollDirection", control = "invertSliderScrollDirection", label = "Invert slider scroll direction", kind = "bool", effect = "NoEffect" },
	{ key = "buildSortMode", control = "", label = "Build list sort", kind = "string", effect = "PobText" },
}

local BY_KEY = {}
for _, spec in ipairs(SPEC) do
	BY_KEY[spec.key] = spec
end

-- the Options controls the app doesn't offer: their settings are never changed from here
local HIDDEN_CONTROLS = { betaTest = true, showPublicBuilds = true, disableDevAutoSave = true }

local shared = {} -- key -> the value Settings.xml holds (kept up to date while not overridden)
local overrides = {} -- key -> the app's own value
local saved_overrides = {} -- overrides an Options Save changed, taken by the next view

local function holder(spec)
	return spec.launch and launch() or main()
end

-- A setting as its string
local function read(spec)
	local value = holder(spec)[spec.key]
	if value == nil then
		value = spec.default
	end
	if spec.kind == "bool" then
		return value and "true" or "false"
	end
	if value == nil then
		return ""
	end
	return tostring(value)
end

-- Parses `text` as the setting's value; raises for one that isn't
local function parse(spec, text)
	if type(text) ~= "string" then
		error("setting " .. spec.key .. " must be text", 0)
	end
	if spec.kind == "bool" then
		if text ~= "true" and text ~= "false" then
			error("setting " .. spec.key .. " must be true or false", 0)
		end
		return text == "true"
	elseif spec.kind == "number" then
		local n = tonumber(text)
		if not n then
			error("setting " .. spec.key .. " must be a number", 0)
		end
		return spec.clamp and spec.clamp(n) or n
	end
	if spec.key == "proxyURL" and text == "" then
		return nil
	elseif spec.key == "buildPath" then
		if text == "" then
			return main().defaultBuildPath
		end
		-- as the Options Save writes it
		return text:match("[\\/]$") and text or (text .. "/")
	end
	return text
end

-- Puts a setting in place, with what PoB does on loading it
local function write(spec, text)
	local value = parse(spec, text)
	holder(spec)[spec.key] = value
	if spec.after and value ~= nil then
		spec.after(value)
	end
end

local function options_open()
	return glue.popup_is_options and glue.popup_is_options()
end

-- Settings.xml's bytes as we last read or wrote them: another program (classic PoB) may write
-- the file while we run, and a shared item edit must not overwrite that (I-2 ruling)
local known_bytes

-- The file's bytes now; nil without a file (or one we can't open)
local function file_bytes()
	local file = io.open(main().userPath .. "Settings.xml", "rb")
	if not file then
		return nil
	end
	local bytes = file:read("*a")
	file:close()
	return bytes
end

-- Whether Settings.xml differs from what we last read or wrote
function glue.settings_changed_outside()
	return file_bytes() ~= known_bytes
end

-- Refuses a write the user asked for (a shared item edit, the build sort, Options' Save) when
-- it would lose something or never happen: another program (classic PoB) wrote the file since
-- we last read or wrote it (I-2 ruling), or PoB couldn't read it at start, which makes its
-- `SaveSettings` a silent no-op (Main.lua:702-704). `what` ends the notice.
function glue.settings_check_writable(what)
	if main().errorReadingSettings then
		error("Settings.xml couldn't be read when auto-pob started; restart auto-pob to " .. what, 0)
	end
	if glue.settings_changed_outside() then
		error("Settings.xml was changed by another program; restart auto-pob to " .. what, 0)
	end
end

-- Raises why a `main:SaveSettings()` that returned `failed` didn't write: PoB shows the reason
-- in a prompt and returns true (Main.lua:719-722, 776-779), and the reason belongs to the
-- command that saved, not to the next frame
function glue.settings_save_failed(failed)
	if failed == true then
		local prompt = pob.main.promptMsg
		pob.main.promptMsg = nil
		error(prompt and glue.error_reason(prompt) or "couldn't save Settings.xml", 0)
	end
end

-- PoB's exit save (`main:Shutdown`, Main.lua:338-339): what PoB keeps in memory until then
-- (the import history's accounts, the last account name, the open build as `<Mode>`) is
-- written, through the wrapper below. A file another program wrote, or one PoB couldn't read,
-- is left alone. Returns why nothing was written, or nil.
function glue.settings_exit_save()
	local ok, err = pcall(glue.settings_check_writable, "save at exit")
	if not ok then
		return tostring(err)
	end
	local saved, failed = pcall(main().SaveSettings, main())
	if not saved then
		return tostring(failed)
	end
	local reported, reason = pcall(glue.settings_save_failed, failed)
	if not reported then
		return tostring(reason)
	end
	return nil
end

-- Records `shared` after PoB's `LoadSettings` and wraps `main:SaveSettings` (R12)
function glue.settings_init()
	known_bytes = file_bytes()
	for _, spec in ipairs(SPEC) do
		shared[spec.key] = read(spec)
	end
	local original = main().SaveSettings
	main().SaveSettings = function(self, ...)
		-- the window's unsaved edits are never written (R29)
		glue.popup_cancel_for_save()
		-- a changed overridden setting was edited by the user: the override takes it
		for key, text in pairs(overrides) do
			local now = read(BY_KEY[key])
			if now ~= text then
				overrides[key] = now
				saved_overrides[key] = now
			end
		end
		for _, spec in ipairs(SPEC) do
			if overrides[spec.key] == nil then
				shared[spec.key] = read(spec)
			end
		end
		-- a write over another program's (a token save, which can't be refused) leaves the
		-- outside change on record, so the user's own writers stay refused until a restart
		local outside = glue.settings_changed_outside()
		local ok, result = pcall(function(...)
			for key in pairs(overrides) do
				write(BY_KEY[key], shared[key])
			end
			return original(self, ...)
		end, ...)
		for key, text in pairs(overrides) do
			write(BY_KEY[key], text)
		end
		if not ok then
			error(result, 0)
		end
		-- PoB returns true when it couldn't write (Main.lua:719-722, 776-779), and writes
		-- nothing after a failed read (Main.lua:702-704)
		if result ~= true and not outside and not self.errorReadingSettings then
			known_bytes = file_bytes()
		end
		return result
	end
end

-- Puts every override back in PoB (after a Cancel restored the window's snapshot)
function glue.settings_reapply()
	for key, text in pairs(overrides) do
		write(BY_KEY[key], text)
	end
end

-- Replaces the override map. Keys this build doesn't know are ignored; a value that doesn't
-- parse raises before anything changes.
function glue.set_overrides(map)
	for key, text in pairs(map) do
		local spec = BY_KEY[key]
		if spec then
			parse(spec, text)
		end
	end
	for key in pairs(overrides) do
		if map[key] == nil or not BY_KEY[key] then
			write(BY_KEY[key], shared[key])
			overrides[key] = nil
		end
	end
	for key, text in pairs(map) do
		local spec = BY_KEY[key]
		if spec then
			if overrides[key] == nil then
				-- while Options is open the live value may be an unsaved edit
				if not (options_open() and shared[key] ~= nil) then
					shared[key] = read(spec)
				end
			end
			write(spec, text)
			overrides[key] = read(spec)
		end
	end
end

function glue.settings_view()
	local rows = {}
	for _, spec in ipairs(SPEC) do
		local overridden = overrides[spec.key] ~= nil
		table.insert(rows, {
			key = spec.key,
			control = spec.control,
			label = spec.label,
			shared = overridden and shared[spec.key] or read(spec),
			value = read(spec),
			overridden = overridden,
			effect = spec.effect,
		})
	end
	local view = { rows = rows, saved_overrides = saved_overrides }
	saved_overrides = {}
	return view
end

-- The Options controls that are not offered (R13)
function glue.settings_hidden(name)
	return HIDDEN_CONTROLS[name] == true
end
