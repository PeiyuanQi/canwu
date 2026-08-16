//! Minimal first-party reference client. It uses only `canwu-api`.

#![allow(clippy::module_name_repetitions)]

use canwu_api::{
    Canwu, Command, CommandEnvelope, DemoIds, EntityRef, Issuer, SimDuration, WorldSnapshot,
};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};
use serde_json::Value;
use std::time::{Duration, Instant};

const ENGLISH_LOGO_PNG: &[u8] = include_bytes!("../../../assets/branding/canwu-logo-en.png");
const CHINESE_LOGO_PNG: &[u8] = include_bytes!("../../../assets/branding/canwu-logo-zh-cn.png");

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Canwu Debug Client",
        options,
        Box::new(|creation_context| Ok(Box::new(DebugApp::new(&creation_context.egui_ctx)))),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LogoLanguage {
    English,
    Chinese,
}

struct DebugApp {
    canwu: Canwu,
    ids: DemoIds,
    english_logo: TextureHandle,
    chinese_logo: TextureHandle,
    logo_language: LogoLanguage,
    running: bool,
    last_tick: Instant,
    selected: Option<EntityRef>,
    search: String,
    saved_snapshot: Option<String>,
    morale_edit: u16,
    map_pan: Vec2,
    map_zoom: f32,
    status: String,
}

impl DebugApp {
    fn new(context: &egui::Context) -> Self {
        let canwu = Canwu::demo(35).expect("built-in scenario must be valid");
        let ids = Canwu::demo_ids();
        Self {
            canwu,
            ids,
            english_logo: load_logo(context, "canwu-logo-en", ENGLISH_LOGO_PNG),
            chinese_logo: load_logo(context, "canwu-logo-zh-cn", CHINESE_LOGO_PNG),
            logo_language: LogoLanguage::English,
            running: false,
            last_tick: Instant::now(),
            selected: Some(EntityRef::Army(ids.army)),
            search: String::new(),
            saved_snapshot: None,
            morale_edit: 72,
            map_pan: Vec2::ZERO,
            map_zoom: 1.0,
            status: "Ready".to_owned(),
        }
    }

    fn reset(&mut self) {
        self.canwu = Canwu::demo(35).expect("built-in scenario must be valid");
        self.running = false;
        self.selected = Some(EntityRef::Army(self.ids.army));
        self.morale_edit = 72;
        "Scenario reset through the lifecycle API".clone_into(&mut self.status);
    }

    fn advance(&mut self, duration: SimDuration) {
        match self.canwu.advance(duration) {
            Ok(events) => {
                self.status = format!("Advanced; {} event(s) emitted", events.len());
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .button(if self.running { "Pause" } else { "Run" })
                .clicked()
            {
                self.running = !self.running;
                self.last_tick = Instant::now();
            }
            if ui.button("Step").clicked() {
                match self.canwu.step() {
                    Ok(events) => self.status = format!("Step emitted {} event(s)", events.len()),
                    Err(error) => self.status = error.to_string(),
                }
            }
            if ui.button("+6 hours").clicked() {
                self.advance(SimDuration::hours(6));
            }
            if ui.button("+1 day").clicked() {
                self.advance(SimDuration::days(1));
            }
            if ui.button("Reset").clicked() {
                self.reset();
            }
            if ui.button("Snapshot").clicked() {
                match self.canwu.snapshot_json() {
                    Ok(snapshot) => {
                        self.saved_snapshot = Some(snapshot);
                        "Snapshot stored in memory".clone_into(&mut self.status);
                    }
                    Err(error) => self.status = error.to_string(),
                }
            }
            let restore_enabled = self.saved_snapshot.is_some();
            if ui
                .add_enabled(restore_enabled, egui::Button::new("Restore"))
                .clicked()
                && let Some(snapshot) = &self.saved_snapshot
            {
                match Canwu::from_snapshot_json(snapshot) {
                    Ok(canwu) => {
                        self.canwu = canwu;
                        self.running = false;
                        "Snapshot restored".clone_into(&mut self.status);
                    }
                    Err(error) => self.status = error.to_string(),
                }
            }
            ui.separator();
            ui.monospace(format!("v{}", Canwu::version()));
            ui.separator();
            ui.monospace(self.canwu.time().to_string());
            ui.separator();
            ui.label(&self.status);
        });
    }

    fn world_browser(&mut self, ui: &mut egui::Ui) {
        self.branding(ui);
        ui.separator();
        ui.heading("World Browser");
        ui.text_edit_singleline(&mut self.search);
        let search = self.search.to_lowercase();
        let world = self.canwu.world();
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::CollapsingHeader::new(format!("Persons ({})", world.people.len()))
                .default_open(true)
                .show(ui, |ui| {
                    for person in &world.people {
                        if matches_search(&search, &person.name) {
                            selectable_entity(
                                ui,
                                &mut self.selected,
                                EntityRef::Person(person.id),
                                &person.name,
                            );
                        }
                    }
                });
            egui::CollapsingHeader::new(format!("Armies ({})", world.armies.len()))
                .default_open(true)
                .show(ui, |ui| {
                    for army in &world.armies {
                        if matches_search(&search, &army.name) {
                            selectable_entity(
                                ui,
                                &mut self.selected,
                                EntityRef::Army(army.id),
                                &army.name,
                            );
                        }
                    }
                });
            egui::CollapsingHeader::new(format!("Territories ({})", world.territories.len()))
                .default_open(true)
                .show(ui, |ui| {
                    for territory in &world.territories {
                        if matches_search(&search, &territory.name) {
                            selectable_entity(
                                ui,
                                &mut self.selected,
                                EntityRef::Territory(territory.id),
                                &territory.name,
                            );
                        }
                    }
                });
            egui::CollapsingHeader::new(format!("Governments ({})", world.governments.len())).show(
                ui,
                |ui| {
                    for government in &world.governments {
                        if matches_search(&search, &government.name) {
                            selectable_entity(
                                ui,
                                &mut self.selected,
                                EntityRef::Government(government.id),
                                &government.name,
                            );
                        }
                    }
                },
            );
            egui::CollapsingHeader::new(format!("Routes ({})", world.routes.len())).show(
                ui,
                |ui| {
                    for route in &world.routes {
                        if matches_search(&search, &route.name) {
                            selectable_entity(
                                ui,
                                &mut self.selected,
                                EntityRef::Route(route.id),
                                &route.name,
                            );
                        }
                    }
                },
            );
        });
    }

    fn branding(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Canwu");
            ui.selectable_value(&mut self.logo_language, LogoLanguage::English, "EN");
            ui.selectable_value(&mut self.logo_language, LogoLanguage::Chinese, "ZH");
        });

        let logo = match self.logo_language {
            LogoLanguage::English => &self.english_logo,
            LogoLanguage::Chinese => &self.chinese_logo,
        };
        let side = ui.available_width().min(180.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
        ui.painter().image(
            logo.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.heading("Inspector");
        let Some(selected) = self.selected.clone() else {
            ui.label("Select an entity");
            return;
        };
        ui.monospace(selected.to_string());
        let world = self.canwu.world();
        let Some(value) = entity_value(&world, &selected) else {
            ui.label("Entity is not present in this snapshot");
            return;
        };
        let type_name = entity_type_name(&selected);
        if let Some(schema) = self.canwu.schema().get(type_name) {
            ui.label(&schema.description);
            ui.separator();
        }
        egui::ScrollArea::vertical()
            .max_height(480.0)
            .show(ui, |ui| {
                if let Some(object) = value.as_object() {
                    for (field, value) in object {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(field);
                            ui.monospace(compact_value(value));
                        });
                    }
                }
            });

        if let EntityRef::Army(army) = selected {
            ui.separator();
            ui.label("Debug command");
            ui.add(egui::Slider::new(&mut self.morale_edit, 0..=100).text("morale"));
            if ui.button("Apply morale through command").clicked() {
                match self.canwu.submit(CommandEnvelope::new(
                    Issuer::Debug,
                    Command::DebugSetArmyMorale {
                        army,
                        morale: self.morale_edit,
                    },
                )) {
                    Ok(receipt) => {
                        self.status = format!("Accepted debug command {}", receipt.command_id);
                    }
                    Err(error) => self.status = error.to_string(),
                }
            }
        }
    }

    fn event_log(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Event / Causality Log");
            ui.label(format!("{} event(s)", self.canwu.events().len()));
        });
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for event in self.canwu.events().iter().rev().take(100).rev() {
                    ui.horizontal_wrapped(|ui| {
                        ui.monospace(format!("{} · {}", event.timestamp, event.kind.event_type()));
                        ui.label(&event.summary);
                        if let Some(cause) = &event.cause {
                            ui.weak(format!("cause: {cause:?}"));
                        }
                    });
                }
            });
    }

    fn map_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Minimal World View");
            ui.weak("drag to pan · scroll to zoom · click a territory");
        });
        let available = ui.available_size();
        let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
        if response.dragged() {
            self.map_pan += response.drag_delta();
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.map_zoom = (self.map_zoom * (1.0 + scroll * 0.001)).clamp(0.35, 3.0);
            }
        }
        let rect = response.rect;
        painter.rect_filled(rect, 4.0, Color32::from_rgb(24, 29, 32));
        let world = self.canwu.world();

        for route in &world.routes {
            let Some(from) = world.territory(route.from) else {
                continue;
            };
            let Some(to) = world.territory(route.to) else {
                continue;
            };
            painter.line_segment(
                [
                    self.map_position(rect, from.position),
                    self.map_position(rect, to.position),
                ],
                Stroke::new(3.0_f32, Color32::from_rgb(108, 91, 64)),
            );
        }
        for territory in &world.territories {
            let position = self.map_position(rect, territory.position);
            let selected = self.selected == Some(EntityRef::Territory(territory.id));
            painter.circle_filled(
                position,
                if selected { 10.0 } else { 7.0 },
                if selected {
                    Color32::LIGHT_GREEN
                } else {
                    Color32::from_rgb(210, 188, 129)
                },
            );
            painter.text(
                position + Vec2::new(0.0, 14.0),
                Align2::CENTER_TOP,
                &territory.name,
                FontId::proportional(13.0),
                Color32::WHITE,
            );
        }
        for army in &world.armies {
            if let Some(territory) = world.territory(army.location) {
                let position = self.map_position(rect, territory.position) + Vec2::new(0.0, -18.0);
                painter.circle_filled(position, 6.0, Color32::from_rgb(193, 71, 71));
                painter.text(
                    position + Vec2::new(9.0, 0.0),
                    Align2::LEFT_CENTER,
                    &army.name,
                    FontId::proportional(12.0),
                    Color32::LIGHT_RED,
                );
            }
        }
        if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
            && let Some(territory) = world.territories.iter().min_by(|left, right| {
                self.map_position(rect, left.position)
                    .distance(pointer)
                    .total_cmp(&self.map_position(rect, right.position).distance(pointer))
            })
            && self
                .map_position(rect, territory.position)
                .distance(pointer)
                < 24.0
        {
            self.selected = Some(EntityRef::Territory(territory.id));
        }
    }

    fn map_position(&self, rect: Rect, point: canwu_api::MapPoint) -> Pos2 {
        let origin = rect.left_top() + Vec2::new(30.0, 25.0) + self.map_pan;
        origin + Vec2::new(point.x, point.y) * self.map_zoom
    }
}

fn load_logo(context: &egui::Context, name: &str, bytes: &[u8]) -> TextureHandle {
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .expect("embedded Canwu logo must be a valid PNG");
    let size = [
        usize::try_from(decoded.width()).expect("logo width must fit in usize"),
        usize::try_from(decoded.height()).expect("logo height must fit in usize"),
    ];
    let pixels = decoded.to_rgba8();
    context.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_raw()),
        egui::TextureOptions::LINEAR,
    )
}

impl eframe::App for DebugApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        if self.running && self.last_tick.elapsed() >= Duration::from_millis(250) {
            self.advance(SimDuration::hours(6));
            self.last_tick = Instant::now();
        }
        if self.running {
            context.request_repaint_after(Duration::from_millis(50));
        }

        egui::TopBottomPanel::top("controls").show(context, |ui| self.controls(ui));
        egui::TopBottomPanel::bottom("events")
            .resizable(true)
            .default_height(190.0)
            .show(context, |ui| self.event_log(ui));
        egui::SidePanel::left("browser")
            .resizable(true)
            .default_width(220.0)
            .show(context, |ui| self.world_browser(ui));
        egui::SidePanel::right("inspector")
            .resizable(true)
            .default_width(300.0)
            .show(context, |ui| self.inspector(ui));
        egui::CentralPanel::default().show(context, |ui| self.map_view(ui));
    }
}

fn matches_search(search: &str, name: &str) -> bool {
    search.is_empty() || name.to_lowercase().contains(search)
}

fn selectable_entity(
    ui: &mut egui::Ui,
    selected: &mut Option<EntityRef>,
    entity: EntityRef,
    label: &str,
) {
    let is_selected = selected.as_ref() == Some(&entity);
    if ui.selectable_label(is_selected, label).clicked() {
        *selected = Some(entity);
    }
}

fn entity_type_name(entity: &EntityRef) -> &'static str {
    match entity {
        EntityRef::Army(_) => "army",
        EntityRef::Domain(_) => "domain",
        EntityRef::Government(_) => "government",
        EntityRef::Organization(_) => "organization",
        EntityRef::Person(_) => "person",
        EntityRef::Resource(_) => "resource",
        EntityRef::Route(_) => "route",
        EntityRef::Territory(_) => "territory",
    }
}

fn entity_value(world: &WorldSnapshot, entity: &EntityRef) -> Option<Value> {
    match entity {
        EntityRef::Army(id) => world.army(*id).and_then(to_value),
        EntityRef::Government(id) => world.government(*id).and_then(to_value),
        EntityRef::Person(id) => world.person(*id).and_then(to_value),
        EntityRef::Route(id) => world.route(*id).and_then(to_value),
        EntityRef::Territory(id) => world.territory(*id).and_then(to_value),
        EntityRef::Domain(_) | EntityRef::Organization(_) | EntityRef::Resource(_) => None,
    }
}

fn to_value<T: serde::Serialize>(value: &T) -> Option<Value> {
    serde_json::to_value(value).ok()
}

fn compact_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}
