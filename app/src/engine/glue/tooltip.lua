-- A recorder with PoB's `Tooltip` method set, so tooltip functions can be run without drawing.
-- Only the lines are kept: `AddLine` records `{ size, text }` (split at line breaks like PoB's
-- own), `AddSeparator` records `{ separator = size }`; everything else does nothing.

glue = glue or {}

local Recorder = {}
Recorder.__index = Recorder

function Recorder:Clear()
	self._lines = {}
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

function Recorder:AddSeparator(size)
	self._lines[#self._lines + 1] = { separator = size or 10 }
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
