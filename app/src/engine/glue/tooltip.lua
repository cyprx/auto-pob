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

function Recorder:SetRecipe() end

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
