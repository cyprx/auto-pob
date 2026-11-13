-- Helpers for pressing PoB's controls as its own UI would. Shared by the glue files.

glue = glue or {}

-- Whether PoB's string `ours` is the text `sent` the UI showed for it: the views show bytes that
-- aren't UTF-8 as U+FFFD, so a lookup by text compares in that form too
function glue.same_text(ours, sent)
	if ours == sent then
		return true
	end
	if type(ours) ~= "string" or type(sent) ~= "string" then
		return false
	end
	return AutoPobLossy(ours) == sent
end

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

-- Types `text` into the edit control as PoB's EditControl takes an edit that turns its text into
-- it. The filter and limit apply to the inserted part only (EditControl.lua:140-172), so text the
-- box already holds (a name being renamed) stays as it is. Typed text goes in one key at a time:
-- over a selection the first key is ReplaceSel, which refuses a character the filter drops (the
-- selection stays), every key after it is Insert, which drops such a character, and a key that
-- would pass the limit is refused. `pasted` text is one insertion: Insert drops the rejected
-- characters, ReplaceSel refuses the whole text if it has one, and a text that would pass the
-- limit is refused whole (an error here). False when the text changed nothing.
-- While a number box's step is applied (Command::Step): PoB steps an isNumeric box with
-- SetText(text, true), which checks neither filter nor limit (EditControl.lua:680-700)
function glue.set_stepping(on)
	glue.stepping = on
end

function glue.type_into(control, text, pasted)
	text = text:gsub("\r", "")
	if glue.stepping and control.isNumeric then
		control:SetText(text, true)
		return true
	end
	local old = control.buf or ""
	-- the edit replaced old[head + 1 .. #old - tail] with text[head + 1 .. #text - tail]; both
	-- cuts fall between characters
	local head = 0
	while head < #old and head < #text and old:byte(head + 1) == text:byte(head + 1) do
		head = head + 1
	end
	while head > 0 and (text:byte(head + 1) or 0) >= 0x80 and (text:byte(head + 1) or 0) < 0xC0 do
		head = head - 1
	end
	local tail = 0
	while tail < #old - head and tail < #text - head
		and old:byte(#old - tail) == text:byte(#text - tail) do
		tail = tail + 1
	end
	while tail > 0 and (text:byte(#text - tail + 1) or 0) >= 0x80 and (text:byte(#text - tail + 1) or 0) < 0xC0 do
		tail = tail - 1
	end
	local front, back = text:sub(1, head), text:sub(#text - tail + 1)
	local inserted = text:sub(head + 1, #text - tail)
	local replacing = head + tail < #old
	local limit = control.limit
	local kept, refused = "", false
	if inserted ~= "" then
		if pasted then
			kept = inserted:gsub(control.filterPattern, "")
			if kept == "" or (replacing and kept ~= inserted) then
				return false
			end
			if limit and #front + #kept + #back > limit then
				error("the text is too long", 0)
			end
		else
			for char in inserted:gmatch("[%z\1-\127\194-\244][\128-\191]*") do
				if not char:find(control.filterPattern) then
					local held = replacing and 0 or #kept
					if limit and #front + held + #char + #back > limit then
						refused = true
					else
						kept = (replacing and "" or kept) .. char
						replacing = false
					end
				end
			end
			if refused then
				error("the text is too long", 0)
			end
			if replacing or kept == "" then
				return false
			end
		end
	end
	control:SetText(front .. kept .. back, true)
	return true
end

function glue.press_check(control, on)
	control.state = on
	control.changeFunc(on)
end

-- Pastes into an edit control as PoB's EditControl does on Ctrl+V (EditControl.lua:530): the
-- text goes through the control's `pasteFilter`, then is typed as any other text.
function glue.paste_into(control, text, typed)
	-- text the user typed goes in key by key (OnChar, EditControl.lua:723-735): neither step
	if typed then
		return glue.type_into(control, text, false)
	end
	if control.pasteFilter then
		text = control.pasteFilter(text)
	end
	text = text:gsub("[\128-\255]", "?")
	return glue.type_into(control, text, true)
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
