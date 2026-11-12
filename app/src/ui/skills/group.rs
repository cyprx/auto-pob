//! The selected group's fields and its gem table.

use super::{SkillsOutput, SkillsUi};
use crate::engine::views::{GemView, SkillsView, SocketGroupView};
use crate::engine::{Command, GemTarget, GemToggle, GemTooltipQuery};
use crate::widgets::{pob_colour, pob_text, tooltip_ui};
use eframe::egui::{self, Color32, Stroke};

const ERROR: Color32 = Color32::from_rgb(255, 80, 80);

impl SkillsUi {
    pub(super) fn group_pane(
        &mut self,
        ui: &mut egui::Ui,
        view: &SkillsView,
        busy: bool,
        out: &mut SkillsOutput,
    ) {
        let gi = self.selected;
        let Some(g) = view.groups.get(gi) else {
            ui.label("No socket group. Add one with + New.");
            return;
        };
        self.group_fields(ui, view, gi, g, busy, out);
        // an item's group has no gem table: PoB hides the gem boxes' delete and the slot
        if g.source.is_some() {
            return;
        }
        ui.separator();
        self.gem_table(ui, gi, g, busy, out);
    }

    fn group_fields(
        &mut self,
        ui: &mut egui::Ui,
        view: &SkillsView,
        gi: usize,
        g: &SocketGroupView,
        busy: bool,
        out: &mut SkillsOutput,
    ) {
        let default = ui.visuals().text_color();
        if let Some(note) = &g.source_note {
            ui.label(pob_text(note, 14.0, default));
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Label:");
            self.field(
                ui,
                &format!("g{gi}_label"),
                &g.label,
                200.0,
                busy,
                |text| Command::SetGroupLabel { group: gi, text },
                &mut out.commands,
            );
            if g.source.is_none() {
                ui.label("Slot:");
                let current = view
                    .slot_choices
                    .iter()
                    .find(|c| c.slot == g.slot)
                    .map(|c| c.label.clone())
                    .or_else(|| g.slot.clone())
                    .unwrap_or_else(|| "None".into());
                let combo = egui::ComboBox::from_id_salt(("skills_slot", gi))
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (i, choice) in view.slot_choices.iter().enumerate() {
                            let item = ui.selectable_label(choice.slot == g.slot, &choice.label);
                            self.rects.note(&item, || format!("slot_item_{i}"));
                            if item.clicked() && choice.slot != g.slot {
                                out.commands.push(Command::SetGroupSlot {
                                    group: gi,
                                    slot: choice.slot.clone(),
                                });
                            }
                        }
                    });
                self.rects.note(&combo.response, || "slot".into());
                if !g.slot_enabled {
                    ui.colored_label(ERROR, "slot disabled");
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            let mut on = g.enabled;
            let enabled = ui.checkbox(&mut on, "Enabled");
            self.rects.note(&enabled, || "group_enabled".into());
            if enabled.changed() {
                out.commands
                    .push(Command::SetGroupEnabled { group: gi, on });
            }
            let mut on = g.full_dps;
            let full = ui.checkbox(&mut on, "Include in Full DPS");
            self.rects.note(&full, || "group_full_dps".into());
            if full.changed() {
                out.commands
                    .push(Command::SetGroupFullDps { group: gi, on });
            }
            if let Some(count) = &g.count {
                ui.label("Count:");
                self.field(
                    ui,
                    &format!("g{gi}_count"),
                    count,
                    40.0,
                    busy,
                    |text| Command::SetGroupCount { group: gi, text },
                    &mut out.commands,
                );
            }
        });
        if g.source.is_some() {
            return;
        }
        if let Some(imbued) = &g.imbued {
            ui.horizontal_wrapped(|ui| {
                ui.label("Imbued support:");
                ui.add_enabled_ui(imbued.enabled, |ui| {
                    let name = imbued.name.as_deref().unwrap_or("\u{2014}");
                    let pick = ui.button(name);
                    self.rects.note(&pick, || "imbued".into());
                    if pick.clicked() {
                        self.open_picker(gi, GemTarget::Imbued);
                    }
                    if imbued.name.is_some() {
                        let clear = ui.button("x");
                        self.rects.note(&clear, || "imbued_clear".into());
                        if clear.clicked() {
                            out.commands.push(Command::SetImbuedSupport {
                                group: gi,
                                gem_id: None,
                            });
                        }
                    }
                });
            });
        }
        if g.sockets_line.is_some() || g.can_optimise {
            ui.horizontal_wrapped(|ui| {
                if let Some(line) = &g.sockets_line {
                    ui.label(pob_text(line, 14.0, default));
                }
                if g.can_optimise {
                    let opt = ui.button("Optimise");
                    self.rects.note(&opt, || "optimise".into());
                    if opt.clicked() {
                        out.commands.push(Command::OptimiseSockets { group: gi });
                    }
                }
            });
        }
    }

    fn gem_table(
        &mut self,
        ui: &mut egui::Ui,
        gi: usize,
        g: &SocketGroupView,
        busy: bool,
        out: &mut SkillsOutput,
    ) {
        let has_count = g.gems.iter().any(|gem| gem.count.is_some());
        egui::Grid::new(("skills_gems", gi))
            .striped(true)
            .min_col_width(24.0)
            .show(ui, |ui| {
                ui.label("");
                ui.label("Gem");
                ui.label("Lvl");
                ui.label("Qual");
                if has_count {
                    ui.label("Cnt");
                }
                ui.end_row();
                for (si, gem) in g.gems.iter().enumerate() {
                    self.gem_row(ui, gi, si, gem, has_count, busy, out);
                }
                // the empty row adds a gem
                ui.label("");
                let add = ui.button("Add gem\u{2026}");
                self.rects
                    .note(&add, || format!("gem_{}_name", g.gems.len()));
                if add.clicked() || add.secondary_clicked() {
                    self.open_picker(gi, GemTarget::Slot(g.gems.len()));
                }
                ui.end_row();
            });
    }

    /// A gem name hovered long enough asks for the gem's own tooltip (R15).
    fn gem_tooltip_hover(
        &mut self,
        name: &egui::Response,
        gi: usize,
        si: usize,
        gem: &GemView,
        out: &mut SkillsOutput,
    ) {
        if gem.gem_id.is_none() || self.picker.is_some() || !name.hovered() {
            if !name.hovered() && self.gem_hover.is_some_and(|(k, _)| k == (gi, si)) {
                self.gem_hover = None;
            }
            if !name.hovered() && self.gem_tooltip_asked == Some((gi, si)) {
                self.gem_tooltip_asked = None;
                self.gem_tooltip = None;
            }
            return;
        }
        let since = match self.gem_hover {
            Some((k, since)) if k == (gi, si) => since,
            _ => {
                self.gem_hover = Some(((gi, si), self.now));
                self.now
            }
        };
        if self.gem_tooltip_asked != Some((gi, si)) {
            if self.now - since >= super::HOVER_DELAY - 1e-6 {
                self.gem_tooltip_asked = Some((gi, si));
                out.queries.push(Command::GemTooltip(GemTooltipQuery {
                    group: gi,
                    target: GemTarget::Slot(si),
                    gem_id: None,
                }));
            } else {
                name.ctx
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        if self.gem_tooltip_asked == Some((gi, si))
            && let Some(t) = &self.gem_tooltip
            && t.query.group == gi
            && t.query.target == GemTarget::Slot(si)
            && !t.lines.is_empty()
        {
            name.clone().on_hover_ui(|ui| tooltip_ui(ui, &t.lines));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn gem_row(
        &mut self,
        ui: &mut egui::Ui,
        gi: usize,
        si: usize,
        gem: &GemView,
        has_count: bool,
        busy: bool,
        out: &mut SkillsOutput,
    ) {
        let default = ui.visuals().text_color();
        let mut on = gem.enabled;
        let check = ui.checkbox(&mut on, "");
        self.rects.note(&check, || format!("gem_{si}_enabled"));
        if check.changed() {
            out.commands.push(Command::SetGemEnabled {
                group: gi,
                slot: si,
                on,
            });
        }
        // unknown gems are red; a gem that fits its socket gets a border in its colour
        let unknown = gem.error.is_some() || gem.gem_id.is_none();
        let colour = if unknown {
            ERROR
        } else {
            pob_colour(&gem.colour, default)
        };
        let colour = if gem.enabled {
            colour
        } else {
            colour.gamma_multiply(0.5)
        };
        let mut button = egui::Button::new(egui::RichText::new(&gem.name_spec).color(colour));
        if gem.matches_socket {
            button = button.stroke(Stroke::new(1.5, colour));
        }
        let name = ui.add(button);
        self.rects.note(&name, || format!("gem_{si}_name"));
        // PoB's gem box drops its list on a right click too (GemSelectControl.lua:800-804)
        if name.clicked() || name.secondary_clicked() {
            self.open_picker(gi, GemTarget::Slot(si));
        }
        self.gem_tooltip_hover(&name, gi, si, gem, out);
        if let Some(error) = &gem.error {
            name.on_hover_text(error);
        }
        self.field(
            ui,
            &format!("g{gi}_s{si}_level"),
            &gem.level.to_string(),
            36.0,
            busy,
            |text| Command::SetGemLevel {
                group: gi,
                slot: si,
                text,
            },
            &mut out.commands,
        );
        self.field(
            ui,
            &format!("g{gi}_s{si}_quality"),
            &gem.quality.to_string(),
            36.0,
            busy,
            |text| Command::SetGemQuality {
                group: gi,
                slot: si,
                text,
            },
            &mut out.commands,
        );
        if has_count {
            match gem.count {
                Some(count) => {
                    self.field(
                        ui,
                        &format!("g{gi}_s{si}_count"),
                        &count.to_string(),
                        30.0,
                        busy,
                        |text| Command::SetGemCount {
                            group: gi,
                            slot: si,
                            text,
                        },
                        &mut out.commands,
                    );
                }
                None => {
                    ui.label("");
                }
            }
        }
        let del = ui.button("x");
        self.rects.note(&del, || format!("gem_{si}_delete"));
        if del.clicked() {
            out.commands.push(Command::DeleteGem {
                group: gi,
                slot: si,
            });
        }
        ui.end_row();
        // a Vaal gem's two toggles go on a second line
        if gem.toggle1.is_some() || gem.toggle2.is_some() {
            ui.label("");
            ui.horizontal(|ui| {
                for (n, (toggle, which)) in [
                    (&gem.toggle1, GemToggle::First),
                    (&gem.toggle2, GemToggle::Second),
                ]
                .into_iter()
                .enumerate()
                {
                    let Some(t) = toggle else { continue };
                    let mut on = t.on;
                    let check = ui.checkbox(&mut on, pob_text(&t.label, 14.0, default));
                    self.rects
                        .note(&check, || format!("gem_{si}_toggle{}", n + 1));
                    if check.changed() {
                        out.commands.push(Command::SetGemToggle {
                            group: gi,
                            slot: si,
                            which,
                            on,
                        });
                    }
                }
            });
            ui.end_row();
        }
    }
}
