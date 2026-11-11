-- PoB's list builders for the lists with a coroutine builder (`ItemDBControl`, the anoint
-- `NotableDBControl`): the draw-only sync Draw does before a list is read (R14, R17).

glue = glue or {}

-- Draw's preamble: restart the builder on a new flag or output revision. A restart wipes
-- the list, so rows are only read once the builder has ended.
local function restart(control)
	local tab = control.itemsTab
	if tab and tab.build.outputRevision ~= control.listOutputRevision then
		control.listBuildFlag = true
	end
	if control.listBuildFlag then
		control.listBuildFlag = false
		wipeTable(control.list)
		control.listBuilder = coroutine.create(control.ListBuilder)
		control.listOutputRevision = tab and tab.build.outputRevision
	end
end

-- Resumes the builder once, as Draw does; true when it has ended. A failed resume is an
-- error, where PoB swallows it outside dev mode.
local function resume(control)
	local ok, err = coroutine.resume(control.listBuilder, control)
	if not ok then
		control.listBuilder = nil
		error(tostring(err), 0)
	end
	if coroutine.status(control.listBuilder) == "dead" then
		control.listBuilder = nil
		return true
	end
	return false
end

-- Whether the list has builder work: a pending restart or a live builder
function glue.list_pending(control)
	local tab = control.itemsTab
	return (control.listBuildFlag or control.listBuilder ~= nil
		or (tab ~= nil and tab.build.outputRevision ~= control.listOutputRevision)) and true or false
end

-- What Draw does before a list control reads its rows. A name sort never yields, so the
-- builder is run to its end here; a stat sort is a background job.
function glue.list_sync(control)
	if not control.ListBuilder then
		return
	end
	restart(control)
	if control.listBuilder and not (control.sortDetail and control.sortDetail.stat) then
		resume(control)
	end
end

-- One resume of the list builder after Draw's restart check. `percent` comes from PoB's
-- own "Sorting... (N%)" text.
function glue.list_step(control)
	restart(control)
	if not control.listBuilder then
		return { finished = true }
	end
	local finished = resume(control)
	local percent
	if not finished then
		percent = tonumber(tostring(control.defaultText or ""):match("%((%d+)%%%)"))
	end
	return { finished = finished, percent = percent }
end
