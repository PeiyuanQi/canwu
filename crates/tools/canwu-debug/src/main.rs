//! Minimal first-party client for the replaceable reference-world integration.

#![allow(clippy::module_name_repetitions)]

use canwu_api::{
    BoundaryReceipt, Canwu, EntityRef, KnowledgeHolderRef, KnowledgeQuery, SimDuration,
};
use canwu_ming_fiscal_reference::{
    DEFAULT_SEED, MingFiscalTraceFrame, MingFiscalTracePhase, MingFiscalTraceWriter,
    TraceViewerHandle, capture_ming_fiscal_trace_frame, default_trace_directory,
    ming_fiscal_reference_scenario, new_ming_fiscal_reference, restore_ming_fiscal_reference,
    start_trace_viewer, trace_error,
};
use canwu_reference_world::{
    MapPoint, ReferenceWorldIds, WorldSnapshot, snapshot as reference_snapshot,
};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const ENGLISH_LOGO_PNG: &[u8] = include_bytes!("../../../../assets/branding/canwu-logo-en.png");
const CHINESE_LOGO_PNG: &[u8] = include_bytes!("../../../../assets/branding/canwu-logo-zh-cn.png");
const DEBUG_FIXTURE: &str = "hongwu-1391";
const MAX_DEBUG_TRACE_FRAMES: usize = 512;

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
    ids: ReferenceWorldIds,
    english_logo: TextureHandle,
    chinese_logo: TextureHandle,
    logo_language: LogoLanguage,
    running: bool,
    last_tick: Instant,
    selected: Option<EntityRef>,
    search: String,
    saved_snapshot: Option<String>,
    trace_writer: Option<MingFiscalTraceWriter>,
    trace_viewer: Option<TraceViewerHandle>,
    trace_frames: Vec<MingFiscalTraceFrame>,
    trace_sequence: usize,
    selected_trace_frame: Option<usize>,
    map_pan: Vec2,
    map_zoom: f32,
    status: String,
}

impl DebugApp {
    fn new(context: &egui::Context) -> Self {
        let (canwu, ids) = new_reference_run();
        let trace_writer =
            MingFiscalTraceWriter::create(DEBUG_FIXTURE, DEFAULT_SEED, canwu.time()).ok();
        let status = trace_writer.as_ref().map_or_else(
            || "Ready (trace dump unavailable)".to_owned(),
            |writer| format!("Ready · trace={}", writer.paths().steps.display()),
        );
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
            trace_writer,
            trace_viewer: None,
            trace_frames: Vec::new(),
            trace_sequence: 0,
            selected_trace_frame: None,
            map_pan: Vec2::ZERO,
            map_zoom: 1.0,
            status,
        }
    }

    fn reset(&mut self) {
        if let Some(mut writer) = self.trace_writer.take() {
            let _ = writer.finish(&self.canwu);
        }
        (self.canwu, self.ids) = new_reference_run();
        self.running = false;
        self.selected = Some(EntityRef::Army(self.ids.army));
        self.trace_frames.clear();
        self.trace_sequence = 0;
        self.selected_trace_frame = None;
        self.trace_writer = self.new_trace_writer();
        "Scenario reset through the lifecycle API; trace restarted".clone_into(&mut self.status);
    }

    fn new_trace_writer(&self) -> Option<MingFiscalTraceWriter> {
        MingFiscalTraceWriter::create(DEBUG_FIXTURE, DEFAULT_SEED, self.canwu.time()).ok()
    }

    fn advance(&mut self, duration: SimDuration) {
        match self.canwu.advance_canonical(duration) {
            Ok(receipts) => {
                let count = receipts.len();
                for receipt in receipts {
                    self.record_trace(MingFiscalTracePhase::CanonicalBoundary, receipt);
                }
                self.status = format!("Advanced canonically; {count} boundary(ies) settled");
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn step_canonical(&mut self) {
        match self.canwu.step_canonical() {
            Ok(Some(receipt)) => {
                let boundary_id = format!("{:?}", receipt.boundary_id);
                self.record_trace(MingFiscalTracePhase::CanonicalBoundary, receipt);
                self.status = format!("Canonical step settled: {boundary_id}");
            }
            Ok(None) => {
                "No canonical boundary is currently scheduled".clone_into(&mut self.status);
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn record_trace(&mut self, phase: MingFiscalTracePhase, receipt: BoundaryReceipt) {
        let sequence = self.trace_sequence;
        match capture_ming_fiscal_trace_frame(&self.canwu, sequence, phase, receipt) {
            Ok(frame) => {
                self.trace_sequence = self.trace_sequence.saturating_add(1);
                let write_error = self
                    .trace_writer
                    .as_mut()
                    .and_then(|writer| writer.write_frame(&frame).err());
                if let Some(error) = write_error {
                    self.trace_writer = None;
                    self.status = format!("Trace dump stopped: {error}");
                }
                self.trace_frames.push(frame);
                self.retain_recent_trace_frames();
                self.selected_trace_frame = Some(self.trace_frames.len().saturating_sub(1));
            }
            Err(error) => self.status = format!("Trace capture failed: {error}"),
        }
    }

    fn run_fiscal_sample(&mut self) {
        let mut writer = self.trace_writer.take();
        let mut sequence = self.trace_sequence;
        let mut frames = Vec::new();
        let id_prefix = format!("debug.{DEBUG_FIXTURE}.{}", self.canwu.revision());
        let result = canwu_ming_fiscal_reference::run_ming_fiscal_sample_cycle_with_trace(
            &mut self.canwu,
            &id_prefix,
            |canwu, phase, receipt| {
                let frame =
                    capture_ming_fiscal_trace_frame(canwu, sequence, phase, receipt.clone())?;
                sequence = sequence.saturating_add(1);
                if let Some(writer) = writer.as_mut() {
                    writer
                        .write_frame(&frame)
                        .map_err(|error| trace_error(&error))?;
                }
                frames.push(frame);
                Ok(())
            },
        );
        self.trace_writer = writer;
        self.trace_sequence = sequence;
        self.trace_frames.extend(frames);
        self.retain_recent_trace_frames();
        if !self.trace_frames.is_empty() {
            self.selected_trace_frame = Some(self.trace_frames.len().saturating_sub(1));
        }
        match result {
            Ok(()) => "Ming fiscal sample cycle completed".clone_into(&mut self.status),
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
                self.step_canonical();
            }
            if ui.button("+6 hours").clicked() {
                self.advance(SimDuration::hours(6));
            }
            if ui.button("+1 day").clicked() {
                self.advance(SimDuration::days(1));
            }
            if ui.button("Run fiscal sample").clicked() {
                self.run_fiscal_sample();
            }
            if ui.button("Open trace viewer").clicked() {
                self.open_trace_viewer();
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
                match restore_ming_fiscal_reference(snapshot) {
                    Ok(canwu) => {
                        if let Some(mut writer) = self.trace_writer.take() {
                            let _ = writer.finish(&self.canwu);
                        }
                        self.canwu = canwu;
                        self.running = false;
                        self.trace_frames.clear();
                        self.trace_sequence = 0;
                        self.selected_trace_frame = None;
                        self.trace_writer = self.new_trace_writer();
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

    fn retain_recent_trace_frames(&mut self) {
        let excess = self
            .trace_frames
            .len()
            .saturating_sub(MAX_DEBUG_TRACE_FRAMES);
        if excess > 0 {
            self.trace_frames.drain(..excess);
        }
    }

    fn world_browser(&mut self, ui: &mut egui::Ui) {
        self.branding(ui);
        ui.separator();
        ui.heading("World Browser");
        ui.text_edit_singleline(&mut self.search);
        let search = self.search.to_lowercase();
        let world = reference_snapshot(&self.canwu).expect("reference world must remain valid");
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
        let world = reference_snapshot(&self.canwu).expect("reference world must remain valid");
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

        if let EntityRef::Person(person) = selected {
            ui.separator();
            ui.label("Trusted debug knowledge projection");
            match self.canwu.admin_query_knowledge(
                KnowledgeHolderRef::Person(person),
                &KnowledgeQuery::default(),
            ) {
                Ok(result) => {
                    ui.monospace(format!("{} current record(s)", result.records.len()));
                    for record in result.records {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(record.schema.kind.to_string());
                            ui.monospace(compact_value(&record.payload));
                        });
                    }
                }
                Err(error) => {
                    ui.colored_label(Color32::LIGHT_RED, error.to_string());
                }
            }
        }
    }

    fn event_log(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Fiscal Trace");
            ui.label(format!(
                "{} recent boundary frame(s) · full trace on disk",
                self.trace_frames.len()
            ));
        });
        if let Some(writer) = &self.trace_writer {
            ui.weak(format!("dump: {}", writer.paths().steps.display()));
        }
        egui::ScrollArea::vertical()
            .max_height(170.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for frame in self.trace_frames.iter().rev().take(100).rev() {
                    let counts = &frame.fiscal.counts;
                    ui.horizontal_wrapped(|ui| {
                        ui.monospace(format!(
                            "#{} · {} · {}",
                            frame.sequence, frame.receipt.settled_at, frame.phase
                        ));
                        ui.label(format!(
                            "assessments={} requests={} receipts={} audits={}",
                            counts.assessments,
                            counts.execution_requests,
                            counts.execution_receipts,
                            counts.audits
                        ));
                    });
                }
            });
        if let Some(frame) = self.trace_frames.last() {
            ui.separator();
            ui.heading("Latest Boundary Detail");
            ui.horizontal_wrapped(|ui| {
                ui.monospace(format!("phase={}", frame.phase));
                ui.monospace(format!("boundary={:?}", frame.receipt.boundary_id));
                ui.monospace(format!("revision={}", frame.revision));
                ui.monospace(format!("checkpoint={}", frame.checkpoint_hash));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "changes={} records={} knowledge={} allocations={}",
                    frame.receipt.change_count,
                    frame.receipt.record_change_count,
                    frame.receipt.knowledge_record_count,
                    frame.receipt.allocations.len()
                ));
                ui.label(format!(
                    "fiscal assessments={} requests={} receipts={} aggregates={}",
                    frame.fiscal.counts.assessments,
                    frame.fiscal.counts.execution_requests,
                    frame.fiscal.counts.execution_receipts,
                    frame.fiscal.counts.aggregates
                ));
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.heading("Event / Causality Log");
            ui.label(format!("{} event(s)", self.canwu.events().len()));
        });
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for event in self.canwu.events().iter().rev().take(100).rev() {
                    ui.horizontal_wrapped(|ui| {
                        ui.monospace(format!(
                            "{} · {}",
                            event.timestamp,
                            event.kind.qualified_event_type()
                        ));
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
        let world = reference_snapshot(&self.canwu).expect("reference world must remain valid");

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

    // Frame indexes are converted to screen coordinates; sub-pixel precision
    // beyond f32 is not meaningful for an egui viewport.
    #[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
    fn trace_timeline(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Settlement Timeline");
            ui.weak(format!("{} frame(s)", self.trace_frames.len()));
            if let Some(index) = self.selected_trace_frame
                && let Some(frame) = self.trace_frames.get(index)
            {
                ui.label(format!(
                    "selected #{} · {} · {}",
                    frame.sequence, frame.receipt.settled_at, frame.phase
                ));
            }
        });
        if self.trace_frames.is_empty() {
            ui.weak("Run or step the simulation to populate the timeline.");
            return;
        }

        let mut selected_index = self
            .selected_trace_frame
            .unwrap_or_else(|| self.trace_frames.len().saturating_sub(1));
        ui.horizontal(|ui| {
            if ui
                .add_enabled(selected_index > 0, egui::Button::new("Previous frame"))
                .clicked()
            {
                selected_index = selected_index.saturating_sub(1);
            }
            ui.add(
                egui::Slider::new(&mut selected_index, 0..=self.trace_frames.len() - 1)
                    .show_value(false)
                    .text("precise frame selection"),
            );
            if ui
                .add_enabled(
                    selected_index + 1 < self.trace_frames.len(),
                    egui::Button::new("Next frame"),
                )
                .clicked()
            {
                selected_index = (selected_index + 1).min(self.trace_frames.len() - 1);
            }
            let first_sequence = self.trace_frames.first().map_or(0, |frame| frame.sequence);
            let last_sequence = self.trace_frames.last().map_or(0, |frame| frame.sequence);
            ui.weak(format!(
                "on-screen sequence range #{first_sequence}–#{last_sequence}"
            ));
        });
        self.selected_trace_frame = Some(selected_index);

        let height = ui.available_height().clamp(120.0, 190.0);
        let (response, painter) =
            ui.allocate_painter(Vec2::new(ui.available_width(), height), Sense::click());
        let rect = response.rect;
        let left = rect.left() + 28.0;
        let right = rect.right() - 28.0;
        let y = rect.top() + 54.0;
        let span = (right - left).max(1.0);
        let last = self.trace_frames.len().saturating_sub(1) as f32;
        painter.line_segment(
            [Pos2::new(left, y), Pos2::new(right, y)],
            Stroke::new(2.0_f32, Color32::from_rgb(90, 98, 104)),
        );

        for (index, frame) in self.trace_frames.iter().enumerate() {
            let x = timeline_x(left, span, last, index);
            let selected = self.selected_trace_frame == Some(index);
            let color = phase_color(frame.phase);
            painter.line_segment(
                [Pos2::new(x, y - 22.0), Pos2::new(x, y + 22.0)],
                Stroke::new(if selected { 2.0_f32 } else { 1.0_f32 }, color),
            );
            painter.circle_filled(Pos2::new(x, y), if selected { 8.0 } else { 5.0 }, color);
            if selected {
                painter.text(
                    Pos2::new(x, y - 36.0),
                    Align2::CENTER_BOTTOM,
                    format!("#{} {}", frame.sequence, frame.phase),
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
            }
            if index == 0 || index == self.trace_frames.len() - 1 || selected {
                painter.text(
                    Pos2::new(x, y + 30.0),
                    Align2::CENTER_TOP,
                    frame.receipt.settled_at.to_string(),
                    FontId::proportional(11.0),
                    Color32::LIGHT_GRAY,
                );
            }
        }

        if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
            && pointer.y >= y - 32.0
            && pointer.y <= y + 32.0
        {
            self.selected_trace_frame = self
                .trace_frames
                .iter()
                .enumerate()
                .min_by(|(left_index, _), (right_index, _)| {
                    let left_distance =
                        (timeline_x(left, span, last, *left_index) - pointer.x).abs();
                    let right_distance =
                        (timeline_x(left, span, last, *right_index) - pointer.x).abs();
                    left_distance.total_cmp(&right_distance)
                })
                .map(|(index, _)| index);
        }

        if let Some(index) = self.selected_trace_frame
            && let Some(frame) = self.trace_frames.get(index)
        {
            let counts = &frame.fiscal.counts;
            ui.horizontal_wrapped(|ui| {
                ui.monospace(format!(
                    "#{} · {} · year {} · revision {}",
                    frame.sequence, frame.phase, frame.fiscal.historical_year, frame.revision
                ));
                ui.label(format!(
                    "assessments={} requests={} receipts={} audits={} aggregates={}",
                    counts.assessments,
                    counts.execution_requests,
                    counts.execution_receipts,
                    counts.audits,
                    counts.aggregates
                ));
            });
        }
    }

    fn open_trace_viewer(&mut self) {
        if let Some(viewer) = &self.trace_viewer {
            match viewer.open_browser() {
                Ok(()) => self.status = format!("Trace viewer reopened · {}", viewer.url()),
                Err(error) => self.status = error.to_string(),
            }
            return;
        }

        let trace_directory = self.trace_writer.as_ref().map_or_else(
            || {
                default_trace_directory()
                    .join("ming-fiscal-reference")
                    .join(DEBUG_FIXTURE)
            },
            |writer| writer.paths().directory.clone(),
        );
        let Some(workspace_root) = find_workspace_root() else {
            "Could not locate the Canwu workspace root".clone_into(&mut self.status);
            return;
        };
        match start_trace_viewer(workspace_root, trace_directory, 0) {
            Ok(viewer) => {
                let url = viewer.url().to_owned();
                match viewer.open_browser() {
                    Ok(()) => self.status = format!("Trace viewer opened · {url}"),
                    Err(error) => self.status = error.to_string(),
                }
                self.trace_viewer = Some(viewer);
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn map_position(&self, rect: Rect, point: MapPoint) -> Pos2 {
        let origin = rect.left_top() + Vec2::new(30.0, 25.0) + self.map_pan;
        origin + Vec2::new(point.x, point.y) * self.map_zoom
    }
}

fn new_reference_run() -> (Canwu, ReferenceWorldIds) {
    let reference = ming_fiscal_reference_scenario(DEBUG_FIXTURE)
        .expect("Ming fiscal reference scenario must be valid");
    let ids = reference.world_ids;
    let canwu = new_ming_fiscal_reference(DEFAULT_SEED, DEBUG_FIXTURE)
        .expect("Ming fiscal reference scenario must initialize");
    (canwu, ids)
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
        egui::CentralPanel::default().show(context, |ui| {
            let timeline_height = ui.available_height().clamp(150.0, 210.0);
            let map_height = (ui.available_height() - timeline_height - 12.0).max(230.0);
            ui.allocate_ui(Vec2::new(ui.available_width(), map_height), |ui| {
                self.map_view(ui);
            });
            ui.separator();
            ui.allocate_ui(Vec2::new(ui.available_width(), timeline_height), |ui| {
                self.trace_timeline(ui);
            });
        });
    }
}

impl Drop for DebugApp {
    fn drop(&mut self) {
        if let Some(mut writer) = self.trace_writer.take() {
            let _ = writer.finish(&self.canwu);
        }
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

fn find_workspace_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("CANWU_WORKSPACE_ROOT") {
        let root = PathBuf::from(root);
        if is_viewer_workspace(&root) {
            return Some(root);
        }
    }
    if let Ok(current) = std::env::current_dir()
        && let Some(root) = find_workspace_from(&current)
    {
        return Some(root);
    }
    let compiled_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    find_workspace_from(&compiled_manifest)
}

fn find_workspace_from(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if is_viewer_workspace(&current) {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn is_viewer_workspace(path: &Path) -> bool {
    path.join("Cargo.toml").is_file()
        && path.join("crates").is_dir()
        && path.join("tools").join("trace-viewer").is_dir()
}

#[allow(clippy::cast_precision_loss)]
fn timeline_x(left: f32, span: f32, last: f32, index: usize) -> f32 {
    if last == 0.0 {
        left + span * 0.5
    } else {
        left + span * (index as f32 / last)
    }
}

fn phase_color(phase: MingFiscalTracePhase) -> Color32 {
    match phase {
        MingFiscalTracePhase::InitialState => Color32::from_rgb(150, 150, 150),
        MingFiscalTracePhase::OpenAssessment => Color32::from_rgb(88, 166, 255),
        MingFiscalTracePhase::AuthorizeExecution => Color32::from_rgb(177, 125, 255),
        MingFiscalTracePhase::AdapterEvidence => Color32::from_rgb(255, 184, 77),
        MingFiscalTracePhase::FiscalExecutionReceipt => Color32::from_rgb(84, 205, 126),
        MingFiscalTracePhase::ReportMaterialization => Color32::from_rgb(108, 199, 214),
        MingFiscalTracePhase::CanonicalBoundary => Color32::from_rgb(232, 116, 116),
    }
}
