-- A recorder with PoB's `Tooltip` method set, so tooltip functions can be run without drawing.
-- Only the lines are kept: `AddLine` records `{ size, text }` (split at line breaks like PoB's
-- own), `AddSeparator` records `{ separator = size }` and drops a second one as PoB's does;
-- everything else does nothing.

glue = glue or {}

local Recorder = {}
Recorder.__index = Recorder

function Recorder:Clear()
	self._lines = {}
	self._imaged = nil
	self._recipe = nil
	self.tooltipHeader = false
end

-- tooltips ask whether to rebuild themselves; a fresh recorder always should
function Recorder:CheckForUpdate()
	return true
end

function Recorder:AddLine(size, text)
	if text then
		for line in (text .. "\n"):gmatch("([^\n]*)\n") do
			self._lines[#self._lines + 1] = { size = size, text = line }
		end
	end
end

-- PoB's own drops a separator right after one that was added under a `tooltipHeader` (an
-- item's, a gem's or a node's tooltip): that one has an image (Tooltip.lua:116-123). Without
-- a header, two in a row stay.
function Recorder:AddSeparator(size)
	local n = #self._lines
	if n > 0 and self._imaged == n then
		return
	end
	self._lines[n + 1] = { separator = size or 10 }
	if self.tooltipHeader then
		self._imaged = n + 1
	end
end

-- the oils of an anointable notable, which PoB draws after its name (Tooltip.lua:112-114)
function Recorder:SetRecipe(recipe)
	if recipe then
		local list = {}
		for i, name in ipairs(recipe) do
			list[i] = name
		end
		self._recipe = list
	else
		self._recipe = nil
	end
end

function Recorder:GetSize()
	return 0, 0
end

function Recorder:GetDynamicSize()
	return 0, 0
end

function Recorder:CalculateColumns() end

function Recorder:Draw() end

function Recorder:lines()
	return self._lines
end

function glue.tooltip()
	return setmetatable({ _lines = {} }, Recorder)
end

-- A node tooltip as data: its lines, PoB's header art (`tooltipHeader`), the oils of an
-- anointable notable, and the granted skill's tooltip PoB draws beside it (`skill`, the viewer's
-- `skillTooltip` recorder; PassiveTreeView.lua:1044-1097, 1460, 1605-1625)
function glue.node_tooltip_data(id, tooltip, skill, trace)
	local out = { id = id, lines = tooltip:lines(), trace = trace or {} }
	if tooltip.tooltipHeader then
		out.header = tostring(tooltip.tooltipHeader)
	end
	if tooltip._recipe and #tooltip._recipe > 0 then
		out.recipe = tooltip._recipe
	end
	if skill and #skill:lines() > 0 then
		out.skill = { lines = skill:lines() }
		if skill.tooltipHeader then
			out.skill.header = tostring(skill.tooltipHeader)
		end
	end
	return out
end

-- Runs `fn(...)` with the viewer's skill tooltip swapped for a recorder; returns the recorder
-- and `fn`'s result. The viewer's own is put back, whatever happens.
function glue.with_skill_recorder(viewer, fn, ...)
	local saved = viewer.skillTooltip
	local skill = glue.tooltip()
	viewer.skillTooltip = skill
	local ok, err = pcall(fn, ...)
	viewer.skillTooltip = saved
	if not ok then
		error(err, 0)
	end
	return skill
end
