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
