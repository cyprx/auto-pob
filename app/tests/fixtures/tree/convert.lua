base = "dot"
-- The subtype of a tree version, as TreeTab:ConvertToVersion reads it (TreeTab.lua:593)
local function subtype(version)
	return version:match("(_%l+_?%l*)")
end
function recipe()
	-- Setup, not recorded: put the active spec on the newest older version with the same
	-- subtype, then save and reload so the build starts from that file.
	local current = build.spec.treeVersion
	local older
	for i = #treeVersionList, 1, -1 do
		local version = treeVersionList[i]
		if version ~= latestTreeVersion and subtype(version) == subtype(current) then
			older = version
			break
		end
	end
	assert(older, "no older version")
	build.treeTab:OpenVersionConvertPopup(older, true)
	main.popups[1].controls.convert.onClick()
	setup_xml = build:SaveDB("convert")
	loadBuildFromXML(setup_xml, "convert")
	assert(build.spec.treeVersion == older)
	act.convert(latestTreeVersion, true)
	checkpoint()
end
