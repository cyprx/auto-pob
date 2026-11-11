-- Helpers for pressing PoB's controls as its own UI would. Shared by the glue files.

glue = glue or {}

-- Runs `fn`, then closes every popup it left open, also when it fails.
function glue.in_popups(fn)
	local popups = pob.main.main.popups
	local depth = #popups
	local ok, err = pcall(fn)
	while #popups > depth do
		pob.main.main:ClosePopup()
	end
	if not ok then
		error(err, 0)
	end
end

-- Presses the confirm button of the newest popup
function glue.confirm_popup(what)
	local popup = pob.main.main.popups[1]
	local button = popup and popup.controls.confirm
	if not (button and button.onClick) then
		error("couldn't " .. what, 0)
	end
	button.onClick()
end

-- Types into an edit control as PoB's EditControl would (R3)
function glue.type_into(control, text)
	text = text:gsub("\r", "")
	local typed = text
	text = text:gsub(control.filterPattern, "")
	-- everything was filtered out: EditControl:Insert does nothing
	if typed ~= "" and text == "" then
		return false
	end
	if control.limit and #text > control.limit then
		error("the text is too long", 0)
	end
	control:SetText(text, true)
	return true
end

function glue.press_check(control, on)
	control.state = on
	control.changeFunc(on)
end

-- Pastes into an edit control as PoB's EditControl does on Ctrl+V (EditControl.lua:530): the
-- text goes through the control's `pasteFilter`, then is typed as any other text.
function glue.paste_into(control, text)
	if control.pasteFilter then
		text = control.pasteFilter(text)
	end
	text = text:gsub("[\128-\255]", "?")
	return glue.type_into(control, text)
end

-- ---- Item and skill sets: the manage popup and its name popup ----

-- Opens a set manager with `open()`, calls `fn(list)` with its set list control and closes
-- what is left open. `what` names the sets ("skill set", "item set") in errors.
function glue.with_set_list(what, open, fn)
	glue.in_popups(function()
		open()
		for _, control in pairs(pob.main.main.popups[1].controls) do
			if control.controls and control.controls.new and control.OnSelDelete then
				return fn(control)
			end
		end
		error("couldn't open the " .. what .. " list", 0)
	end)
end

function glue.check_title(what, title)
	if not title:gsub("\r", ""):match("%S") then
		error("the " .. what .. " needs a name", 0)
	end
end

-- Types the title into the name popup `RenameSet` opened and presses Save (Save stays disabled
-- until the name has a visible character). When that fails, Cancel is pressed, which drops a
-- set that New or Copy had already made.
function glue.save_name(what, title)
	local popup = pob.main.main.popups[1]
	local ok, applied = pcall(glue.type_into, popup.controls.edit, title)
	if ok and applied and popup.controls.save.enabled then
		popup.controls.save.onClick()
		return
	end
	popup.controls.cancel.onClick()
	if not ok then
		error(applied, 0)
	end
	-- the filter emptied the title, or Save stayed disabled: either way it is blank
	glue.check_title(what, "")
end
